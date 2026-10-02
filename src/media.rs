use crate::{
    Error, Result,
    store::{Outcome, Store},
};
use agentcut_core::{FingerprintStrategy, OperationBatch, Project};
use agentcut_render::{
    CancelToken, FfmpegBackend,
    probe::{detect_kind, fingerprint},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Originals are addressed by bytes, never by a caller-controlled destination.
pub fn import(
    root: &Path,
    source: &Path,
    id: &str,
    expected: u64,
    key: &str,
    backend: &FfmpegBackend,
) -> Result<Outcome> {
    let mut store = Store::open(root)?;
    let mut input = File::open(source)?;
    let staged = root.join("cache").join(format!(
        "import-{}.{}",
        uuid::Uuid::new_v4(),
        source
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| s.len() <= 12 && s.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("media")
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        let mut fp = fingerprint(&staged, FingerprintStrategy::Sha256)?;
        fp.modified_unix_ns = None;
        let hash = fp
            .sha256
            .as_ref()
            .ok_or_else(|| Error::Invalid("missing content hash".into()))?;
        let relative = format!("originals/{hash}");
        let destination = root.join(&relative);
        if destination.exists() {
            if hash_file(&destination)? != *hash {
                return Err(Error::Invalid("existing original is corrupt".into()));
            }
        } else {
            // hard_link atomically publishes without replacing an existing original.
            std::fs::hard_link(&staged, &destination)?;
            File::open(root.join("originals"))?.sync_all()?;
        }
        let metadata = backend.prober().probe(&staged)?;
        if let Some(video) = &metadata.video
            && (matches!(
                video.color_transfer.as_deref(),
                Some("smpte2084" | "arib-std-b67")
            ) || video
                .color_primaries
                .as_deref()
                .is_some_and(|s| s.starts_with("bt2020")))
        {
            return Err(Error::Invalid(
                "HDR ingest is unsupported; supply an explicitly tone-mapped SDR derivative".into(),
            ));
        }
        let project = store.project()?;
        let batch: OperationBatch = serde_json::from_value(json!({
            "schemaVersion":"1.0.0", "projectId":project.project_id,"baseRevision":expected,"idempotencyKey":key,
            "description":"Managed original import", "operations":[{"id":format!("import-{id}"),"op":"asset.add","params":{"id":id,"uri":relative,"label":id,"kind":detect_kind(&metadata),"fingerprint":fp,"metadata":metadata}}]
        }))?;
        store.apply(&batch, false)
    })();
    let _ = std::fs::remove_file(&staged);
    result
}

pub fn verify_assets(root: &Path, project: &Project) -> Result<()> {
    for asset in &project.assets {
        let expected = asset
            .fingerprint
            .sha256
            .as_deref()
            .ok_or_else(|| Error::Invalid(format!("asset {} has no SHA-256", asset.id)))?;
        if asset.uri != format!("originals/{expected}") {
            return Err(Error::Invalid(format!(
                "asset {} is not a managed original",
                asset.id
            )));
        }
        if hash_file(&root.join(&asset.uri))? != expected {
            return Err(Error::Invalid(format!("asset {} bytes changed", asset.id)));
        }
    }
    Ok(())
}

/// Publish only a completely decoded, measured artifact; prior outputs stay intact.
pub fn render(root: &Path, sequence: &str, backend: &FfmpegBackend) -> Result<Value> {
    let store = Store::open(root)?;
    let project = store.project()?;
    let id = uuid::Uuid::new_v4().to_string();
    store.start_job(&id, project.revision)?;
    let result = render_job(root, sequence, backend, &project, &id);
    match &result {
        Ok(manifest) => store.finish_job(&id, manifest)?,
        Err(error) => store.fail_job(&id, &error.to_string())?,
    }
    result
}

fn render_job(
    root: &Path,
    sequence: &str,
    backend: &FfmpegBackend,
    project: &Project,
    id: &str,
) -> Result<Value> {
    verify_assets(root, project)?;
    let directory = root.join("renders").join(id);
    std::fs::create_dir(&directory)?;
    let staged = directory.join("unverified.mp4");
    let mut plan = backend.plan(project, root, sequence, "h264-mp4", &staged, None)?;
    // Upstream already compiles display-matrix transforms. Disable FFmpeg's
    // implicit autorotation so each input is rotated exactly once.
    let mut args = Vec::new();
    for arg in &plan.args {
        if arg == "-i" {
            args.push("-noautorotate".into());
        }
        args.push(arg.clone());
    }
    let output_index = args
        .len()
        .checked_sub(1)
        .ok_or_else(|| Error::Invalid("empty backend plan".into()))?;
    args.splice(
        output_index..output_index,
        [
            "-x264-params",
            "colorprim=bt709:transfer=bt709:colormatrix=bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-metadata:s:v:0",
            "rotate=0",
        ]
        .map(String::from),
    );
    plan.args = args;
    plan.plan_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"upstream":plan.plan_hash,"args":plan.args,"adapterVersion":1})
        )?)
    );
    std::fs::write(
        directory.join("plan.json"),
        serde_json::to_vec_pretty(&plan)?,
    )?;
    agentcut_render::run(&plan, false, &CancelToken::new(), &mut |_| {})?;
    let verified = verify(&staged, backend, plan.frame_count, project, sequence)?;
    let final_path = directory.join("video.mp4");
    let sheet = contact_sheet(&staged, &directory, backend, plan.frame_count)?;
    let manifest = json!({"artifactId":id,"projectId":project.project_id,"revision":project.revision,"sequenceId":sequence,"planHash":plan.plan_hash,"output":"video.mp4","verification":verified,"contactSheet":sheet,"snapshot":project});
    let mut manifest_file = File::create(directory.join("manifest.json"))?;
    manifest_file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    manifest_file.sync_all()?;
    File::open(&staged)?.sync_all()?;
    std::fs::rename(staged, &final_path)?;
    File::open(&directory)?.sync_all()?;
    Ok(
        json!({"artifactId":id,"revision":project.revision,"path":final_path,"manifest":directory.join("manifest.json")}),
    )
}

