//! Bounded, cached source evidence. Suggestions do not mutate an edit.
use crate::{
    Error, Result, media,
    process::{self, Uncontrolled},
    store::Store,
};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::File, path::Path, process::Command, time::Duration};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Metadata,
    Frame,
    Silence,
    Scenes,
}

pub fn inspect(
    root: &Path,
    asset_id: &str,
    kind: Kind,
    start_ms: i64,
    end_ms: Option<i64>,
    backend: &FfmpegBackend,
) -> Result<Value> {
    if start_ms < 0 {
        return Err(Error::Invalid("startMs must be nonnegative".into()));
    }
    let lease = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("analysis.lock"))?;
    lease.lock_shared()?;
    let project = Store::open(root)?.project()?;
    let asset = project.require_asset(asset_id)?;
    media::verify_asset(root, asset)?;
    let source = root.join(&asset.uri);
    if let Some(duration) = asset.metadata.duration
        && start_ms as f64 / 1000.0 >= duration.as_seconds_f64()
    {
        return Err(Error::Invalid(
            "inspection starts beyond source duration".into(),
        ));
    }
    if matches!(kind, Kind::Metadata) {
        let probe = process::run(
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
                .arg(&source),
            Duration::from_secs(60),
            &mut Uncontrolled,
        )?;
        return Ok(
            json!({"assetId":asset_id,"sha256":asset.fingerprint.sha256,"metadata":asset.metadata,"capability":crate::color::capability(asset),"raw":serde_json::from_slice::<Value>(&probe.stdout)?}),
        );
    }
    let end = end_ms.unwrap_or(start_ms + 60_000);
    if end <= start_ms || end - start_ms > 300_000 {
        return Err(Error::Invalid(
            "analysis range requires 0 < duration <= 300000 ms".into(),
        ));
    }
    let tool = process::run(
        Command::new(backend.ffmpeg_path()).arg("-version"),
        Duration::from_secs(10),
        &mut Uncontrolled,
    )?;
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"source":asset.fingerprint.sha256,"kind":kind,"startMs":start_ms,"endMs":end,"tool":String::from_utf8_lossy(&tool.stdout),"version":2})
        )?)
    );
    let directory = root.join("analysis");
    let manifest = directory.join(format!("{key}.json"));
    let image = directory.join(format!("{key}.png"));
    if manifest.exists() {
        let mut result: Value = crate::json::read(&manifest)?;
        if matches!(kind, Kind::Frame) && result["sha256"] != media::hash_file(&image)? {
            return Err(Error::Invalid("cached frame bytes changed".into()));
        }
        result["cacheHit"] = json!(true);
        return Ok(result);
    }
    let mut command = Command::new(backend.ffmpeg_path());
    command
        .args([
            "-nostdin",
            "-v",
            "info",
            "-protocol_whitelist",
            "file",
            "-ss",
            &format!("{:.3}", start_ms as f64 / 1000.0),
            "-noautorotate",
            "-i",
        ])
        .arg(source);
    let mut result = json!({"assetId":asset_id,"sourceHash":asset.fingerprint.sha256,"kind":kind,"startMs":start_ms,"endMs":end,"cacheHit":false});
    match kind {
        Kind::Frame => {
            let staged = directory.join(format!("{key}-{}.partial.png", uuid::Uuid::new_v4()));
            let color = crate::color::source_filter(asset)?;
            let filter = format!(
                "{}scale=1280:1280:force_original_aspect_ratio=decrease,setsar=1",
                if color.is_empty() {
                    color
                } else {
                    format!("{color},")
                }
            );
            command
                .args(["-vf", &filter, "-frames:v", "1"])
                .arg(&staged);
            process::run(&mut command, Duration::from_secs(60), &mut Uncontrolled)?;
            File::open(&staged)?.sync_all()?;
            std::fs::rename(&staged, &image)?;
            result["path"] = json!(image);
            result["sha256"] = json!(media::hash_file(&image)?);
            result["mimeType"] = json!("image/png");
            result["bytes"] = json!(std::fs::metadata(&image)?.len());
        }
        Kind::Silence | Kind::Scenes => {
            command.args(["-t", &format!("{:.3}", (end - start_ms) as f64 / 1000.0)]);
            if matches!(kind, Kind::Silence) {
                command.args(["-vn", "-af", "silencedetect=noise=-35dB:d=0.4"]);
            } else {
                command.args(["-an", "-vf", "select='gt(scene,0.35)',metadata=print"]);
            }
            command.args(["-f", "null", "-"]);
            let output = process::run(&mut command, Duration::from_secs(600), &mut Uncontrolled)?;
            let mut evidence = Vec::new();
            for line in output.stderr.lines() {
                for tag in [
                    "silence_start:",
                    "silence_end:",
                    "pts_time:",
                    "lavfi.scene_score=",
                ] {
                    if let Some((_, suffix)) = line.split_once(tag)
                        && let Some(value) = suffix
                            .split_whitespace()
                            .next()
                            .and_then(|s| s.parse::<f64>().ok())
                    {
                        let absolute = if tag == "lavfi.scene_score=" {
                            Value::Null
                        } else {
                            json!(start_ms + (value * 1000.0).round() as i64)
                        };
                        evidence.push(json!({"kind":tag.trim_end_matches([':', '=']),"value":value,"sourceTimeMs":absolute}));
                    }
                }
            }
            result["evidence"] = json!(evidence);
            result["suggestionsOnly"] = json!(true);
            result["diagnosticsMayBeTruncated"] = json!(output.stderr.len() >= 64 * 1024);
        }
        Kind::Metadata => unreachable!(),
    }
    let staged = directory.join(format!("{key}-{}.partial.json", uuid::Uuid::new_v4()));
    std::fs::write(&staged, serde_json::to_vec_pretty(&result)?)?;
    File::open(&staged)?.sync_all()?;
    std::fs::rename(staged, manifest)?;
    File::open(directory)?.sync_all()?;
    Ok(result)
}
