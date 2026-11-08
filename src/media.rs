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
    hash_file_controlled(path, &mut crate::process::Uncontrolled)
}

pub fn hash_file_controlled(
    path: &Path,
    control: &mut dyn crate::process::Control,
) -> Result<String> {
    control.check()?;
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        control.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Reject existing symbolic links in project-owned directory slots before
/// opening their contents. This is a static path check, not a defense against
/// a concurrent process replacing directories after validation.
pub(crate) fn project_directory(root: &Path, name: &str) -> Result<PathBuf> {
    debug_assert!(matches!(name, "originals" | "analysis" | "cache"));
    let path = root.canonicalize()?.join(name);
    if !std::fs::symlink_metadata(&path)?.file_type().is_dir() {
        return Err(Error::Invalid(format!(
            "project {name} must be a real directory"
        )));
    }
    Ok(path)
}

/// Check a cached or managed file without following a symlink at its leaf.
pub(crate) fn regular_file(path: &Path, description: &str) -> Result<()> {
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(Error::Invalid(format!(
            "{description} must be a regular file"
        )));
    }
    Ok(())
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
    // Reject mistakes before staging multi-gigabyte originals or invoking tools.
    // Imports expand into a core operation batch, which already requires eight
    // bytes. Do not impose a new maximum on historical import keys.
    if key.trim().is_empty() || key.len() < 8 {
        return Err(Error::Invalid(
            "import idempotency key must contain at least 8 nonblank bytes; use a descriptive --key such as import-source-01".into(),
        ));
    }
    let cache = project_directory(root, "cache")?;
    let originals = project_directory(root, "originals")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("import.lock"))?;
    lock.try_lock().map_err(|_| {
        Error::Invalid("another import owns this project; retry after it finishes".into())
    })?;
    let mut store = Store::open(root)?;
    store.recover_imports(root)?;
    // A committed replay can legitimately name an old revision. Let change()
    // compare its full request after probing, as before. New stale requests can
    // fail now; change() still rechecks atomically against concurrent edits.
    if store.request_outcome(key)?.is_none() {
        let revision = store.project()?.revision;
        if revision != expected {
            return Err(Error::Conflict {
                expected,
                current: revision,
            });
        }
    }
    let source_metadata = std::fs::metadata(source)?;
    if !source_metadata.is_file() {
        return Err(Error::Invalid(
            "import source must be a regular file".into(),
        ));
    }
    if source_metadata.len() > crate::jobs::free_space(root)?.saturating_sub(128 * 1024 * 1024) {
        return Err(Error::Invalid(
            "insufficient space to stage the original".into(),
        ));
    }
    let special = matches!(
        source.extension().and_then(|v| v.to_str()),
        Some("ttf" | "otf" | "ttc" | "otc" | "cube" | "3dl")
    );
    if special && source_metadata.len() > 32 * 1024 * 1024 {
        return Err(Error::Invalid("font/LUT exceeds 32 MiB".into()));
    }
    let mut input = File::open(source)?;
    let import_id = uuid::Uuid::new_v4().to_string();
    let staged = cache.join(format!(
        "import-{}.{}",
        import_id,
        source
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| s.len() <= 12 && s.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("media")
    ));
    store.begin_import(
        &import_id,
        id,
        &format!(
            "cache/{}",
            staged
                .file_name()
                .ok_or_else(|| Error::Invalid("staging filename missing".into()))?
                .to_string_lossy()
        ),
    )?;
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        store.import_state(&import_id, "verifying", None)?;
        let mut fp = fingerprint(&staged, FingerprintStrategy::Sha256)?;
        fp.modified_unix_ns = None;
        let hash = fp
            .sha256
            .as_ref()
            .ok_or_else(|| Error::Invalid("missing content hash".into()))?;
        let relative = format!("originals/{hash}");
        let destination = originals.join(hash);
        let mut raw_probe = Value::Null;
        let mut metadata = if special {
            // Keep the staged-size check too: a source can grow during copy.
            if fp.size_bytes > 32 * 1024 * 1024 {
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
            {
                let raw: Value = serde_json::from_slice(&result.stdout)?;
                raw_probe = raw.clone();
                agentcut_render::probe::normalize_probe_json(&raw)?
            }
        };
        if metadata.video.as_ref().is_some_and(|v| {
            matches!(
                v.color_transfer.as_deref(),
                Some("smpte2084" | "arib-std-b67")
            )
        }) {
            crate::preserve::capture_metadata(
                backend,
                &staged,
                &mut raw_probe,
                &mut crate::process::Uncontrolled,
            )?;
        }
        if let Some(streams) = raw_probe["streams"].as_array() {
            let origin = raw_probe["format"]["start_time"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let duration = streams
                .iter()
                .filter_map(|s| {
                    let start = s["start_time"].as_str()?.parse::<f64>().ok()?;
                    let duration = s["duration"].as_str()?.parse::<f64>().ok()?;
                    Some(start + duration - origin)
                })
                .fold(0.0_f64, f64::max);
            if duration.is_finite() && duration > 0.0 {
                metadata.duration = Some(agentcut_core::RationalTime::new(
                    (duration * 1_000_000.0).round() as i64,
                    agentcut_core::RationalRate {
                        numerator: 1_000_000,
                        denominator: 1,
                    },
                ));
            }
        }
        if destination.exists() {
            regular_file(&destination, "existing managed original")?;
            if hash_file(&destination)? != *hash {
                return Err(Error::Invalid("existing original is corrupt".into()));
            }
        } else {
            // hard_link atomically publishes without replacing an existing original.
            std::fs::hard_link(&staged, &destination)?;
            File::open(root.join("originals"))?.sync_all()?;
        }
        store.import_state(&import_id, "ready", None)?;
        let project = store.project()?;
        let batch: OperationBatch = serde_json::from_value(json!({
            "schemaVersion":"1.0.0", "projectId":project.project_id,"baseRevision":expected,"idempotencyKey":key,
            "description":"Managed original import", "operations":[{"id":format!("import-{id}"),"op":"asset.add","params":{"id":id,"uri":relative,"label":id,"kind":detect_kind(&metadata),"fingerprint":fp,"metadata":metadata}}]
        }))?;
        store.change(
            key,
            expected,
            serde_json::to_value(&batch)?,
            false,
            |current| {
                let mut next = agentcut_core::apply_batch(current, &batch)?.project;
                let asset = next
                    .assets
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or_else(|| Error::Invalid("imported asset missing".into()))?;
                asset
                    .extensions
                    .insert(crate::color::PROBE.into(), raw_probe);
                Ok(next)
            },
        )
    })();
    let _ = std::fs::remove_file(&staged);
    store.import_state(
        &import_id,
        if result.is_ok() {
            "committed"
        } else {
            "failed"
        },
        result.as_ref().err().map(ToString::to_string),
    )?;
    result
}