fn verify(
    path: &Path,
    backend: &FfmpegBackend,
    frames: i64,
    project: &Project,
    sequence: &str,
) -> Result<Value> {
    let decoded = Command::new(backend.ffmpeg_path())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()?;
    if !decoded.status.success() {
        return Err(Error::Invalid(format!(
            "decode failed: {}",
            String::from_utf8_lossy(&decoded.stderr)
        )));
    }
    let probe = Command::new(backend.ffprobe_path())
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;
    if !probe.status.success() {
        return Err(Error::Invalid("output probe failed".into()));
    }
    let metadata: Value = serde_json::from_slice(&probe.stdout)?;
    let streams = metadata["streams"]
        .as_array()
        .ok_or_else(|| Error::Invalid("no output streams".into()))?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| Error::Invalid("no video".into()))?;
    let seq = project.require_sequence(sequence)?;
    if video["nb_read_frames"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        != Some(frames)
        || video["width"] != seq.canvas.width
        || video["height"] != seq.canvas.height
    {
        return Err(Error::Invalid(
            "render dimensions or decoded frame count differs from plan".into(),
        ));
    }
    let expected_rate = format!(
        "{}/{}",
        seq.frame_rate.numerator, seq.frame_rate.denominator
    );
    if video["codec_name"] != "h264"
        || video["pix_fmt"] != "yuv420p"
        || video["avg_frame_rate"] != expected_rate
        || video["sample_aspect_ratio"] != "1:1"
        || video["color_transfer"] != "bt709"
        || video["color_primaries"] != "bt709"
        || video["color_space"] != "bt709"
    {
        return Err(Error::Invalid(
            "output differs from SDR H.264 delivery policy".into(),
        ));
    }
    let duration = frames as f64 / seq.frame_rate.as_f64();
    let has_audio = seq.tracks.iter().flat_map(|t| &t.items).any(|item| {
        item.enabled
            && matches!(&item.payload, agentcut_core::ItemPayload::Clip(clip)
            if clip.audio.enabled && project.assets.iter().any(|asset|
                asset.id == clip.asset_id && asset.metadata.audio.is_some()))
    });
    if has_audio {
        let audio = streams
            .iter()
            .find(|s| s["codec_type"] == "audio")
            .ok_or_else(|| Error::Invalid("expected audio is missing".into()))?;
        let audio_duration = audio["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| Error::Invalid("audio duration is missing".into()))?;
        if audio["codec_name"] != "aac" || (audio_duration - duration).abs() > 0.1 {
            return Err(Error::Invalid(
                "audio codec/duration differs from delivery policy".into(),
            ));
        }
    }
    Ok(json!({"decoded":true,"expectedFrames":frames,"sha256":hash_file(path)?,"probe":metadata}))
}

pub fn backend(ffmpeg: PathBuf, ffprobe: PathBuf) -> FfmpegBackend {
    FfmpegBackend::new(ffmpeg, ffprobe)
}

fn contact_sheet(
    path: &Path,
    directory: &Path,
    backend: &FfmpegBackend,
    frames: i64,
) -> Result<Value> {
    let step = (frames / 4).max(1);
    let filter = format!("select='not(mod(n,{step}))',scale=180:320,tile=4x1");
    let result = Command::new(backend.ffmpeg_path())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vf", &filter, "-frames:v", "1"])
        .arg(directory.join("sheet.png"))
        .output()?;
    if !result.status.success() {
        return Err(Error::Invalid(format!(
            "contact sheet failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(
        json!({"path":"sheet.png","outputFrames":[0,step,step*2,step*3],"tileWidth":180,"tileHeight":320}),
    )
}

/// Consistent DB snapshot plus every original referenced anywhere in history.
/// Derived renders are deliberately not required to resume editing.
pub fn backup(root: &Path, destination: &Path) -> Result<Value> {
    let store = Store::open(root)?;
    std::fs::create_dir(destination)?;
    std::fs::write(
        destination.join("backup.incomplete"),
        b"Do not open until backup completes",
    )?;
    for name in ["originals", "analysis", "cache", "renders", "exports"] {
        std::fs::create_dir(destination.join(name))?;
    }
    store.backup(&destination.join("project.sqlite"))?;
    let copied = Store::open_backup(destination)?;
    let mut hashes = std::collections::BTreeSet::new();
    for row in copied.history()? {
        let revision = row["revision"]
            .as_u64()
            .ok_or_else(|| Error::Invalid("invalid revision".into()))?;
        let project = copied.revision(revision)?;
        verify_assets(root, &project)?;
        for asset in &project.assets {
            if hashes.insert(asset.uri.clone()) {
                let target = destination.join(&asset.uri);
                std::fs::copy(root.join(&asset.uri), &target)?;
                File::open(&target)?.sync_all()?;
                if hash_file(&target)? != asset.fingerprint.sha256.as_deref().unwrap_or("") {
                    return Err(Error::Invalid("backup source changed during copy".into()));
                }
            }
        }
    }
    // Keep artifact records truthful: derived files were intentionally omitted.
    copied.mark_backup_jobs_unavailable()?;
    File::open(destination.join("originals"))?.sync_all()?;
    std::fs::remove_file(destination.join("backup.incomplete"))?;
    File::open(destination)?.sync_all()?;
    Ok(
        json!({"path":destination,"originalCount":hashes.len(),"revision":copied.project()?.revision,"derivedArtifactsIncluded":false}),
    )
}
