//! Creator-selected thumbnail candidates from a frozen, verified review video.
use crate::{
    Error, Result, compare, delivery, media,
    process::{self, Uncontrolled},
    store::Store,
};
use agentcut_core::{RationalRate, RationalTime, RoundingMode};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const MAX_CANDIDATES: usize = 8;
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThumbnailRequest {
    pub schema_version: u32,
    pub at_ms: Vec<i64>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

impl ThumbnailRequest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("thumbnail request schemaVersion must be 1"));
        }
        if self.at_ms.is_empty() || self.at_ms.len() > MAX_CANDIDATES {
            return Err(invalid("thumbnail request requires 1..8 timestamps"));
        }
        if self.at_ms.iter().any(|ms| *ms < 0) {
            return Err(invalid(
                "thumbnail timestamps must be nonnegative output milliseconds",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Frame {
    index: i64,
    request_index: usize,
    at_ms: i64,
}

/// Parse exact decoded frame PTS rather than relying on a rounded `pts_time`.
/// Missing, truncated or malformed logs cannot establish frame timing.
fn showinfo_timing(stderr: &str) -> Result<(RationalRate, i64)> {
    let mut clock = None;
    let mut points = Vec::new();
    for line in stderr
        .lines()
        .filter(|line| line.contains("[Parsed_showinfo_"))
    {
        if let Some((_, rest)) = line.split_once("config in time_base: ") {
            if clock.is_some() {
                return Err(invalid(
                    "thumbnail frame timing has multiple decoder clocks",
                ));
            }
            let fraction = rest
                .split(',')
                .next()
                .ok_or_else(|| invalid("thumbnail frame clock missing"))?;
            let (numerator, denominator) = fraction
                .split_once('/')
                .ok_or_else(|| invalid("thumbnail frame clock is invalid"))?;
            let numerator = numerator
                .trim()
                .parse::<u32>()
                .map_err(|_| invalid("thumbnail frame clock is invalid"))?;
            let denominator = denominator
                .trim()
                .parse::<u32>()
                .map_err(|_| invalid("thumbnail frame clock is invalid"))?;
            clock = Some(RationalRate::new(denominator, numerator)?);
        }
        if line.contains("] n:") {
            let (_, rest) = line
                .split_once(" pts:")
                .ok_or_else(|| invalid("thumbnail decoded frame timestamp missing"))?;
            let pts = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| invalid("thumbnail decoded frame timestamp missing"))?
                .parse::<i64>()
                .map_err(|_| invalid("thumbnail decoded frame timestamp is invalid"))?;
            points.push(pts);
        }
    }
    let clock = clock.ok_or_else(|| invalid("thumbnail decoded frame clock is unavailable"))?;
    let first = points
        .first()
        .copied()
        .ok_or_else(|| invalid("thumbnail decoded frame timestamp is unavailable"))?;
    Ok((clock, first))
}

fn seek_before(frame: i64, rate: RationalRate) -> Result<String> {
    let micros = i128::from(frame)
        .checked_mul(i128::from(rate.denominator))
        .and_then(|value| value.checked_mul(1_000_000))
        .ok_or_else(|| invalid("thumbnail frame time is too large"))?
        / i128::from(rate.numerator);
    // Seeking one microsecond before the nominal frame start prevents decimal
    // and FFmpeg time-base rounding from skipping the selected frame. We verify
    // its exact decoded PTS afterward instead of assuming the seek succeeded.
    let micros = micros.saturating_sub(1).max(0);
    Ok(format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000))
}

fn image_metadata(path: &Path, backend: &FfmpegBackend) -> Result<Value> {
    let bytes = fs::metadata(path)?.len();
    if bytes == 0 || bytes > MAX_IMAGE_BYTES {
        return Err(invalid(
            "thumbnail PNG exceeds the 16 MiB output limit or is empty",
        ));
    }
    let probe = process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "json",
            ])
            .arg(path),
        Duration::from_secs(30),
        &mut Uncontrolled,
    )?;
    let probe: Value = serde_json::from_slice(&probe.stdout)?;
    let streams = probe["streams"]
        .as_array()
        .filter(|streams| streams.len() == 1)
        .ok_or_else(|| invalid("thumbnail PNG probe is ambiguous"))?;
    let stream = &streams[0];
    let width = stream["width"]
        .as_u64()
        .filter(|v| (1..=1280).contains(v))
        .ok_or_else(|| invalid("thumbnail PNG width is unavailable or oversized"))?;
    let height = stream["height"]
        .as_u64()
        .filter(|v| (1..=1280).contains(v))
        .ok_or_else(|| invalid("thumbnail PNG height is unavailable or oversized"))?;
    if stream["codec_name"] != "png" {
        return Err(invalid("thumbnail output is not PNG"));
    }
    process::run(
        Command::new(backend.ffmpeg_path())
            .args([
                "-v",
                "error",
                "-xerror",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-i",
            ])
            .arg(path)
            .args(["-f", "null", "-"]),
        Duration::from_secs(30),
        &mut Uncontrolled,
    )?;
    Ok(
        json!({"bytes":bytes,"width":width,"height":height,"sha256":media::hash_file(path)?,"mimeType":"image/png"}),
    )
}