pub fn verify_assets(root: &Path, project: &Project) -> Result<()> {
    for asset in &project.assets {
        verify_asset(root, asset)?;
    }
    Ok(())
}

pub fn verify_asset(root: &Path, asset: &agentcut_core::Asset) -> Result<()> {
    verify_asset_controlled(root, asset, &mut crate::process::Uncontrolled)
}

fn managed_original<'a>(
    root: &Path,
    asset: &'a agentcut_core::Asset,
) -> Result<(&'a str, PathBuf)> {
    let expected = asset
        .fingerprint
        .sha256
        .as_deref()
        .ok_or_else(|| Error::Invalid(format!("asset {} has no SHA-256", asset.id)))?;
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid("invalid asset content identity".into()));
    }
    if asset.uri != format!("originals/{expected}") {
        return Err(Error::Invalid(format!(
            "asset {} is not a managed original",
            asset.id
        )));
    }
    let target = project_directory(root, "originals")?.join(expected);
    regular_file(&target, "managed original")?;
    Ok((expected, target))
}

pub(crate) fn verify_asset_controlled(
    root: &Path,
    asset: &agentcut_core::Asset,
    control: &mut dyn crate::process::Control,
) -> Result<()> {
    let (expected, target) = managed_original(root, asset)?;
    if hash_file_controlled(&target, control)? != expected {
        return Err(Error::Invalid(format!("asset {} bytes changed", asset.id)));
    }
    Ok(())
}

// Identifies common replacements or writes between duplicate asset records.
// This is a static check, not isolation from concurrent filesystem mutations.
type OriginalStamp = (u64, u64, u64, i64, i64, i64, i64);

fn original_stamp(path: &Path) -> Result<OriginalStamp> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(Error::Invalid(
            "managed original must be a regular file".into(),
        ));
    }
    Ok((
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    ))
}

