use crate::{
    Error, Result,
    store::{Outcome, Store},
};
use agentcut_core::{FingerprintStrategy, OperationBatch, Project};
use agentcut_render::{
    FfmpegBackend,
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
        let metadata = if matches!(
            staged.extension().and_then(|v| v.to_str()),
            Some("ttf" | "otf" | "ttc" | "otc" | "cube" | "3dl")
        ) {
            if std::fs::metadata(&staged)?.len() > 32 * 1024 * 1024 {
                return Err(Error::Invalid("font/LUT exceeds 32 MiB".into()));
            }
            backend.prober().probe(&staged)?
        } else {
            let result = crate::process::run(
                Command::new(backend.ffprobe_path())
                    .args([
                        "-v",
                        "error",
                        "-protocol_whitelist",
                        "file",
                        "-show_format",
                        "-show_streams",
                        "-of",
                        "json",
                    ])
                    .arg(&staged),
                std::time::Duration::from_secs(60),
                &mut crate::process::Uncontrolled,
            )?;
            agentcut_render::probe::normalize_probe_json(&serde_json::from_slice(&result.stdout)?)?
        };
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
        verify_asset(root, asset)?;
    }
    Ok(())
}

pub fn verify_asset(root: &Path, asset: &agentcut_core::Asset) -> Result<()> {
    let expected = asset
        .fingerprint
        .sha256
        .as_deref()
        .ok_or_else(|| Error::Invalid(format!("asset {} has no SHA-256", asset.id)))?;
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid("invalid asset content identity".into()));
    }
    let target = root.join(&asset.uri);
    if std::fs::symlink_metadata(&target)?.file_type().is_symlink() {
        return Err(Error::Invalid(
            "managed originals cannot be symbolic links".into(),
        ));
    }
    if asset.uri != format!("originals/{expected}") {
        return Err(Error::Invalid(format!(
            "asset {} is not a managed original",
            asset.id
        )));
    }
    if hash_file(&root.join(&asset.uri))? != expected {
        return Err(Error::Invalid(format!("asset {} bytes changed", asset.id)));
    }
    Ok(())
}

/// Publish only a completely decoded, measured artifact; prior outputs stay intact.
pub fn render(root: &Path, sequence: &str, backend: &FfmpegBackend) -> Result<Value> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("worker.lock"))?;
    lock.try_lock().map_err(|_| {
        Error::Invalid("worker is active; use render-start to queue the job".into())
    })?;
    let store = Store::open(root)?;
    let project = store.project()?;
    let id = uuid::Uuid::new_v4().to_string();
    store.start_job(&id, project.revision)?;
    let result = render_attempt(
        root,
        sequence,
        backend,
        &project,
        &id,
        &mut crate::process::Uncontrolled,
    );
    match &result {
        Ok(manifest) => store.finish_job(&id, manifest)?,
        Err(error) => store.fail_job(&id, &error.to_string())?,
    }
    result
}

pub(crate) fn render_attempt(
    root: &Path,
    sequence: &str,
    backend: &FfmpegBackend,
    project: &Project,
    id: &str,
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    control.check()?;
    verify_assets(root, project)?;
    let directory = root.join("renders").join(id);
    std::fs::create_dir(&directory)?;
    let staged = directory.join("unverified.mp4");
    let preset = agentcut_render::preset::require("h264-mp4")?;
    let normalized = agentcut_core::normalize::normalize_sequence(project, sequence)?;
    let ir = agentcut_render::ir::build(project, &normalized, root, preset, &staged, None, false)?;
    let info = crate::process::run(
        Command::new(backend.ffmpeg_path()).arg("-version"),
        std::time::Duration::from_secs(10),
        control,
    )?;
    let toolchain = String::from_utf8_lossy(&info.stdout)
        .lines()
        .next()
        .unwrap_or("unknown")
        .to_owned();
    let mut plan = agentcut_render::compile::compile(&ir, backend.ffmpeg_path(), &toolchain)?;
    if plan.duration_seconds > 3600.0 {
        return Err(Error::Invalid(
            "output exceeds the one-hour worker limit".into(),
        ));
    }
    let reserve = (plan.duration_seconds * 512_000.0) as u64 + 128 * 1024 * 1024;
    if crate::jobs::free_space(root)? < reserve {
        return Err(Error::Invalid(
            "insufficient scratch space for this render".into(),
        ));
    }

    // Upstream already compiles display-matrix transforms. Disable FFmpeg's
    // implicit autorotation so each input is rotated exactly once.
    let mut args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-filter_complex_threads".into(),
        "2".into(),
    ];
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
            "-threads",
            "2",
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
    let mut side_paths = Vec::new();
    for side in &plan.side_files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&side.path)?;
        file.write_all(&side.contents)?;
        side_paths.push(side.path.clone());
    }
    let execution = crate::process::run(
        Command::new(&plan.program).args(&plan.args),
        std::time::Duration::from_secs(8 * 3600),
        control,
    );
    for path in side_paths {
        let _ = std::fs::remove_file(path);
    }
    let output = execution?;
    std::fs::write(directory.join("stderr.log"), &output.stderr)?;
    std::fs::rename(&plan.temporary_output, &staged)?;
    control.stage("verifying")?;
    let verified = verify(
        &staged,
        backend,
        plan.frame_count,
        project,
        sequence,
        control,
    )?;
    let final_path = directory.join("video.mp4");
    let sheet = contact_sheet(&staged, &directory, backend, plan.frame_count, control)?;
    let manifest = json!({"artifactId":id,"projectId":project.project_id,"revision":project.revision,"sequenceId":sequence,"planHash":plan.plan_hash,"output":"video.mp4","verification":verified,"contactSheet":sheet,"snapshot":project});
    let mut manifest_file = File::create(directory.join("manifest.json"))?;
    manifest_file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    manifest_file.sync_all()?;
    File::open(&staged)?.sync_all()?;
    control.check()?;
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
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    crate::process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-v", "error", "-xerror", "-i"])
            .arg(path)
            .args(["-f", "null", "-"]),
        std::time::Duration::from_secs(4 * 3600),
        control,
    )?;
    let probe = crate::process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(path),
        std::time::Duration::from_secs(4 * 3600),
        control,
    )?;
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
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    let step = (frames / 4).max(1);
    let filter = format!("select='not(mod(n,{step}))',scale=180:320,tile=4x1");
    crate::process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-vf", &filter, "-frames:v", "1"])
            .arg(directory.join("sheet.png")),
        std::time::Duration::from_secs(4 * 3600),
        control,
    )?;
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