fn verified_sdr_review(manifest: &Value, role: &str) -> Result<()> {
    let verification = match role {
        "master" => &manifest["verification"],
        "sdr-review-preview" => {
            let files = manifest["delivery"]["files"]
                .as_array()
                .ok_or_else(|| invalid("SDR review video metadata is unavailable"))?;
            let mut matching = files.iter().filter(|file| file["path"] == "review-sdr.mp4");
            let file = matching
                .next()
                .ok_or_else(|| invalid("SDR review video metadata is unavailable"))?;
            if matching.next().is_some() {
                return Err(invalid("SDR review video metadata is ambiguous"));
            }
            &file["verification"]
        }
        _ => return Err(invalid("unsupported review video role")),
    };
    let streams = verification["probe"]["streams"]
        .as_array()
        .ok_or_else(|| invalid("SDR review video color metadata is unavailable"))?;
    let mut videos = streams
        .iter()
        .filter(|stream| stream["codec_type"] == "video");
    let video = videos
        .next()
        .ok_or_else(|| invalid("SDR review video color metadata is unavailable"))?;
    if videos.next().is_some()
        || video["color_primaries"] != "bt709"
        || video["color_transfer"] != "bt709"
        || video["color_space"] != "bt709"
        || video["color_range"] != "tv"
    {
        return Err(invalid("verified Rec.709 SDR review video is unavailable"));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn publish_no_replace(parent: &File, staging: &Path, destination: &Path) -> Result<()> {
    use nix::fcntl::{RenameFlags, renameat2};
    renameat2(
        parent,
        staging,
        parent,
        destination,
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| Error::Io(std::io::Error::from_raw_os_error(error as i32)))?;
    Ok(())
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn publish_no_replace(_parent: &File, _staging: &Path, _destination: &Path) -> Result<()> {
    Err(invalid("atomic thumbnail publication requires Linux GNU"))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Create hash-bound PNG candidates from selected times of a queued render.
/// Rendering and the creator's editorial selection remain separate decisions.
pub fn create(
    root: &Path,
    job_id: &str,
    destination: &Path,
    request: &ThumbnailRequest,
    backend: &FfmpegBackend,
) -> Result<Value> {
    request.validate()?;
    let root = root.canonicalize()?;
    let store = Store::open(&root)?;
    let frozen = compare::artifact(&store, &root, job_id)?;
    if frozen.identity["sequenceBinding"] != "job-input" {
        return Err(invalid(
            "thumbnail selection requires a queued render; re-render with render-start",
        ));
    }
    let sequence = frozen.snapshot.require_sequence(&frozen.sequence_id)?;
    let duration = delivery::review_duration(sequence, &frozen.manifest)
        .map_err(|_| invalid("verified review-video duration is unavailable or inconsistent"))?;
    let master = PathBuf::from(
        frozen.identity["master"]["path"]
            .as_str()
            .ok_or_else(|| invalid("verified master path missing"))?,
    );
    let master_sha = frozen.identity["master"]["sha256"]
        .as_str()
        .ok_or_else(|| invalid("verified master hash missing"))?;
    let review = PathBuf::from(
        frozen.identity["review"]["path"]
            .as_str()
            .ok_or_else(|| invalid("verified review video path missing"))?,
    );
    let review_sha = frozen.identity["review"]["sha256"]
        .as_str()
        .ok_or_else(|| invalid("verified review video hash missing"))?;
    let role = frozen.identity["review"]["role"]
        .as_str()
        .ok_or_else(|| invalid("verified review video role missing"))?;
    verified_sdr_review(&frozen.manifest, role)?;
    let millis = RationalRate::frames(1000)?;
    let mut selected = Vec::with_capacity(request.at_ms.len());
    let mut unique = BTreeSet::new();
    for (request_index, at_ms) in request.at_ms.iter().copied().enumerate() {
        let at = RationalTime::new(at_ms, millis);
        if at.checked_cmp(duration)?.is_ge() {
            return Err(invalid(format!(
                "thumbnail timestamp {at_ms}ms is outside the frozen review video"
            )));
        }
        let index = at
            .rescale_to(sequence.frame_rate, RoundingMode::Floor)?
            .value;
        if !unique.insert(index) {
            return Err(invalid(
                "thumbnail timestamps must select distinct output frames",
            ));
        }
        selected.push(Frame {
            index,
            request_index,
            at_ms,
        });
    }
    selected.sort_by_key(|frame| frame.index);
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let target = destination
        .file_name()
        .ok_or_else(|| invalid("thumbnail destination has no filename"))?;
    match fs::symlink_metadata(parent.join(target)) {
        Ok(_) => return Err(invalid("thumbnail destination already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent_file = File::open(&parent)?;
    let stage_name = format!(".avw-thumbnails-{}.incomplete", uuid::Uuid::new_v4());
    let stage = parent.join(&stage_name);
    fs::create_dir(&stage)?;
    let result = (|| -> Result<Value> {
        write_new(
            &stage.join("thumbnails.incomplete"),
            b"Thumbnail candidates are incomplete",
        )?;
        let mut candidates = vec![Value::Null; selected.len()];
        for (position, frame) in selected.iter().enumerate() {
            let name = format!("candidate-{:02}.png", position + 1);
            let path = stage.join(&name);
            let seek = seek_before(frame.index, sequence.frame_rate)?;
            let output = process::run(Command::new(backend.ffmpeg_path())
                .args(["-nostdin","-v","info","-xerror","-threads","1","-copyts","-ss",&seek,"-protocol_whitelist","file","-i"])
                .arg(&review).args(["-map","0:v:0","-an","-sn","-vf",
                    "showinfo,scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease",
                    "-fps_mode","passthrough","-frames:v","1","-update","1"])
                .arg(&path), Duration::from_secs(120), &mut Uncontrolled)?;
            let (clock, pts) = showinfo_timing(&output.stderr)?;
            let actual = RationalTime::new(pts, clock);
            let expected = RationalTime::new(frame.index, sequence.frame_rate);
            if !actual.same_instant(expected)? {
                return Err(invalid(format!(
                    "thumbnail frame {} timestamp differs from the frozen output clock",
                    frame.index
                )));
            }
            let image = image_metadata(&path, backend)?;
            File::open(&path)?.sync_all()?;
            candidates[frame.request_index] = json!({"requestedAtMs":frame.at_ms,"frameIndex":frame.index,
                "decodedTimestamp":actual,"path":name,"image":image});
        }
        if media::hash_file(&master)? != master_sha || media::hash_file(&review)? != review_sha {
            return Err(invalid(
                "rendered source bytes changed during thumbnail extraction",
            ));
        }
        let report = json!({"schemaVersion":1,"jobId":job_id,"artifactId":frozen.identity["artifactId"],
            "projectId":frozen.snapshot.project_id,"revision":frozen.snapshot.revision,"sequenceId":frozen.sequence_id,
            "source":{"role":role,"sha256":review_sha,"masterSha256":master_sha},
            "candidates":candidates,
            "limitations":["Candidates are generated from the review video and bounded to 1280 pixels per axis",
                "An SDR review preview may be scaled or tone-mapped; it does not establish master/HDR display appearance",
                "PNG color interpretation can vary by viewer and display",
                "Requested milliseconds select the containing frozen output frame; its decoded timestamp is reported exactly",
                "The render manifest is not independently authenticated; editorial suitability requires creator review"]});
        write_new(
            &stage.join("manifest.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        fs::remove_file(stage.join("thumbnails.incomplete"))?;
        File::open(&stage)?.sync_all()?;
        publish_no_replace(&parent_file, Path::new(&stage_name), Path::new(target))?;
        parent_file.sync_all()?;
        Ok(json!({"path":parent.join(target),"manifest":report}))
    })();
    if result.is_err() && stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn showinfo_requires_a_single_clock_and_complete_integer_timestamps() {
        let text = "[Parsed_showinfo_1 @ x] config in time_base: 1/30000, frame_rate: 30000/1001\n[Parsed_showinfo_1 @ x] n:   0 pts:   1001 pts_time:0.033367\n[Parsed_showinfo_1 @ x] n:   1 pts: 2002 pts_time:0.066733\n";
        let (clock, pts) = showinfo_timing(text).unwrap();
        assert_eq!(clock, RationalRate::frames(30000).unwrap());
        assert_eq!(pts, 1001);
        assert!(showinfo_timing("[Parsed_showinfo_1 @ x] n: 0 pts: N/A").is_err());
        assert!(showinfo_timing(&format!("{text}{text}")).is_err());
        assert_eq!(
            seek_before(15, RationalRate::new(30000, 1001).unwrap()).unwrap(),
            "0.500499"
        );
    }
}