fn verify_render_originals_with<F>(
    root: &Path,
    project: &Project,
    control: &mut dyn crate::process::Control,
    mut hash: F,
) -> Result<()>
where
    F: FnMut(&Path, &mut dyn crate::process::Control) -> Result<String>,
{
    // This cache exists for one render verification only. Every asset still
    // validates its own identity, managed path and current regular-file status.
    let mut verified = std::collections::BTreeMap::<String, OriginalStamp>::new();
    for asset in &project.assets {
        let (expected, target) = managed_original(root, asset)?;
        control.check()?;
        let before = original_stamp(&target)?;
        if verified.get(expected) == Some(&before) {
            continue;
        }
        if hash(&target, control)? != expected {
            return Err(Error::Invalid(format!("asset {} bytes changed", asset.id)));
        }
        let after = original_stamp(&target)?;
        if before != after {
            return Err(Error::Invalid(format!(
                "asset {} changed during verification",
                asset.id
            )));
        }
        verified.insert(expected.to_owned(), after);
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
    let started = std::time::Instant::now();
    let mut timings = std::collections::BTreeMap::new();
    control.check()?;
    // Projects authored before the current protection rules may contain a
    // visual solo that hides a protected clip. Revalidate the frozen input,
    // even when no edit has occurred since that earlier revision.
    crate::policy::validate(project, project)?;
    // A collapsed subtitle interval cannot be represented by the sidecars.
    // Catch it before the potentially expensive encode, while preserving the
    // frozen input and any previously published artifact.
    crate::delivery::sidecars(project.require_sequence(sequence)?)?;
    let assembled = crate::assembly::policy(project.require_sequence(sequence)?)?.is_some();
    crate::audio_cleanup::validate_scope(project, sequence, assembled)?;
    verify_render_originals_with(root, project, control, hash_file_controlled)?;
    timings.insert("verifyOriginals", started.elapsed().as_secs_f64());
    let planning_started = std::time::Instant::now();
    let snapshot = project;
    let resolved = crate::preserve::resolve_metadata(project, root, backend, control)?;
    let project = &resolved;
    let directory = root.join("renders").join(id);
    std::fs::create_dir(&directory)?;
    let staged = directory.join("unverified.mp4");
    crate::animation::validate(project, sequence)?;
    let contract = crate::preserve::output(project, sequence)?;
    let preflight = crate::assembly::preflight(project, sequence)?;
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
    let (mut plan, seeks, color_decisions, cleanup) = if assembled {
        (
            crate::preserve::plan(project, root, sequence, &staged, backend, toolchain)?,
            std::collections::BTreeMap::new(),
            preflight["sources"].clone(),
            Vec::new(),
        )
    } else {
        let preset = agentcut_render::preset::require("h264-mp4")?;
        let instanced = instance_cut_sources(project, sequence)?;
        let (prepared, seeks) = seek_project(&instanced, sequence)?;
        let mut prepared = crate::color::geometry(&prepared);
        clear_disabled_track_solos(&mut prepared);
        let mut normalized = agentcut_core::normalize::normalize_sequence(&prepared, sequence)?;
        layout_caption_lines(&mut normalized)?;
        let mut ir =
            agentcut_render::ir::build(&prepared, &normalized, root, preset, &staged, None, false)?;
        for bus in &mut ir.buses {
            bus.effects.retain(|e| e.enabled);
        }
        if ir.warnings.iter().any(|w| w.code == "W_FONT_GLYPH_MISSING") {
            return Err(Error::Invalid(
                "caption font lacks required glyphs; bind a font covering every authored character"
                    .into(),
            ));
        }
        let mut plan = agentcut_render::compile::compile(&ir, backend.ffmpeg_path(), &toolchain)?;
        let decisions = crate::color::adapt(&mut plan, &instanced)?;
        crate::audio::adapt_fades(&mut plan, &ir)?;
        let cleanup = crate::audio_cleanup::adapt(&mut plan, &ir, project, sequence)?;
        if cleanup.iter().any(|item| item["status"] == "applied") {
            crate::audio_cleanup::require_filter(backend, control)?;
        }
        crate::audio::adapt(&mut plan, &ir, project.require_sequence(sequence)?)?;
        crate::animation::adapt(&mut plan, &ir, &prepared)?;
        (plan, seeks, decisions, cleanup)
    };
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

    // The source adapter normalizes physical geometry before placement.
    // Disable implicit autorotation so each input is rotated exactly once.
    let mut args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-filter_complex_threads".into(),
        "2".into(),
    ];
    let mut media_inputs = plan.inputs.iter().peekable();
    for (index, arg) in plan.args.iter().enumerate() {
        if arg == "-i" {
            let path = plan
                .args
                .get(index + 1)
                .ok_or_else(|| Error::Invalid("input path missing".into()))?;
            if media_inputs
                .peek()
                .is_some_and(|input| input.path.to_string_lossy() == *path)
            {
                let input = media_inputs.next().expect("peeked input");
                if let Some(seconds) = seeks.get(&input.asset_id) {
                    args.extend(["-ss".into(), seconds.to_string()]);
                }
            }
            // Codec options are scoped to the next input. The output-side
            // thread limit below does not bound decoder frame pools; automatic
            // decoder threads can exhaust the worker's 4 GiB address space.
            args.extend(["-threads".into(), "1".into(), "-noautorotate".into()]);
        }
        args.push(arg.clone());
    }
    let output_index = args
        .len()
        .checked_sub(1)
        .ok_or_else(|| Error::Invalid("empty backend plan".into()))?;
    let mut output_options = vec![
        "-threads".to_owned(),
        "2".to_owned(),
        "-metadata:s:v:0".to_owned(),
        "rotate=0".to_owned(),
    ];
    if !assembled {
        output_options.extend(
            [
                "-x264-params",
                "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ]
            .map(String::from),
        );
    }
    args.splice(output_index..output_index, output_options);
    plan.args = args;
    plan.plan_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"upstream":plan.plan_hash,"args":plan.args,"adapterVersion":10,"colorDecisions":color_decisions,"inputSeeksSeconds":seeks})
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
    timings.insert("plan", planning_started.elapsed().as_secs_f64());
    let encode_started = std::time::Instant::now();
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
    timings.insert("encode", encode_started.elapsed().as_secs_f64());
    let audio_started = std::time::Instant::now();
    let audio_qc = crate::audio::finish(
        &staged,
        backend,
        project.require_sequence(sequence)?,
        control,
    )?;
    timings.insert("audioFinish", audio_started.elapsed().as_secs_f64());
    control.stage("verifying")?;
    let verification_started = std::time::Instant::now();
    let needs_review = contract.codec == "hevc"
        || crate::assembly::policy(project.require_sequence(sequence)?)?
            .is_some_and(|p| p.quality == crate::assembly::Quality::Lossless);
    let verified = if needs_review {
        verify(
            &staged,
            backend,
            plan.frame_count,
            project,
            sequence,
            control,
        )?
    } else {
        verify_with_sheet(
            &staged,
            backend,
            plan.frame_count,
            project,
            sequence,
            Some(&directory),
            control,
        )?
    };
    timings.insert("verifyMaster", verification_started.elapsed().as_secs_f64());
    let final_path = directory.join("video.mp4");
    let review_started = std::time::Instant::now();
    let review = if needs_review {
        Some(crate::preserve::review_preview(
            &staged, &directory, project, sequence, backend, control,
        )?)
    } else {
        None
    };
    timings.insert("reviewPreview", review_started.elapsed().as_secs_f64());
    let delivery_started = std::time::Instant::now();
    let review_path = if review.is_some() {
        directory.join("review-sdr.mp4")
    } else {
        staged.clone()
    };
    let mut delivery = crate::delivery::extras(
        &review_path,
        &directory,
        project.require_sequence(sequence)?,
        backend,
        control,
    )?;
    if let Some(verification) = review {
        delivery["reviewVideo"] = json!("review-sdr.mp4");
        delivery["files"].as_array_mut().ok_or_else(|| Error::Invalid("delivery files missing".into()))?.push(json!({"path":"review-sdr.mp4","sha256":verification["sha256"],"mimeType":"video/mp4","verification":verification,"role":"SDR review preview; master remains source-preserving"}));
    }
    let sheet = contact_sheet_manifest(&directory, plan.frame_count, control)?;
    timings.insert("delivery", delivery_started.elapsed().as_secs_f64());
    timings.insert("totalBeforePublish", started.elapsed().as_secs_f64());
    let manifest = json!({"artifactId":id,"projectId":project.project_id,"revision":project.revision,"sequenceId":sequence,"planHash":plan.plan_hash,"output":"video.mp4","verification":verified,"contactSheet":sheet,"snapshot":snapshot,"inputSeeksSeconds":seeks,"colorDecisions":color_decisions,"audioQc":audio_qc,"audioCleanup":cleanup,"delivery":delivery,"outputColor":contract,"editPreflight":preflight,"timingsSeconds":timings,"resourcePolicy":{"childAddressSpaceBytes":4294967296_u64,"childFileBytes":34359738368_u64,"threads":2,"decoderThreadsPerInput":1,"encodeDeadlineSeconds":28800}});
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

pub(crate) fn verify(
    path: &Path,
    backend: &FfmpegBackend,
    frames: i64,
    project: &Project,
    sequence: &str,
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    verify_with_sheet(path, backend, frames, project, sequence, None, control)
}

