//! Versioned, immutable analysis jobs; accepting results is a separate edit.
use crate::{
    Error, Result, media,
    process::{self, Control},
};
use agentcut_core::Project;
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Task {
    FrameIndex {
        asset_id: String,
        start_ms: i64,
        end_ms: i64,
    },
    Proxy {
        asset_id: String,
        start_ms: i64,
        end_ms: i64,
        width: u32,
        height: u32,
    },
    Track {
        asset_id: String,
        start_ms: i64,
        end_ms: i64,
        region: crate::tracking::Region,
        sample_ms: i64,
        min_confidence: f64,
        max_motion_fraction: f64,
    },
    Provider {
        asset_id: String,
        task: String,
        parameters: Value,
    },
}
impl Task {
    pub fn asset_id(&self) -> &str {
        match self {
            Self::FrameIndex { asset_id, .. }
            | Self::Proxy { asset_id, .. }
            | Self::Track { asset_id, .. }
            | Self::Provider { asset_id, .. } => asset_id,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provider {
    pub program: PathBuf,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub task: Task,
    #[serde(default)]
    pub provider: Option<Provider>,
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
pub fn validate(input: &Input, project: &Project) -> Result<()> {
    let asset = project.require_asset(input.task.asset_id())?;
    match &input.task {
        Task::FrameIndex {
            start_ms, end_ms, ..
        }
        | Task::Proxy {
            start_ms, end_ms, ..
        }
        | Task::Track {
            start_ms, end_ms, ..
        } => {
            if *start_ms < 0
                || end_ms <= start_ms
                || end_ms - start_ms > 300_000
                || asset.metadata.video.is_none()
                || asset
                    .metadata
                    .duration
                    .is_some_and(|d| *end_ms as f64 > d.as_seconds_f64() * 1000.0 + 1.0)
            {
                return Err(invalid(
                    "video analysis requires an in-source interval of at most five minutes",
                ));
            }
        }
        Task::Provider {
            task, parameters, ..
        } => {
            crate::workflow::id(task)?;
            if input.provider.is_none() {
                return Err(invalid("external analysis provider is not configured"));
            }
            if !parameters.is_object() || serde_json::to_vec(parameters)?.len() > 1024 * 1024 {
                return Err(invalid("provider parameters require an object <=1 MiB"));
            }
        }
    }
    if let Task::Proxy { width, height, .. } = &input.task
        && (*width < 2
            || *height < 2
            || *width > 1280
            || *height > 1280
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2))
    {
        return Err(invalid("proxy canvas requires even dimensions in 2..1280"));
    }
    if let Task::Track {
        region,
        start_ms,
        end_ms,
        sample_ms,
        min_confidence,
        max_motion_fraction,
        ..
    } = &input.task
    {
        crate::tracking::validate(
            region,
            *start_ms,
            *end_ms,
            *sample_ms,
            *min_confidence,
            *max_motion_fraction,
        )?;
    }
    Ok(())
}
struct Deadline<'a> {
    inner: &'a mut dyn Control,
    started: Instant,
}
impl Control for Deadline<'_> {
    fn check(&mut self) -> Result<()> {
        if self.started.elapsed() > Duration::from_secs(900) {
            return Err(Error::Timeout);
        }
        self.inner.check()
    }
    fn stage(&mut self, stage: &str) -> Result<()> {
        self.check()?;
        self.inner.stage(stage)
    }
}
pub fn run(
    root: &Path,
    project: &Project,
    input: &Input,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<Value> {
    let started = Instant::now();
    let mut control = Deadline {
        inner: control,
        started,
    };
    validate(input, project)?;
    let asset = project.require_asset(input.task.asset_id())?;
    media::verify_asset_controlled(root, asset, &mut control)?;
    if let Some(provider) = &input.provider
        && media::hash_file_controlled(&provider.program, &mut control)? != provider.sha256
    {
        return Err(invalid("analysis provider executable changed"));
    }
    let tool = process::run(
        Command::new(backend.ffmpeg_path()).arg("-version"),
        Duration::from_secs(10),
        &mut control,
    )?;
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"source":asset.fingerprint.sha256,"input":input,"tool":String::from_utf8_lossy(&tool.stdout),"adapterVersion":2})
        )?)
    );
    let final_dir = root.join("analysis").join(&key);
    let manifest_path = final_dir.join("manifest.json");
    if manifest_path.exists() {
        let manifest: Value = crate::json::read(&manifest_path)?;
        let result_path = final_dir.join("result.json");
        if manifest["resultSha256"] != media::hash_file_controlled(&result_path, &mut control)? {
            return Err(invalid("cached analysis bytes changed"));
        }
        for file in manifest["files"]
            .as_array()
            .ok_or_else(|| invalid("analysis files missing"))?
        {
            let name = file["name"]
                .as_str()
                .ok_or_else(|| invalid("analysis filename missing"))?;
            if Path::new(name).components().count() != 1
                || file["sha256"]
                    != media::hash_file_controlled(&final_dir.join(name), &mut control)?
            {
                return Err(invalid("cached analysis attachment changed"));
            }
        }
        return Ok(
            json!({"path":result_path,"sha256":manifest["resultSha256"],"mimeType":"application/json","manifest":manifest_path,"cacheHit":true,"analysisKey":key,"needsReview":true}),
        );
    }
    if final_dir.exists() {
        return Err(invalid(
            "incomplete analysis cache requires collection before retry",
        ));
    }
    let scratch = root
        .join("cache")
        .join(format!("analysis-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&scratch)?;
    let result = (|| {
        let source = root.join(&asset.uri);
        let mut files = Vec::new();
        let data = match &input.task {
            Task::FrameIndex {
                start_ms, end_ms, ..
            } => {
                let origin = asset
                    .extensions
                    .get(crate::color::PROBE)
                    .and_then(|v| v["format"]["start_time"].as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let output = process::run(
                    Command::new(backend.ffprobe_path())
                        .args([
                            "-v",
                            "error",
                            "-protocol_whitelist",
                            "file",
                            "-select_streams",
                            "v:0",
                            "-read_intervals",
                            &format!(
                                "{}%{}",
                                *start_ms as f64 / 1000.0 + origin,
                                *end_ms as f64 / 1000.0 + origin
                            ),
                            "-show_frames",
                            "-show_entries",
                            "frame=pts,best_effort_timestamp,best_effort_timestamp_time,duration",
                            "-of",
                            "json",
                        ])
                        .arg(&source),
                    Duration::from_secs(900),
                    &mut control,
                )?;
                let raw: Value = crate::json::parse(&output.stdout)?;
                let frames = raw["frames"]
                    .as_array()
                    .ok_or_else(|| invalid("frame index missing"))?
                    .iter()
                    .filter(|f| {
                        f["best_effort_timestamp_time"]
                            .as_str()
                            .and_then(|s| s.parse::<f64>().ok())
                            .is_some_and(|t| {
                                t - origin >= *start_ms as f64 / 1000.0
                                    && t - origin < *end_ms as f64 / 1000.0
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if frames.is_empty() {
                    return Err(invalid("requested range contains no indexed frames"));
                }
                json!({"coveredSourceIntervalMs":[start_ms,end_ms],"sourceOriginSeconds":origin,"timeBase":asset.metadata.video.as_ref().and_then(|v|v.time_base),"frames":frames,"timestampPolicy":"original PTS/best effort timestamps, never average-fps frame numbering"})
            }
            Task::Proxy {
                start_ms,
                end_ms,
                width,
                height,
                ..
            } => {
                let path = scratch.join("proxy.mp4");
                let color = crate::color::source_filter(asset)?;
                let color = if color.is_empty() {
                    color
                } else {
                    format!("{color},")
                };
                let filter = format!(
                    "{color}scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1,fps=30"
                );
                process::run(
                    Command::new(backend.ffmpeg_path())
                        .args([
                            "-nostdin",
                            "-v",
                            "error",
                            "-noautorotate",
                            "-threads",
                            "1",
                            "-protocol_whitelist",
                            "file",
                            "-ss",
                            &format!("{}", *start_ms as f64 / 1000.0),
                            "-i",
                        ])
                        .arg(&source)
                        .args([
                            "-t",
                            &format!("{}", (*end_ms - *start_ms) as f64 / 1000.0),
                            "-vf",
                            &filter,
                            "-threads",
                            "2",
                            "-c:v",
                            "libx264",
                            "-preset",
                            "ultrafast",
                            "-crf",
                            "26",
                            "-pix_fmt",
                            "yuv420p",
                            "-x264-params",
                            "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                            "-c:a",
                            "aac",
                            "-metadata:s:v:0",
                            "rotate=0",
                        ])
                        .arg(&path),
                    Duration::from_secs(900),
                    &mut control,
                )?;
                process::run(
                    Command::new(backend.ffmpeg_path())
                        .args(["-nostdin", "-v", "error", "-xerror", "-i"])
                        .arg(&path)
                        .args(["-f", "null", "-"]),
                    Duration::from_secs(900),
                    &mut control,
                )?;
                files.push(json!({"name":"proxy.mp4","sha256":media::hash_file_controlled(&path,&mut control)?}));
                json!({"file":"proxy.mp4","quality":"preview","notForFinal":true,"sourceTimeMap":[{"outputStartMs":0,"outputEndMs":end_ms-start_ms,"sourceStartMs":start_ms,"sourceEndMs":end_ms}],"color":crate::color::capability(asset)})
            }
            Task::Track { .. } => serde_json::to_value(crate::tracking::track(
                &source,
                asset,
                &input.task,
                backend,
                &mut control,
            )?)?,
            Task::Provider {
                task, parameters, ..
            } => {
                let provider = input
                    .provider
                    .as_ref()
                    .ok_or_else(|| invalid("provider missing"))?;
                let request = scratch.join("provider-input.json");
                let output = scratch.join("provider-output.json");
                std::fs::write(
                    &request,
                    serde_json::to_vec(
                        &json!({"schemaVersion":1,"task":task,"source":{"path":source,"sha256":asset.fingerprint.sha256,"metadata":asset.metadata},"parameters":parameters,"outputPolicy":{"schemaVersion":1,"kind":"analysis","sourceSha256":asset.fingerprint.sha256,"maxBytes":8388608},"resourcePolicy":{"deadlineSeconds":900,"addressSpaceBytes":4294967296_u64,"noProjectMutation":true}}),
                    )?,
                )?;
                process::run(
                    Command::new(&provider.program)
                        .args(["--avw-request"])
                        .arg(&request)
                        .arg("--avw-result")
                        .arg(&output)
                        .current_dir(&scratch),
                    Duration::from_secs(900),
                    &mut control,
                )?;
                let output: Value = crate::json::read(&output)?;
                if output["schemaVersion"] != 1
                    || output["kind"] != "analysis"
                    || output["sourceSha256"] != serde_json::to_value(&asset.fingerprint.sha256)?
                    || !output["data"].is_object()
                {
                    return Err(invalid(
                        "provider output must match version, analysis kind, source hash and object data",
                    ));
                }
                output
            }
        };
        let report = json!({"schemaVersion":1,"sourceSha256":asset.fingerprint.sha256,"task":input.task,"analysisKey":key,"data":data,"files":files,"executedWallMs":started.elapsed().as_millis() as u64,"cost":{"providerDollars":null,"localCompute":"measured wall time; no configured dollar rate"},"reviewRequired":true});
        let output = scratch.join("result.json");
        let mut file = File::create(&output)?;
        file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        file.sync_all()?;
        let sha = media::hash_file_controlled(&output, &mut control)?;
        let mut manifest = File::create(scratch.join("manifest.json"))?;
        manifest.write_all(&serde_json::to_vec_pretty(
            &json!({"resultSha256":sha,"files":files}),
        )?)?;
        manifest.sync_all()?;
        for file in &files {
            File::open(
                scratch.join(
                    file["name"]
                        .as_str()
                        .ok_or_else(|| invalid("attachment name missing"))?,
                ),
            )?
            .sync_all()?;
        }
        control.check()?;
        std::fs::rename(&scratch, &final_dir)?;
        File::open(root.join("analysis"))?.sync_all()?;
        Ok(
            json!({"path":final_dir.join("result.json"),"sha256":sha,"mimeType":"application/json","manifest":final_dir.join("manifest.json"),"cacheHit":false,"analysisKey":key,"needsReview":true}),
        )
    })();
    if scratch.exists() {
        std::fs::remove_dir_all(scratch)?;
    }
    result
}
