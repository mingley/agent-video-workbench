//! Check a caller's delivery brief against one frozen, hash-verified render.
//!
//! These are checks of explicitly requested, measurable properties. They do
//! not assert upload acceptance, editorial quality, caption accuracy or the
//! authenticity of a manifest whose hash is stored beside the artifact.
use crate::{
    Error, Result, media,
    process::{self, Uncontrolled},
    store::Store,
};
use agentcut_core::{ItemPayload, Sequence};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum Sidecar {
    Srt,
    Vtt,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Color {
    Sdr,
    HdrPq,
    HdrHlg,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryBrief {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sidecars: Vec<Sidecar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

impl DeliveryBrief {
    /// Validate entirely before opening a project or reading an artifact.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("delivery brief schemaVersion must be 1"));
        }
        for value in [&self.dimensions, &self.aspect_ratio].into_iter().flatten() {
            if value.width == 0 || value.height == 0 {
                return Err(invalid(
                    "delivery brief dimensions and aspect ratio must be positive",
                ));
            }
        }
        if self.max_duration_ms == Some(0) || self.max_bytes == Some(0) {
            return Err(invalid("delivery brief limits must be positive"));
        }
        let unique: BTreeSet<_> = self.required_sidecars.iter().collect();
        if unique.len() != self.required_sidecars.len() {
            return Err(invalid("delivery brief contains duplicate sidecars"));
        }
        if self.dimensions.is_none()
            && self.aspect_ratio.is_none()
            && self.max_duration_ms.is_none()
            && self.max_bytes.is_none()
            && self.required_sidecars.is_empty()
            && self.color.is_none()
        {
            return Err(invalid("delivery brief requires at least one constraint"));
        }
        Ok(())
    }
}

fn scoped_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
    .canonicalize()?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(invalid(
            "delivery artifact is outside the project or is not a file",
        ));
    }
    Ok(resolved)
}