pub(crate) fn verify_with_sheet(
    path: &Path,
    backend: &FfmpegBackend,
    frames: i64,
    project: &Project,
    sequence: &str,
    sheet_directory: Option<&Path>,
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    // Count the frames in the same strict decode that checks video and audio.
    // Passthrough forbids timestamp-driven duplication/drop; container nb_frames
    // alone is not evidence of a complete decode. Keep progress output bounded.
    let mut decode = Command::new(backend.ffmpeg_path());
    decode
        .args(["-v", "error", "-xerror", "-threads", "1", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-fps_mode",
            "passthrough",
            "-nostats",
            "-stats_period",
            "3600",
            "-progress",
            "pipe:1",
            "-f",
            "null",
            "-",
        ]);
    if let Some(directory) = sheet_directory {
        // The null output stays first so progress counts every decoded video
        // frame. A second output samples the same decoder for the review sheet;
        // finishing that image must not truncate the full validation decode.
        decode
            .args([
                "-map",
                "0:v:0",
                "-vf",
                &contact_sheet_filter(frames),
                "-frames:v",
                "1",
                "-an",
                "-threads",
                "1",
            ])
            .arg(directory.join("sheet.png"));
    }
    let decoded = crate::process::run(
        &mut decode,
        std::time::Duration::from_secs(4 * 3600),
        control,
    )?;
    let decoded_frames = decoded_frame_count(&decoded.stdout)?;
    let probe = crate::process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-threads",
                "1",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(path),
        std::time::Duration::from_secs(4 * 3600),
        control,
    )?;
    let mut metadata: Value = serde_json::from_slice(&probe.stdout)?;
    let video = metadata["streams"]
        .as_array_mut()
        .and_then(|streams| streams.iter_mut().find(|s| s["codec_type"] == "video"))
        .ok_or_else(|| Error::Invalid("no video".into()))?;
    // Preserve the manifest field consumed by review/delivery timing checks.
    video["nb_read_frames"] = json!(decoded_frames.to_string());
    let contract = crate::preserve::output(project, sequence)?;
    if contract.hdr {
        crate::preserve::capture_metadata(backend, path, &mut metadata, control)?;
    }
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
    if video["codec_name"] != contract.codec
        || video["pix_fmt"] != contract.pixel_format
        || video["avg_frame_rate"] != expected_rate
        || video["sample_aspect_ratio"] != "1:1"
        || video["color_transfer"] != contract.transfer
        || video["color_primaries"] != contract.primaries
        || video["color_space"] != contract.matrix
        || video["color_range"] != "tv"
    {
        return Err(Error::Invalid(
            "output differs from the declared color/codec/bit-depth delivery policy".into(),
        ));
    }
    if contract.hdr {
        let first = crate::assembly::clips(seq)?[0];
        let agentcut_core::ItemPayload::Clip(clip) = &first.payload else {
            unreachable!()
        };
        let expected = crate::preserve::hdr_metadata(project.require_asset(&clip.asset_id)?)?;
        if crate::preserve::probe_hdr_metadata(&metadata)? != expected {
            return Err(Error::Invalid(
                "HDR mastering metadata differs from the source contract".into(),
            ));
        }
    }
    let duration = frames as f64 / seq.frame_rate.as_f64();
    if has_expected_audio(project, sequence)? {
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
    Ok(
        json!({"decoded":true,"frameCountMethod":"ffmpeg-passthrough-decode-progress","expectedFrames":frames,"sha256":hash_file_controlled(path, control)?,"probe":metadata}),
    )
}

fn decoded_frame_count(progress: &[u8]) -> Result<i64> {
    let text = std::str::from_utf8(progress)
        .map_err(|_| Error::Invalid("invalid decode progress".into()))?;
    let mut frames = None;
    let mut complete = false;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("frame=") {
            frames = value.trim().parse::<i64>().ok().filter(|n| *n >= 0);
        }
        if let Some(value) = line.strip_prefix("progress=") {
            complete = value == "end";
        }
    }
    frames
        .filter(|_| complete)
        .ok_or_else(|| Error::Invalid("decode did not report a final frame count".into()))
}

pub fn backend(ffmpeg: PathBuf, ffprobe: PathBuf) -> FfmpegBackend {
    FfmpegBackend::new(ffmpeg, ffprobe)
}

fn contact_sheet_filter(frames: i64) -> String {
    let step = (frames / 4).max(1);
    format!(
        "select='not(mod(n,{step}))',scale=180:320:force_original_aspect_ratio=decrease,pad=180:320:(ow-iw)/2:(oh-ih)/2,tile=4x1"
    )
}

fn contact_sheet_manifest(
    directory: &Path,
    frames: i64,
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    let step = (frames / 4).max(1);
    let selected: Vec<_> = (0..4)
        .map(|n| n * step)
        .take_while(|&n| n < frames)
        .collect();
    File::open(directory.join("sheet.png"))?.sync_all()?;
    Ok(
        json!({"path":"sheet.png","outputFrames":selected,"tileWidth":180,"tileHeight":320,"sha256":hash_file_controlled(&directory.join("sheet.png"),control)?}),
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
    let objects=hashes.iter().map(|uri|Ok(json!({"path":uri,"sha256":hash_file(&destination.join(uri))?,"bytes":std::fs::metadata(destination.join(uri))?.len()}))).collect::<Result<Vec<_>>>()?;
    let manifest = json!({"format":"avw-portable-project","version":1,"projectId":copied.project()?.project_id,"revision":copied.project()?.revision,"objects":objects,"derivedArtifactsIncluded":false});
    let mut manifest_file = File::create(destination.join("backup-manifest.json"))?;
    manifest_file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    manifest_file.sync_all()?;
    // Keep artifact records truthful: derived files were intentionally omitted.
    copied.mark_backup_jobs_unavailable()?;
    File::open(destination.join("originals"))?.sync_all()?;
    std::fs::remove_file(destination.join("backup.incomplete"))?;
    File::open(destination)?.sync_all()?;
    Ok(
        json!({"path":destination,"originalCount":hashes.len(),"revision":copied.project()?.revision,"derivedArtifactsIncluded":false}),
    )
}

/// FFmpeg 8 drawtext can display the line separator as a missing glyph.
/// Give each caption line its own text plane, preserving the original cue in
/// the project and its source map. No line separator reaches drawtext.
pub fn layout_caption_lines(
    normalized: &mut agentcut_core::normalize::NormalizedSequence,
) -> Result<()> {
    use agentcut_core::{Crop, FitMode, Size2d, VerticalAlignment};
    for layer in &mut normalized.video_layers {
        let mut laid_out = Vec::new();
        for item in &layer.items {
            let Some(text) = &item.text else {
                laid_out.push(item.clone());
                continue;
            };
            if item.caption.is_none() || !text.text.contains('\n') {
                laid_out.push(item.clone());
                continue;
            }
            let lines: Vec<_> = text.text.lines().collect();
            if lines.len() > 3 {
                return Err(Error::Invalid(
                    "caption exceeds three renderable lines; split the cue".into(),
                ));
            }
            let height = (text.style.font_size * text.style.line_height)
                .ceil()
                .max(1.0);
            let block_height = height * lines.len() as f64;
            let top = match text.style.vertical_alignment {
                VerticalAlignment::Top => text.transform.position.y - text.text_box.height / 2.0,
                VerticalAlignment::Middle => text.transform.position.y - block_height / 2.0,
                VerticalAlignment::Bottom => {
                    text.transform.position.y + text.text_box.height / 2.0 - block_height
                }
            };
            for (n, line) in lines.iter().enumerate() {
                let mut line_item = item.clone();
                line_item.item_id = format!("{}_line_{n}", item.item_id);
                let mut line_text = text.clone();
                line_text.text = (*line).into();
                line_text.text_box.height = height;
                line_text.transform.position.y = top + height * (n as f64 + 0.5);
                line_text.style.vertical_alignment = VerticalAlignment::Middle;
                line_item.placement = Some(agentcut_core::normalize::resolve_placement(
                    Size2d {
                        width: line_text.text_box.width.ceil() as u32,
                        height: height as u32,
                    },
                    normalized.canvas,
                    FitMode::None,
                    Crop::none(),
                    &line_text.transform,
                )?);
                line_item.text = Some(line_text);
                if let Some(caption) = &mut line_item.caption {
                    caption.text = (*line).into();
                }
                laid_out.push(line_item);
            }
        }
        layer.items = laid_out;
    }
    Ok(())
}

fn clear_disabled_track_solos(project: &mut Project) {
    // The pinned normalizer counts disabled solos before skipping disabled
    // tracks. Clear them only in this render copy, including nested sequences.
    for sequence in &mut project.sequences {
        for track in &mut sequence.tracks {
            if !track.enabled {
                track.solo = false;
            }
        }
    }
}

fn has_expected_audio(project: &Project, sequence: &str) -> Result<bool> {
    // Check the same per-kind solo selection and nested expansion as the
    // compositor, rather than treating a visual solo as an audio solo.
    let mut prepared = project.clone();
    clear_disabled_track_solos(&mut prepared);
    let normalized = agentcut_core::normalize::normalize_sequence(&prepared, sequence)?;
    let duration = if normalized.duration.value > 0 {
        normalized.duration
    } else {
        agentcut_core::RationalTime::new(1, normalized.frame_rate)
    };
    let range = agentcut_core::TimeRange::new(
        agentcut_core::RationalTime::zero(normalized.frame_rate),
        duration,
    )?;
    for item in normalized
        .video_layers
        .iter()
        .chain(&normalized.audio_layers)
        .flat_map(|layer| &layer.items)
    {
        if item.audio.as_ref().is_some_and(|audio| audio.enabled)
            && let Some(asset_id) = &item.asset_id
            && prepared.require_asset(asset_id)?.metadata.audio.is_some()
            && item.timeline_range.intersects(range)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Give repeated normal-speed cuts independent decoder clocks. Sharing one
/// decoder across distant source ranges makes FFmpeg queue full-resolution
/// frames for earlier branches while a later branch consumes up to its trim.
/// These aliases exist only in the render preparation; project history and
/// source identities remain unchanged. Complex transition/nested paths retain
/// their existing compiler behavior rather than changing handle semantics.
fn instance_cut_sources(project: &Project, sequence: &str) -> Result<Project> {
    use agentcut_core::{AssetKind, ItemPayload};
    let seq = project.require_sequence(sequence)?;
    if !seq.transitions.is_empty()
        || seq
            .tracks
            .iter()
            .flat_map(|t| &t.items)
            .any(|i| matches!(i.payload, ItemPayload::Sequence(_)))
    {
        return Ok(project.clone());
    }
    let mut counts = std::collections::BTreeMap::new();
    for item in seq.tracks.iter().flat_map(|t| &t.items) {
        if let ItemPayload::Clip(clip) = &item.payload {
            *counts.entry(clip.asset_id.clone()).or_insert(0_usize) += 1;
        }
    }
    let mut prepared = project.clone();
    let mut assets = Vec::new();
    let mut next_id = 0;
    for item in prepared
        .require_sequence_mut(sequence)?
        .tracks
        .iter_mut()
        .flat_map(|t| &mut t.items)
    {
        if let ItemPayload::Clip(clip) = &mut item.payload {
            let source = project.require_asset(&clip.asset_id)?;
            if counts.get(&clip.asset_id).copied().unwrap_or(0) < 2
                || !clip.video.speed.is_normal()
                || !matches!(source.kind, AssetKind::Video | AssetKind::Audio)
            {
                continue;
            }
            let id = loop {
                let id = format!("avw_render_input_{next_id}");
                next_id += 1;
                if project.asset(&id).is_none() {
                    break id;
                }
            };
            let mut asset = source.clone();
            asset.id = id.clone();
            asset
                .extensions
                .insert("avw.render.sourceAssetId".into(), json!(source.id));
            clip.asset_id = id;
            assets.push(asset);
        }
    }
    prepared.assets.extend(assets);
    Ok(prepared)
}

fn seek_project(
    project: &Project,
    sequence: &str,
) -> Result<(Project, std::collections::BTreeMap<String, i64>)> {
    use agentcut_core::{AssetKind, ItemPayload, RationalRate, RationalTime};
    let seq = project.require_sequence(sequence)?;
    if seq
        .tracks
        .iter()
        .flat_map(|t| &t.items)
        .any(|item| matches!(item.payload, ItemPayload::Sequence(_)))
    {
        return Ok((project.clone(), std::collections::BTreeMap::new()));
    }
    let mut starts = std::collections::BTreeMap::new();
    let mut disabled = std::collections::BTreeSet::new();
    for item in seq.tracks.iter().flat_map(|t| &t.items) {
        if let ItemPayload::Clip(clip) = &item.payload {
            if !clip.video.speed.is_normal() {
                disabled.insert(clip.asset_id.clone());
            }
            if matches!(
                project.require_asset(&clip.asset_id)?.kind,
                AssetKind::Video | AssetKind::Audio
            ) {
                let entry = starts
                    .entry(clip.asset_id.clone())
                    .or_insert(clip.source_range.start);
                if clip.source_range.start.checked_cmp(*entry)?.is_lt() {
                    *entry = clip.source_range.start;
                }
            }
        }
    }
    let seeks: std::collections::BTreeMap<_, _> = starts
        .into_iter()
        .filter(|(id, _)| !disabled.contains(id))
        .filter_map(|(id, time)| {
            let seconds = time.as_seconds();
            let whole = seconds.numerator / seconds.denominator;
            i64::try_from(whole)
                .ok()
                .filter(|s| *s > 0)
                .map(|s| (id, s))
        })
        .collect();
    let mut prepared = project.clone();
    let seq = prepared
        .sequence_mut(sequence)
        .ok_or_else(|| Error::Invalid("sequence missing".into()))?;
    for item in seq.tracks.iter_mut().flat_map(|t| &mut t.items) {
        if let ItemPayload::Clip(clip) = &mut item.payload
            && let Some(&seconds) = seeks.get(&clip.asset_id)
        {
            clip.source_range.start = clip.source_range.start.checked_sub(RationalTime::new(
                seconds,
                RationalRate {
                    numerator: 1,
                    denominator: 1,
                },
            ))?;
        }
    }
    Ok((prepared, seeks))
}

#[cfg(test)]
mod render_input_tests {
    use super::*;
    #[test]
    fn decode_count_requires_complete_valid_progress() {
        assert_eq!(
            decoded_frame_count(b"frame=1\nprogress=continue\nframe=900\nprogress=end\n").unwrap(),
            900
        );
        for invalid in [
            b"frame=900\nprogress=continue\n".as_slice(),
            b"progress=end\n",
            b"frame=-1\nprogress=end\n",
            b"frame=bad\nprogress=end\n",
        ] {
            assert!(decoded_frame_count(invalid).is_err());
        }
    }
    use agentcut_core::{
        Asset, AssetKind, AudioMetadata, BlendMode, ClipData, Fingerprint, ItemPayload,
        MediaMetadata, RationalRate, Sequence, SequenceRefData, TimeRange, TimelineItem, Track,
        TrackType, Transform2d,
    };

    fn duplicate_original_fixture() -> (tempfile::TempDir, Project, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("originals")).unwrap();
        let bytes = vec![0x5a; 256 * 1024 + 1];
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let path = root.path().join("originals").join(&sha);
        std::fs::write(&path, &bytes).unwrap();
        let mut project = Project::new("duplicate", 64, 64, RationalRate::frames(30).unwrap());
        let mut fingerprint = Fingerprint::unprobed();
        fingerprint.strategy = FingerprintStrategy::Sha256;
        fingerprint.size_bytes = bytes.len() as u64;
        fingerprint.sha256 = Some(sha.clone());
        let first = Asset {
            id: "first".into(),
            kind: AssetKind::Video,
            uri: format!("originals/{sha}"),
            proxy_uri: None,
            label: "generated".into(),
            fingerprint,
            metadata: MediaMetadata::unprobed(),
            tags: Vec::new(),
            extensions: Default::default(),
        };
        let mut second = first.clone();
        second.id = "second".into();
        project.assets.extend([first, second]);
        (root, project, path)
    }

    #[test]
    fn repeated_original_is_hashed_once_per_render_and_rechecked_next_time() {
        let (root, mut project, path) = duplicate_original_fixture();
        let other = b"another generated original";
        let other_sha = format!("{:x}", Sha256::digest(other));
        std::fs::write(root.path().join("originals").join(&other_sha), other).unwrap();
        let mut distinct = project.assets[0].clone();
        distinct.id = "distinct".into();
        distinct.uri = format!("originals/{other_sha}");
        distinct.fingerprint.size_bytes = other.len() as u64;
        distinct.fingerprint.sha256 = Some(other_sha);
        project.assets.push(distinct);

        let mut reads = 0;
        verify_render_originals_with(
            root.path(),
            &project,
            &mut crate::process::Uncontrolled,
            |path, control| {
                reads += 1;
                hash_file_controlled(path, control)
            },
        )
        .unwrap();
        assert_eq!(reads, 2, "two different originals must each be hashed");

        verify_render_originals_with(
            root.path(),
            &project,
            &mut crate::process::Uncontrolled,
            |path, control| {
                reads += 1;
                hash_file_controlled(path, control)
            },
        )
        .unwrap();
        assert_eq!(reads, 4, "verification cannot persist across renders");

        std::fs::write(&path, b"changed original").unwrap();
        let error = verify_render_originals_with(
            root.path(),
            &project,
            &mut crate::process::Uncontrolled,
            hash_file_controlled,
        )
        .unwrap_err();
        assert!(error.to_string().contains("asset first bytes changed"));
    }

    #[test]
    fn repeated_original_still_validates_each_asset_and_regular_path() {
        let (root, project, path) = duplicate_original_fixture();
        let mut wrong_uri = project.clone();
        wrong_uri.assets[1].uri = "originals/not-the-digest".into();
        let error = verify_render_originals_with(
            root.path(),
            &wrong_uri,
            &mut crate::process::Uncontrolled,
            hash_file_controlled,
        )
        .unwrap_err();
        assert!(error.to_string().contains("asset second is not a managed original"));

        let mut invalid_sha = project.clone();
        invalid_sha.assets[1].fingerprint.sha256 = Some("g".repeat(64));
        let error = verify_render_originals_with(
            root.path(),
            &invalid_sha,
            &mut crate::process::Uncontrolled,
            hash_file_controlled,
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid asset content identity"));

        let mut missing_sha = project.clone();
        missing_sha.assets[1].fingerprint.sha256 = None;
        let error = verify_render_originals_with(
            root.path(),
            &missing_sha,
            &mut crate::process::Uncontrolled,
            hash_file_controlled,
        )
        .unwrap_err();
        assert!(error.to_string().contains("asset second has no SHA-256"));

        let outside = root.path().join("outside");
        std::fs::write(&outside, b"outside").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(
            verify_render_originals_with(
                root.path(),
                &project,
                &mut crate::process::Uncontrolled,
                hash_file_controlled,
            )
            .is_err()
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            verify_render_originals_with(
                root.path(),
                &project,
                &mut crate::process::Uncontrolled,
                hash_file_controlled,
            )
            .is_err()
        );
    }

    #[test]
    fn a_changed_duplicate_is_rehashed_before_the_render() {
        use std::cell::Cell;

        struct ChangeAfterFirst<'a> {
            armed: &'a Cell<bool>,
            path: PathBuf,
        }
        impl crate::process::Control for ChangeAfterFirst<'_> {
            fn check(&mut self) -> Result<()> {
                if self.armed.replace(false) {
                    std::fs::write(&self.path, b"changed size")?;
                }
                Ok(())
            }
        }

        let (root, project, path) = duplicate_original_fixture();
        let armed = Cell::new(false);
        let reads = Cell::new(0);
        let mut control = ChangeAfterFirst {
            armed: &armed,
            path,
        };
        let error = verify_render_originals_with(
            root.path(),
            &project,
            &mut control,
            |path, control| {
                reads.set(reads.get() + 1);
                let digest = hash_file_controlled(path, control)?;
                if reads.get() == 1 {
                    armed.set(true);
                }
                Ok(digest)
            },
        )
        .unwrap_err();
        assert_eq!(reads.get(), 2);
        assert!(error.to_string().contains("asset second bytes changed"));
    }

    #[test]
    fn a_symlink_replacing_a_duplicate_is_not_reused() {
        use std::cell::Cell;

        struct ReplaceAfterFirst<'a> {
            armed: &'a Cell<bool>,
            path: PathBuf,
            outside: PathBuf,
        }
        impl crate::process::Control for ReplaceAfterFirst<'_> {
            fn check(&mut self) -> Result<()> {
                if self.armed.replace(false) {
                    std::fs::remove_file(&self.path)?;
                    std::os::unix::fs::symlink(&self.outside, &self.path)?;
                }
                Ok(())
            }
        }

        let (root, project, path) = duplicate_original_fixture();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"unverified").unwrap();
        let armed = Cell::new(false);
        let reads = Cell::new(0);
        let mut control = ReplaceAfterFirst {
            armed: &armed,
            path,
            outside,
        };
        let error = verify_render_originals_with(
            root.path(),
            &project,
            &mut control,
            |path, control| {
                reads.set(reads.get() + 1);
                let digest = hash_file_controlled(path, control)?;
                armed.set(true);
                Ok(digest)
            },
        )
        .unwrap_err();
        assert_eq!(reads.get(), 1);
        assert!(error.to_string().contains("managed original must be a regular file"));
    }

    #[test]
    fn render_original_verification_checks_cancellation_during_hashing() {
        struct CancelAfterChecks(usize);
        impl crate::process::Control for CancelAfterChecks {
            fn check(&mut self) -> Result<()> {
                self.0 += 1;
                if self.0 >= 4 {
                    Err(Error::Cancelled)
                } else {
                    Ok(())
                }
            }
        }

        let (root, project, _) = duplicate_original_fixture();
        let result = verify_render_originals_with(
            root.path(),
            &project,
            &mut CancelAfterChecks(0),
            hash_file_controlled,
        );
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    #[ignore = "requires AVW_TEST_FFMPEG and AVW_TEST_FFPROBE"]
    fn single_decode_verifies_real_media_and_rejects_damage() {
        let backend = backend(
            std::env::var("AVW_TEST_FFMPEG")
                .expect("test FFmpeg path")
                .into(),
            std::env::var("AVW_TEST_FFPROBE")
                .expect("test ffprobe path")
                .into(),
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.mp4");
        let status = Command::new(backend.ffmpeg_path())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=64x64:r=30000/1001",
                "-frames:v",
                "360",
                "-c:v",
                "libx264",
                "-threads",
                "1",
                "-x264-params",
                "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                "-pix_fmt",
                "yuv420p",
                "-color_range",
                "tv",
                "-movflags",
                "+faststart",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let project = Project::new(
            "decode",
            64,
            64,
            RationalRate {
                numerator: 30000,
                denominator: 1001,
            },
        );
        let mut control = crate::process::Uncontrolled;
        let verified = verify(&path, &backend, 360, &project, "seq_main", &mut control).unwrap();
        assert_eq!(verified["probe"]["streams"][0]["nb_read_frames"], "360");
        assert!(verify(&path, &backend, 359, &project, "seq_main", &mut control).is_err());
        let bytes = std::fs::read(&path).unwrap();
        // Keep the faststart header but truncate the actual encoded frames.
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(verify(&path, &backend, 360, &project, "seq_main", &mut control).is_err());
    }

    #[test]
    #[ignore = "requires AVW_TEST_FFMPEG and AVW_TEST_FFPROBE"]
    fn combined_decode_keeps_sheet_pixels_and_validates_after_the_last_tile() {
        let backend = backend(
            std::env::var("AVW_TEST_FFMPEG")
                .expect("test FFmpeg path")
                .into(),
            std::env::var("AVW_TEST_FFPROBE")
                .expect("test ffprobe path")
                .into(),
        );
        let temp = tempfile::tempdir().unwrap();
        let project = Project::new(
            "sheets",
            64,
            64,
            RationalRate {
                numerator: 30000,
                denominator: 1001,
            },
        );
        let mut control = crate::process::Uncontrolled;
        for frames in [1, 2, 3, 5, 360] {
            let directory = temp.path().join(frames.to_string());
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("source.mp4");
            let status = Command::new(backend.ffmpeg_path())
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=s=64x64:r=30000/1001",
                    "-frames:v",
                    &frames.to_string(),
                    "-c:v",
                    "libx264",
                    "-threads",
                    "1",
                    "-x264-params",
                    "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                    "-pix_fmt",
                    "yuv420p",
                    "-color_range",
                    "tv",
                    "-movflags",
                    "+faststart",
                ])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            let verification = verify_with_sheet(
                &path,
                &backend,
                frames,
                &project,
                "seq_main",
                Some(&directory),
                &mut control,
            )
            .unwrap();
            assert_eq!(verification["expectedFrames"], frames);
            let manifest = contact_sheet_manifest(&directory, frames, &mut control).unwrap();
            if frames < 4 {
                assert_eq!(
                    manifest["outputFrames"],
                    json!((0..frames).collect::<Vec<_>>())
                );
            }
            // The previous standalone sheet path is an independent pixel
            // reference. Matching PNG bytes includes scaling/padding/tile order.
            let legacy = directory.join("legacy.png");
            let status = Command::new(backend.ffmpeg_path())
                .args(["-v", "error", "-threads", "1", "-i"])
                .arg(&path)
                .args(["-vf", &contact_sheet_filter(frames), "-frames:v", "1"])
                .arg(&legacy)
                .status()
                .unwrap();
            assert!(status.success());
            assert_eq!(
                std::fs::read(directory.join("sheet.png")).unwrap(),
                std::fs::read(&legacy).unwrap()
            );
            if frames == 360 {
                // The last sampled tile is frame 270. Damage near the end of
                // the stream must still fail, despite already sampling it.
                let bytes = std::fs::read(&path).unwrap();
                std::fs::write(&path, &bytes[..bytes.len() * 95 / 100]).unwrap();
                let failed = directory.join("failed");
                std::fs::create_dir(&failed).unwrap();
                assert!(
                    verify_with_sheet(
                        &path,
                        &backend,
                        frames,
                        &project,
                        "seq_main",
                        Some(&failed),
                        &mut control
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn disabled_solos_are_cleared_for_nested_render_preparation_only() {
        let rate = RationalRate::frames(30).unwrap();
        let mut project = Project::new("nested", 360, 640, rate);
        let mut metadata = MediaMetadata::unprobed();
        metadata.audio = Some(AudioMetadata {
            sample_rate: 48000,
            channels: 1,
            channel_layout: "mono".into(),
            sample_format: "fltp".into(),
        });
        project.assets.push(Asset {
            id: "source".into(),
            kind: AssetKind::Video,
            uri: "originals/retained".into(),
            proxy_uri: None,
            label: "source".into(),
            fingerprint: Fingerprint::unprobed(),
            metadata,
            tags: Vec::new(),
            extensions: Default::default(),
        });
        let range = TimeRange::new(crate::workflow::time(0), crate::workflow::time(1000)).unwrap();
        let clip = |id: &str| TimelineItem {
            id: id.into(),
            name: id.into(),
            enabled: true,
            start: range.start,
            duration: range.duration,
            linked_group_id: None,
            payload: ItemPayload::Clip(ClipData::new("source", range)),
            effects: Vec::new(),
            keyframes: Vec::new(),
            extensions: Default::default(),
        };
        let mut child = Sequence::new("child", "Child", 360, 640, rate, 48000);
        child.tracks[0].id = "child-video".into();
        child.tracks[1].id = "child-audio".into();
        let mut video = clip("video");
        if let ItemPayload::Clip(ref mut item) = video.payload {
            item.audio.enabled = false;
        }
        child.tracks[0].items.push(video);
        child.tracks[1].items.push(clip("audio"));
        for (id, kind) in [
            ("disabled-video", TrackType::Video),
            ("disabled-audio", TrackType::Audio),
        ] {
            let mut track = Track::empty(id, id, kind, 1);
            track.solo = true;
            track.enabled = false;
            child.tracks.push(track);
        }
        let nested = SequenceRefData {
            sequence_id: "child".into(),
            source_range: range,
            transform: Transform2d::centered(project.sequences[0].canvas),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
        };
        for (index, id) in [(0, "nested-video"), (1, "nested-audio")] {
            project.sequences[0].tracks[index].items.push(TimelineItem {
                id: id.into(),
                name: id.into(),
                enabled: true,
                start: range.start,
                duration: range.duration,
                linked_group_id: None,
                payload: ItemPayload::Sequence(nested.clone()),
                effects: Vec::new(),
                keyframes: Vec::new(),
                extensions: Default::default(),
            });
        }
        project.sequences.push(child);
        let original = project.clone();
        let without_fix =
            agentcut_core::normalize::normalize_sequence(&project, "seq_main").unwrap();
        assert!(without_fix.video_layers.is_empty() && without_fix.audio_layers.is_empty());
        let mut prepared = project.clone();
        clear_disabled_track_solos(&mut prepared);
        let with_fix = agentcut_core::normalize::normalize_sequence(&prepared, "seq_main").unwrap();
        assert_eq!(
            with_fix.video_layers[0].items[0].item_id,
            "nested-video__video"
        );
        assert_eq!(
            with_fix.audio_layers[0].items[0].item_id,
            "nested-audio__audio"
        );
        assert_eq!(project, original);
        assert!(has_expected_audio(&project, "seq_main").unwrap());

        // A visual solo must not make audio QC ignore the active audio track.
        project.sequences[0].tracks[0].solo = true;
        assert!(has_expected_audio(&project, "seq_main").unwrap());
        // A separate enabled audio solo still excludes non-solo audio tracks.
        let mut solo = Track::empty("active-solo-audio", "active", TrackType::Audio, 1);
        solo.solo = true;
        project.sequences[0].tracks.push(solo);
        assert!(!has_expected_audio(&project, "seq_main").unwrap());
    }

    #[test]
    fn repeated_cut_inputs_preserve_source_clocks_and_original_project() {
        let mut project = Project::new("cuts", 1080, 1920, RationalRate::frames(30).unwrap());
        let source = Asset {
            id: "source".into(),
            kind: AssetKind::Video,
            uri: "originals/retained".into(),
            proxy_uri: None,
            label: "source".into(),
            fingerprint: Fingerprint::unprobed(),
            metadata: MediaMetadata::unprobed(),
            tags: Vec::new(),
            extensions: Default::default(),
        };
        project.assets.push(source.clone());
        let mut collision = source.clone();
        collision.id = "avw_render_input_0".into();
        project.assets.push(collision);
        for (i, start) in [16900, 6100].into_iter().enumerate() {
            project.sequences[0].tracks[0].items.push(TimelineItem {
                id: format!("cut{i}"),
                name: "cut".into(),
                enabled: true,
                start: crate::workflow::time(i as i64 * 3000),
                duration: crate::workflow::time(3000),
                linked_group_id: None,
                payload: ItemPayload::Clip(ClipData::new(
                    "source",
                    TimeRange {
                        start: crate::workflow::time(start),
                        duration: crate::workflow::time(3000),
                    },
                )),
                effects: Vec::new(),
                keyframes: Vec::new(),
                extensions: Default::default(),
            });
        }
        let original = project.clone();
        let instanced = instance_cut_sources(&project, "seq_main").unwrap();
        let (prepared, seeks) = seek_project(&instanced, "seq_main").unwrap();
        assert_eq!(project, original);
        assert_eq!(prepared.assets.len(), 4);
        for (before, after) in original.sequences[0].tracks[0]
            .items
            .iter()
            .zip(&prepared.sequences[0].tracks[0].items)
        {
            assert_eq!(
                (before.start, before.duration),
                (after.start, after.duration)
            );
            let (ItemPayload::Clip(old), ItemPayload::Clip(new)) =
                (&before.payload, &after.payload)
            else {
                panic!("expected clips")
            };
            assert_ne!(old.asset_id, new.asset_id);
            let asset = prepared.require_asset(&new.asset_id).unwrap();
            assert_eq!(asset.uri, source.uri);
            assert_eq!(asset.fingerprint, source.fingerprint);
            assert_eq!(asset.extensions["avw.render.sourceAssetId"], "source");
            let restored = new
                .source_range
                .start
                .checked_add(crate::workflow::time(seeks[&new.asset_id] * 1000))
                .unwrap();
            assert!(
                restored
                    .checked_cmp(old.source_range.start)
                    .unwrap()
                    .is_eq()
            );
        }
        assert_eq!(seeks.values().copied().collect::<Vec<_>>(), vec![16, 6]);
    }
}