fn valid_sha(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn signed_integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

// An absent orientation transform means no display rotation. A display matrix
// can contain scale, shear or reflection beyond the extracted rotation, so it
// is intentionally not treated as a pure rotation.
fn rotation(video: &Value) -> Option<i64> {
    let mut rotations = Vec::new();
    if let Some(tags) = video.get("tags") {
        let tags = tags.as_object()?;
        if let Some(value) = tags.get("rotate") {
            rotations.push(signed_integer(value)?);
        }
    }
    if let Some(sides) = video.get("side_data_list") {
        for side in sides.as_array()? {
            if side["side_data_type"] == "Display Matrix" {
                return None;
            }
        }
    }
    let mut result = None;
    for angle in rotations {
        if angle % 90 != 0 {
            return None;
        }
        let normalized = angle.rem_euclid(360);
        if result.is_some_and(|previous| previous != normalized) {
            return None;
        }
        result = Some(normalized);
    }
    Some(result.unwrap_or(0))
}

fn ratio(value: &Value, separator: char) -> Option<(u64, u64)> {
    let (a, b) = value.as_str()?.split_once(separator)?;
    let (a, b) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
    (a > 0 && b > 0).then_some((a, b))
}

// ffprobe prints container times as decimal seconds. Keep those decimals
// exact rather than rounding a fractional millisecond down to a passing value.
fn decimal_seconds(value: &Value) -> Option<(i128, i128)> {
    let text = value.as_str()?;
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > 9
    {
        return None;
    }
    let denominator = 10_i128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let whole: i128 = whole.parse().ok()?;
    let fraction: i128 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    let numerator = whole.checked_mul(denominator)?.checked_add(fraction)?;
    Some((
        if negative {
            numerator.checked_neg()?
        } else {
            numerator
        },
        denominator,
    ))
}

fn decimal_duration_ms(value: &Value) -> Option<(u128, u128)> {
    let (num, den) = decimal_seconds(value)?;
    if num <= 0 {
        return None;
    }
    Some((
        u128::try_from(num.checked_mul(1000)?).ok()?,
        u128::try_from(den).ok()?,
    ))
}

fn stream_duration_ms(stream: &Value) -> Option<(u128, u128)> {
    let ticks = stream["duration_ts"].as_u64()?;
    let (num, den) = ratio(&stream["time_base"], '/')?;
    if ticks == 0 {
        return None;
    }
    Some((
        u128::from(ticks)
            .checked_mul(u128::from(num))?
            .checked_mul(1000)?,
        u128::from(den),
    ))
}

fn stream_end_ms(stream: &Value, container_start: Option<(i128, i128)>) -> Option<(u128, u128)> {
    let (start_num, start_den) = container_start?;
    let start = i128::from(signed_integer(&stream["start_pts"])?);
    let duration = i128::from(stream["duration_ts"].as_u64()?);
    if duration <= 0 {
        return None;
    }
    let (num, den) = ratio(&stream["time_base"], '/')?;
    let num = i128::from(num);
    let den = i128::from(den);
    let end = start
        .checked_add(duration)?
        .checked_mul(num)?
        .checked_mul(start_den)?
        .checked_sub(start_num.checked_mul(den)?)?;
    if end <= 0 {
        return None;
    }
    Some((
        u128::try_from(end.checked_mul(1000)?).ok()?,
        u128::try_from(den.checked_mul(start_den)?).ok()?,
    ))
}

// A stream beginning before the declared container origin can make the
// union of streams longer than either its individual duration or the latest
// end measured from that origin. Do not treat that ambiguous union as a pass.
fn starts_at_or_after_container(
    stream: &Value,
    container_start: Option<(i128, i128)>,
) -> Option<bool> {
    let (start_num, start_den) = container_start?;
    let pts = i128::from(signed_integer(&stream["start_pts"])?);
    let (num, den) = ratio(&stream["time_base"], '/')?;
    let left = pts.checked_mul(i128::from(num))?.checked_mul(start_den)?;
    let right = start_num.checked_mul(i128::from(den))?;
    Some(left >= right)
}

fn within_maximum(duration: Option<(u128, u128)>, maximum: u64) -> Option<bool> {
    let (numerator, denominator) = duration?;
    Some(numerator <= u128::from(maximum).checked_mul(denominator)?)
}

fn dimensions(video: &Value) -> Option<(u64, u64, i64)> {
    let (width, height) = (video["width"].as_u64()?, video["height"].as_u64()?);
    if width == 0 || height == 0 {
        return None;
    }
    let angle = rotation(video)?;
    Some(if angle == 90 || angle == 270 {
        (height, width, angle)
    } else {
        (width, height, angle)
    })
}

fn add(
    checks: &mut Vec<Value>,
    name: &str,
    requested: Value,
    status: &str,
    actual: Value,
    reason: &str,
) {
    checks.push(json!({"constraint":name,"requested":requested,"status":status,"actual":actual,"reason":reason}));
}

fn color_tags(video: &Value) -> Option<(&str, &str, &str)> {
    let get = |key| {
        video
            .get(key)?
            .as_str()
            .filter(|v| !matches!(*v, "" | "unknown" | "unspecified" | "reserved" | "N/A"))
    };
    Some((
        get("color_primaries")?,
        get("color_transfer")?,
        get("color_space")?,
    ))
}

fn sidecar(
    root: &Path,
    directory: &Path,
    manifest: &Value,
    sequence: &Sequence,
    requested: Sidecar,
) -> (&'static str, Value, &'static str) {
    let name = match requested {
        Sidecar::Srt => "captions.srt",
        Sidecar::Vtt => "captions.vtt",
    };
    let Some(files) = manifest["delivery"]["files"].as_array() else {
        return (
            "unknown",
            Value::Null,
            "sidecar manifest information is unavailable",
        );
    };
    let entries: Vec<_> = files.iter().filter(|f| f["path"] == name).collect();
    if entries.is_empty() {
        return (
            "fail",
            json!({"path":name}),
            "required sidecar is not listed in the render manifest",
        );
    }
    if entries.len() != 1 {
        return (
            "unknown",
            json!({"path":name}),
            "the sidecar manifest contains duplicate entries",
        );
    }
    let Some(expected) = valid_sha(&entries[0]["sha256"]) else {
        return (
            "unknown",
            json!({"path":name}),
            "sidecar hash evidence is missing or malformed",
        );
    };
    let path = directory.join(name);
    let path = match path.canonicalize() {
        Ok(path) if path.starts_with(root) && path.is_file() => path,
        Ok(_) => {
            return (
                "fail",
                json!({"path":name}),
                "sidecar is not a project file",
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ("fail", json!({"path":name}), "required sidecar is missing");
        }
        Err(_) => {
            return (
                "unknown",
                json!({"path":name}),
                "sidecar could not be inspected",
            );
        }
    };
    match media::hash_file(&path) {
        Ok(hash) if hash == expected => {}
        Ok(_) => {
            return (
                "fail",
                json!({"path":name}),
                "required sidecar bytes do not match its recorded hash",
            );
        }
        Err(_) => {
            return (
                "unknown",
                json!({"path":name}),
                "sidecar bytes could not be read",
            );
        }
    }
    let Ok((srt, vtt, mapping)) = crate::delivery::sidecars(sequence) else {
        return (
            "unknown",
            json!({"path":name,"sha256":expected}),
            "frozen sequence sidecars could not be reconstructed",
        );
    };
    let Some(cues) = mapping["cues"].as_array() else {
        return (
            "unknown",
            Value::Null,
            "frozen caption cue evidence is unavailable",
        );
    };
    if cues.is_empty() {
        return (
            "fail",
            json!({"path":name,"sha256":expected,"cueCount":0}),
            "frozen sequence has no caption cues",
        );
    }
    let cue_ids: BTreeSet<_> = cues
        .iter()
        .filter_map(|cue| cue["itemId"].as_str())
        .collect();
    if cue_ids.len() != cues.len() {
        return (
            "unknown",
            Value::Null,
            "frozen caption cue identities are invalid",
        );
    }
    let has_text = sequence.tracks.iter().flat_map(|track| &track.items).any(|item| {
        cue_ids.contains(item.id.as_str())
            && matches!(&item.payload, ItemPayload::Caption(caption) if !caption.text.trim().is_empty())
    });
    if !has_text {
        return (
            "fail",
            json!({"path":name,"sha256":expected,"cueCount":cues.len()}),
            "caption cues have no nonblank text",
        );
    }
    let generated = match requested {
        Sidecar::Srt => srt.as_bytes(),
        Sidecar::Vtt => vtt.as_bytes(),
    };
    if format!("{:x}", Sha256::digest(generated)) != expected {
        return (
            "unknown",
            json!({"path":name,"sha256":expected,"cueCount":cues.len()}),
            "sidecar does not match this version's reconstruction of the frozen captions",
        );
    }
    if media::hash_file(&path).ok().as_deref() != Some(expected) {
        return (
            "unknown",
            json!({"path":name}),
            "sidecar changed or became unreadable during delivery check",
        );
    }
    (
        "pass",
        json!({"path":name,"sha256":expected,"cueCount":cues.len()}),
        "verified sidecar matches the frozen sequence and contains nonblank caption text",
    )
}

/// Evaluate a brief against one successful render without modifying the project.
pub fn check(
    root: &Path,
    id: &str,
    brief: &DeliveryBrief,
    backend: &FfmpegBackend,
) -> Result<Value> {
    brief.validate()?;
    let root = root.canonicalize()?;
    let store = Store::open(&root)?;
    let job = store.job(id)?;
    if job.state != "succeeded" {
        return Err(invalid("delivery check requires a succeeded render job"));
    }
    let result = job
        .result
        .as_ref()
        .ok_or_else(|| invalid("render result missing"))?;
    let revision = u64::try_from(job.revision).map_err(|_| invalid("invalid job revision"))?;
    let snapshot = store.revision(revision)?;
    let manifest_path = scoped_file(
        &root,
        Path::new(
            result["manifest"]
                .as_str()
                .ok_or_else(|| invalid("render manifest missing"))?,
        ),
    )?;
    let manifest: Value = crate::json::read(&manifest_path)?;
    let sequence = manifest["sequenceId"]
        .as_str()
        .ok_or_else(|| invalid("manifest sequence missing"))?;
    let artifact = result["artifactId"]
        .as_str()
        .ok_or_else(|| invalid("render artifact ID missing"))?;
    if manifest["projectId"] != json!(snapshot.project_id)
        || manifest["revision"] != revision
        || result["revision"] != revision
        || manifest["artifactId"] != artifact
        || manifest["snapshot"] != serde_json::to_value(&snapshot)?
    {
        return Err(invalid(
            "render manifest does not match its frozen project revision",
        ));
    }
    let frozen_sequence = snapshot.require_sequence(sequence)?;
    let input = job.input.as_ref().ok_or_else(|| invalid(
        "this synchronous render does not bind its sequence to a durable job; re-render with render-start and check that new job",
    ))?;
    if input.sequence != sequence
        || input.analysis.is_some()
        || input.asr.is_some()
        || input.expected_revision != revision
        || job.attempt < 1
        || artifact != format!("{}-{}", job.id, job.attempt)
    {
        return Err(invalid(
            "render manifest does not match its frozen job input",
        ));
    }
    let binding = "job-input";
    let master = scoped_file(
        &root,
        Path::new(
            result["path"]
                .as_str()
                .ok_or_else(|| invalid("render path missing"))?,
        ),
    )?;
    let directory = manifest_path
        .parent()
        .ok_or_else(|| invalid("manifest directory missing"))?;
    if scoped_file(&root, &directory.join("video.mp4"))? != master
        || manifest["output"] != "video.mp4"
    {
        return Err(invalid("render path and manifest disagree"));
    }
    let expected = valid_sha(&manifest["verification"]["sha256"])
        .ok_or_else(|| invalid("render hash missing or malformed"))?;
    let sha = media::hash_file(&master)?;
    if sha != expected {
        return Err(invalid("rendered master bytes changed"));
    }
    let needs_probe = brief.dimensions.is_some()
        || brief.aspect_ratio.is_some()
        || brief.max_duration_ms.is_some()
        || brief.color.is_some();
    let probe = if needs_probe {
        process::run(Command::new(backend.ffprobe_path()).args(["-v","error","-protocol_whitelist","file","-threads","1","-show_entries","format=start_time,duration:stream=codec_type,width,height,sample_aspect_ratio,start_pts,duration_ts,time_base,color_primaries,color_transfer,color_space:stream_tags=rotate:stream_disposition=attached_pic:stream_side_data=side_data_type,rotation","-of","json"]).arg(&master), Duration::from_secs(60), &mut Uncontrolled)
            .ok().and_then(|result| serde_json::from_slice::<Value>(&result.stdout).ok())
    } else {
        None
    };
    if media::hash_file(&master)? != sha {
        return Err(invalid(
            "rendered master bytes changed during delivery check",
        ));
    }
    let bytes = std::fs::metadata(&master)?.len();
    let videos = probe
        .as_ref()
        .and_then(|p| p["streams"].as_array())
        .and_then(|streams| {
            let videos = streams
                .iter()
                .filter(|stream| stream["codec_type"] == "video")
                .collect::<Vec<_>>();
            if videos.iter().any(|stream| {
                !matches!(stream["disposition"]["attached_pic"].as_u64(), Some(0 | 1))
            }) {
                return None;
            }
            Some(
                videos
                    .into_iter()
                    .filter(|stream| stream["disposition"]["attached_pic"] == 0)
                    .collect::<Vec<_>>(),
            )
        });
    let video = match videos.as_deref() {
        Some([video]) => Some(*video),
        _ => None,
    };
    let mut checks = Vec::new();
    if let Some(requested) = &brief.dimensions {
        if let Some((width, height, angle)) = video.and_then(dimensions) {
            add(
                &mut checks,
                "dimensions",
                json!(requested),
                if width == u64::from(requested.width) && height == u64::from(requested.height) {
                    "pass"
                } else {
                    "fail"
                },
                json!({"width":width,"height":height,"rotationDegrees":angle}),
                "display-oriented raster dimensions from a fresh probe of the verified master",
            );
        } else {
            add(
                &mut checks,
                "dimensions",
                json!(requested),
                "unknown",
                Value::Null,
                "verified dimensions or display rotation are unavailable",
            );
        }
    }
    if let Some(requested) = &brief.aspect_ratio {
        let measured = video.and_then(|v| {
            dimensions(v).and_then(|(width, height, angle)| {
                ratio(&v["sample_aspect_ratio"], ':').and_then(|(a, b)| {
                    let (a, b) = if angle == 90 || angle == 270 {
                        (b, a)
                    } else {
                        (a, b)
                    };
                    Some((
                        u128::from(width).checked_mul(u128::from(a))?,
                        u128::from(height).checked_mul(u128::from(b))?,
                    ))
                })
            })
        });
        if let Some((width, height)) = measured {
            let lhs = width.checked_mul(u128::from(requested.height));
            let rhs = height.checked_mul(u128::from(requested.width));
            if let (Some(lhs), Some(rhs)) = (lhs, rhs) {
                add(
                    &mut checks,
                    "aspectRatio",
                    json!(requested),
                    if lhs == rhs { "pass" } else { "fail" },
                    json!({"widthUnits":width.to_string(),"heightUnits":height.to_string()}),
                    "exact display aspect ratio from raster, sample aspect ratio and rotation",
                );
            } else {
                add(
                    &mut checks,
                    "aspectRatio",
                    json!(requested),
                    "unknown",
                    Value::Null,
                    "aspect ratio arithmetic overflow",
                );
            }
        } else {
            add(
                &mut checks,
                "aspectRatio",
                json!(requested),
                "unknown",
                Value::Null,
                "sample aspect ratio, dimensions or display rotation are unavailable",
            );
        }
    }
    if let Some(maximum) = brief.max_duration_ms {
        let container_duration = probe
            .as_ref()
            .and_then(|p| decimal_duration_ms(&p["format"]["duration"]));
        let container_start = probe
            .as_ref()
            .and_then(|p| decimal_seconds(&p["format"]["start_time"]));
        let streams = probe.as_ref().and_then(|p| p["streams"].as_array());
        let mut verdicts = vec![within_maximum(container_duration, maximum)];
        if video.is_none() {
            verdicts.push(None);
        }
        let actual = |duration: Option<(u128, u128)>| {
            duration.map(|(num,den)| json!({"millisecondsNumerator":num.to_string(),"millisecondsDenominator":den.to_string()}))
        };
        let mut actual_streams = Vec::new();
        if let Some(streams) = streams {
            for stream in streams {
                let kind = stream["codec_type"].as_str();
                if !matches!(kind, Some("audio" | "video" | "subtitle")) {
                    verdicts.push(None);
                    continue;
                }
                if kind == Some("video") && stream["disposition"]["attached_pic"] == 1 {
                    continue;
                }
                let duration = stream_duration_ms(stream);
                let end = stream_end_ms(stream, container_start);
                let start_within_container = starts_at_or_after_container(stream, container_start);
                verdicts.extend([
                    within_maximum(duration, maximum),
                    within_maximum(end, maximum),
                ]);
                if start_within_container != Some(true) {
                    verdicts.push(None);
                }
                actual_streams.push(json!({"type":kind,"duration":actual(duration),"endFromContainerStart":actual(end),"startsAtOrAfterContainer":start_within_container}));
            }
        } else {
            verdicts.push(None);
        }
        let status = if verdicts.contains(&Some(false)) {
            "fail"
        } else if verdicts.contains(&None) {
            "unknown"
        } else {
            "pass"
        };
        add(
            &mut checks,
            "maxDurationMs",
            json!(maximum),
            status,
            json!({"container":actual(container_duration),"streams":actual_streams}),
            "fresh-probed container duration and every audio/video/subtitle stream's duration and end relative to container start must be within the limit; streams starting before the container make the result unknown; these are metadata rather than measured playback",
        );
    }
    if let Some(maximum) = brief.max_bytes {
        add(
            &mut checks,
            "maxBytes",
            json!(maximum),
            if bytes <= maximum { "pass" } else { "fail" },
            json!(bytes),
            "actual verified master file size in bytes",
        );
    }
    if let Some(requested) = brief.color {
        if let Some(tags) = video.and_then(color_tags) {
            let expected = match requested {
                Color::Sdr => ("bt709", "bt709", "bt709"),
                Color::HdrPq => ("bt2020", "smpte2084", "bt2020nc"),
                Color::HdrHlg => ("bt2020", "arib-std-b67", "bt2020nc"),
            };
            add(
                &mut checks,
                "color",
                json!(requested),
                if tags == expected { "pass" } else { "fail" },
                json!({"primaries":tags.0,"transfer":tags.1,"matrix":tags.2}),
                "freshly probed video color tags only; this does not assess appearance or mastering metadata",
            );
        } else {
            add(
                &mut checks,
                "color",
                json!(requested),
                "unknown",
                Value::Null,
                "video color tags are unavailable or unspecified",
            );
        }
    }
    for requested in &brief.required_sidecars {
        let (status, actual, reason) =
            sidecar(&root, directory, &manifest, frozen_sequence, *requested);
        add(
            &mut checks,
            "requiredSidecar",
            json!(requested),
            status,
            actual,
            reason,
        );
    }
    let status = if checks.iter().any(|c| c["status"] == "fail") {
        "fail"
    } else if checks.iter().any(|c| c["status"] == "unknown") {
        "unknown"
    } else {
        "pass"
    };
    Ok(
        json!({"schemaVersion":1,"status":status,"brief":brief,"artifact":{"jobId":id,"artifactId":artifact,"revision":revision,"sequenceId":sequence,"sequenceBinding":binding,"path":master,"sha256":sha,"bytes":bytes,"manifest":manifest_path},"checks":checks,"limitations":["When metadata is requested, the master is probed again and rehashed; the manifest is not independently authenticated", "Durations are metadata, not an independent playback measurement", "Caption sidecar checks do not establish speech completeness, accessibility, timing or text accuracy", "Color tags do not establish visual appearance or HDR mastering quality", "Passing does not establish upload acceptance or editorial approval"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentcut_core::OperationBatch;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn brief(value: Value) -> DeliveryBrief {
        serde_json::from_value(value).unwrap()
    }
    fn test_backend(temp: &TempDir, video: Value) -> FfmpegBackend {
        test_backend_with_format(
            temp,
            video,
            json!({"duration":"1.001000","start_time":"0.000000"}),
        )
    }
    fn test_backend_with_format(temp: &TempDir, video: Value, format: Value) -> FfmpegBackend {
        let path = temp.path().join("test-ffprobe");
        let streams = if video.is_array() {
            video
        } else {
            json!([video])
        };
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\n",
                json!({"streams":streams,"format":format})
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        FfmpegBackend::new(PathBuf::from("/bin/false"), path)
    }
    fn video() -> Value {
        json!({"codec_type":"video","width":360,"height":640,"sample_aspect_ratio":"1:1",
          "start_pts":0,"duration_ts":1001,"time_base":"1/1000","color_primaries":"bt709",
          "color_transfer":"bt709","color_space":"bt709","disposition":{"attached_pic":0},"tags":{}})
    }
    fn fixture(temp: &TempDir, with_caption: bool) -> (PathBuf, PathBuf, String) {
        let root = temp.path().join("project");
        let mut store = Store::create(&root, "Test").unwrap();
        if with_caption {
            let project = store.project().unwrap();
            let batch: OperationBatch = serde_json::from_value(json!({
                "schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":0,
                "idempotencyKey":"caption-check", "operations":[
                  {"id":"track", "op":"track.add", "params":{"id":"captions", "sequence":"seq_main", "type":"caption"}},
                  {"id":"cue", "op":"caption.add", "params":{"id":"cue", "track":"captions", "at":{"value":0,"rate":{"numerator":30,"denominator":1}}, "duration":{"value":30,"rate":{"numerator":30,"denominator":1}}, "text":"Hello from a caption"}}
                ]})).unwrap();
            store.apply(&batch, false).unwrap();
        }
        let snapshot = store.project().unwrap();
        let job = store
            .enqueue(
                "test-render",
                &crate::jobs::RenderInput {
                    sequence: "seq_main".into(),
                    priority: 0,
                    expected_revision: snapshot.revision,
                    ffmpeg: "ffmpeg".into(),
                    ffprobe: "ffprobe".into(),
                    asr: None,
                    analysis: None,
                },
            )
            .unwrap();
        let artifact = format!("{}-1", job.id);
        let sequence = snapshot.require_sequence("seq_main").unwrap();
        let (srt, vtt, captions) = crate::delivery::sidecars(sequence).unwrap();
        let dir = root.join("renders").join(&artifact);
        std::fs::create_dir_all(&dir).unwrap();
        let master = dir.join("video.mp4");
        std::fs::write(&master, b"fake video bytes").unwrap();
        let mut files = Vec::new();
        for (name, data) in [("captions.srt", srt), ("captions.vtt", vtt)] {
            let file = dir.join(name);
            std::fs::write(&file, data).unwrap();
            files.push(json!({"path":name,"sha256":media::hash_file(&file).unwrap()}));
        }
        let manifest_path = dir.join("manifest.json");
        let manifest = json!({"artifactId":artifact,"projectId":snapshot.project_id,
            "revision":snapshot.revision,"sequenceId":"seq_main","output":"video.mp4","snapshot":snapshot,
            "verification":{"sha256":media::hash_file(&master).unwrap(),"probe":{"streams":[{"width":9999}]}},
            "delivery":{"files":files,"captions":captions}});
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let result = json!({"artifactId":artifact,"revision":snapshot.revision,"manifest":manifest_path,"path":master});
        store
            .conn
            .execute(
                "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&result).unwrap(), job.id],
            )
            .unwrap();
        (root, manifest_path, job.id)
    }
    fn result_for<'a>(report: &'a Value, name: &str) -> &'a Value {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["constraint"] == name)
            .unwrap()
    }

    #[test]
    fn brief_validation_precedes_io_and_rejects_invalid_constraints() {
        let temp = TempDir::new().unwrap();
        let backend = test_backend(&temp, video());
        for value in [
            json!({"schemaVersion":2,"maxBytes":1}),
            json!({"schemaVersion":1}),
            json!({"schemaVersion":1,"maxBytes":0}),
            json!({"schemaVersion":1,"aspectRatio":{"width":0,"height":2}}),
            json!({"schemaVersion":1,"requiredSidecars":["srt","srt"]}),
        ] {
            let error = check(
                Path::new("/no/such/project"),
                "missing",
                &brief(value),
                &backend,
            )
            .unwrap_err();
            assert!(matches!(error, Error::Invalid(_)), "{error}");
        }
        assert!(
            serde_json::from_value::<DeliveryBrief>(
                json!({"schemaVersion":1,"maxBytes":10,"typo":true})
            )
            .is_err()
        );
    }

    #[test]
    fn checks_fresh_probe_actual_bytes_and_frozen_caption_content() {
        let temp = TempDir::new().unwrap();
        let (root, _, job) = fixture(&temp, true);
        let backend = test_backend(&temp, video());
        let request = brief(
            json!({"schemaVersion":1,"dimensions":{"width":360,"height":640},
          "aspectRatio":{"width":9,"height":16},"maxDurationMs":1001,"maxBytes":16,
          "requiredSidecars":["srt","vtt"],"color":"sdr"}),
        );
        let report = check(&root, &job, &request, &backend).unwrap();
        assert_eq!(report["status"], "pass");
        assert_eq!(result_for(&report, "dimensions")["actual"]["width"], 360);
        assert_eq!(result_for(&report, "maxBytes")["actual"], 16);
        assert_eq!(
            result_for(&report, "requiredSidecar")["actual"]["cueCount"],
            1
        );
        assert_eq!(report["artifact"]["revision"], 1);
        let too_short = brief(json!({"schemaVersion":1,"maxDurationMs":1000}));
        assert_eq!(
            check(&root, &job, &too_short, &backend).unwrap()["status"],
            "fail"
        );
    }

    #[test]
    fn unavailable_probe_geometry_and_color_are_unknown_not_pass() {
        let temp = TempDir::new().unwrap();
        let (root, _, job) = fixture(&temp, true);
        let mut metadata = video();
        metadata["side_data_list"] = json!([{"side_data_type":"Display Matrix","rotation":90}]);
        metadata["color_primaries"] = json!("unknown");
        let request = brief(
            json!({"schemaVersion":1,"dimensions":{"width":360,"height":640},
          "aspectRatio":{"width":9,"height":16},"color":"sdr"}),
        );
        let report = check(&root, &job, &request, &test_backend(&temp, metadata)).unwrap();
        assert_eq!(report["status"], "unknown");
        assert!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["status"] == "unknown")
        );
        let mut metadata = video();
        metadata["sample_aspect_ratio"] = Value::Null;
        let report = check(&root, &job, &request, &test_backend(&temp, metadata)).unwrap();
        assert_eq!(result_for(&report, "aspectRatio")["status"], "unknown");
        let unavailable = FfmpegBackend::new(
            PathBuf::from("/bin/false"),
            PathBuf::from("/no/such/ffprobe"),
        );
        let report = check(&root, &job, &request, &unavailable).unwrap();
        assert_eq!(report["status"], "unknown");
    }

    #[test]
    fn duration_checks_container_and_audio_end_with_conservative_unknowns() {
        let temp = TempDir::new().unwrap();
        let (root, _, job) = fixture(&temp, true);
        let request = brief(json!({"schemaVersion":1,"maxDurationMs":1001}));
        let valid_format = json!({"duration":"1.001000","start_time":"0.000000"});
        let audio = json!({"codec_type":"audio","start_pts":500,"duration_ts":800,"time_base":"1/1000","disposition":{"attached_pic":0}});
        let backend =
            test_backend_with_format(&temp, json!([video(), audio.clone()]), valid_format.clone());
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );
        let mut missing_start = audio.clone();
        missing_start.as_object_mut().unwrap().remove("start_pts");
        let backend = test_backend_with_format(
            &temp,
            json!([video(), missing_start.clone()]),
            valid_format.clone(),
        );
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "unknown"
        );
        missing_start["duration_ts"] = json!(1200);
        let backend =
            test_backend_with_format(&temp, json!([video(), missing_start]), valid_format.clone());
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );
        for missing in ["NaN", "-1.000000", "0.000000", "1e3"] {
            let backend = test_backend_with_format(
                &temp,
                video(),
                json!({"duration":missing,"start_time":"0.000000"}),
            );
            assert_eq!(
                check(&root, &job, &request, &backend).unwrap()["status"],
                "unknown"
            );
        }
        let backend = test_backend_with_format(&temp, video(), json!({"duration":"1.002000"}));
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );
        let mut fractional = video();
        fractional["duration_ts"] = json!(15015);
        fractional["time_base"] = json!("1/10000");
        let backend = test_backend_with_format(
            &temp,
            fractional,
            json!({"duration":"1.501500","start_time":"0.000000"}),
        );
        let fail = brief(json!({"schemaVersion":1,"maxDurationMs":1501}));
        let pass = brief(json!({"schemaVersion":1,"maxDurationMs":1502}));
        assert_eq!(
            check(&root, &job, &fail, &backend).unwrap()["status"],
            "fail"
        );
        assert_eq!(
            check(&root, &job, &pass, &backend).unwrap()["status"],
            "pass"
        );
    }

    #[test]
    fn stream_before_container_origin_cannot_cause_false_duration_pass() {
        let temp = TempDir::new().unwrap();
        let (root, _, job) = fixture(&temp, true);
        let request = brief(json!({"schemaVersion":1,"maxDurationMs":4000}));
        let mut picture = video();
        picture["duration_ts"] = json!(4000);
        let audio =
            json!({"codec_type":"audio","start_pts":-1000,"duration_ts":3000,"time_base":"1/1000"});
        let format = json!({"duration":"4.000000","start_time":"0.000000"});
        let backend = test_backend_with_format(
            &temp,
            json!([picture.clone(), audio.clone()]),
            format.clone(),
        );
        let report = check(&root, &job, &request, &backend).unwrap();
        assert_eq!(report["status"], "unknown");
        assert_eq!(
            report["checks"][0]["actual"]["streams"][1]["startsAtOrAfterContainer"],
            false
        );

        let mut longer = audio.clone();
        longer["duration_ts"] = json!(5000);
        let backend = test_backend_with_format(&temp, json!([picture.clone(), longer]), format);
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );

        picture["start_pts"] = json!(-1000);
        let backend = test_backend_with_format(
            &temp,
            json!([picture, audio]),
            json!({"duration":"4.000000","start_time":"-1.000000"}),
        );
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "pass"
        );
    }

    #[test]
    fn missing_empty_corrupt_duplicate_and_legacy_sidecars_never_pass() {
        let temp = TempDir::new().unwrap();
        let (root, path, job) = fixture(&temp, true);
        let backend = test_backend(&temp, video());
        let request = brief(json!({"schemaVersion":1,"requiredSidecars":["srt"]}));
        let srt = path.parent().unwrap().join("captions.srt");
        let original = std::fs::read(&srt).unwrap();
        std::fs::remove_file(&srt).unwrap();
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );
        std::fs::write(&srt, b"corrupted").unwrap();
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "fail"
        );
        std::fs::write(&srt, &original).unwrap();
        let original_manifest: Value = crate::json::read(&path).unwrap();
        let mut modified = original_manifest.clone();
        let entry = modified["delivery"]["files"][0].clone();
        modified["delivery"]["files"]
            .as_array_mut()
            .unwrap()
            .push(entry);
        std::fs::write(&path, serde_json::to_vec(&modified).unwrap()).unwrap();
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "unknown"
        );
        modified = original_manifest;
        std::fs::write(&srt, b"different older sidecar bytes").unwrap();
        modified["delivery"]["files"][0]["sha256"] = json!(media::hash_file(&srt).unwrap());
        std::fs::write(&path, serde_json::to_vec(&modified).unwrap()).unwrap();
        assert_eq!(
            check(&root, &job, &request, &backend).unwrap()["status"],
            "unknown"
        );
        let empty_temp = TempDir::new().unwrap();
        let (empty_root, _, empty_job) = fixture(&empty_temp, false);
        assert_eq!(
            check(&empty_root, &empty_job, &request, &backend).unwrap()["status"],
            "fail"
        );
    }

    #[test]
    fn refuses_changed_master_or_mismatched_frozen_revision() {
        let temp = TempDir::new().unwrap();
        let (root, manifest_path, job) = fixture(&temp, true);
        let backend = test_backend(&temp, video());
        let request = brief(json!({"schemaVersion":1,"maxBytes":100}));
        let master = manifest_path.parent().unwrap().join("video.mp4");
        std::fs::write(&master, b"changed bytes").unwrap();
        assert!(check(&root, &job, &request, &backend).is_err());
        std::fs::write(&master, b"fake video bytes").unwrap();
        let mut manifest: Value = crate::json::read(&manifest_path).unwrap();
        manifest["snapshot"]["name"] = json!("tampered project");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(check(&root, &job, &request, &backend).is_err());
    }

    #[test]
    fn synchronous_renders_are_refused_without_a_frozen_job_input() {
        let temp = TempDir::new().unwrap();
        let (root, manifest_path, _) = fixture(&temp, true);
        let mut manifest: Value = crate::json::read(&manifest_path).unwrap();
        let store = Store::open(&root).unwrap();
        let revision = store.project().unwrap().revision;
        let directory = root.join("renders/synchronous");
        std::fs::create_dir(&directory).unwrap();
        let master = directory.join("video.mp4");
        std::fs::write(&master, b"fake video bytes").unwrap();
        let sync_manifest = directory.join("manifest.json");
        manifest["artifactId"] = json!("synchronous");
        std::fs::write(&sync_manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
        store.start_job("synchronous", revision).unwrap();
        store.finish_job("synchronous", &json!({"artifactId":"synchronous","revision":revision,"path":master,"manifest":sync_manifest})).unwrap();
        let error = check(
            &root,
            "synchronous",
            &brief(json!({"schemaVersion":1,"maxBytes":100})),
            &test_backend(&temp, video()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("render-start"));
    }
}
