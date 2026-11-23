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
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const MAX_CANDIDATES: usize = 8;
const MAX_GROUP_GAP: i64 = 8;
const BUNDLE_EDGE: u32 = 1280;
const INLINE_EDGE: u32 = 768;
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
fn showinfo_timing(stderr: &str) -> Result<(RationalRate, Vec<i64>)> {
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
    Ok((clock, points))
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

fn png_metadata(bytes: &[u8], max_edge: u32) -> Result<Value> {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_checksums(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut input = Cursor::new(bytes);
    let mut decoder = png::Decoder::new_with_options(&mut input, options);
    decoder.set_limits(png::Limits {
        bytes: MAX_IMAGE_BYTES as usize,
    });
    let png_error = |error: png::DecodingError| invalid(format!("invalid thumbnail PNG: {error}"));
    let mut reader = decoder.read_info().map_err(png_error)?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if !(1..=max_edge).contains(&width)
        || !(1..=max_edge).contains(&height)
        || info.animation_control.is_some()
    {
        return Err(invalid(
            "expected a single PNG within the review image dimensions",
        ));
    }
    // Decode every row, then validate the remaining chunks through IEND.
    while reader.next_row().map_err(png_error)?.is_some() {}
    reader.finish().map_err(png_error)?;
    if input.position() != bytes.len() as u64 {
        return Err(invalid("trailing data after thumbnail PNG"));
    }
    Ok(json!({"bytes":bytes.len(),"width":width,"height":height,
        "sha256":format!("{:x}", Sha256::digest(bytes)),"mimeType":"image/png"}))
}

fn bundle_image(path: &Path) -> Result<Value> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    (&file).take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(invalid(
            "thumbnail PNG exceeds the 16 MiB output limit or is empty",
        ));
    }
    let image = png_metadata(&bytes, BUNDLE_EDGE)?;
    file.sync_all()?;
    Ok(image)
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

struct Review {
    video: PathBuf,
    master: PathBuf,
    frame_rate: RationalRate,
    duration: RationalTime,
    metadata: Map<String, Value>,
}

impl Review {
    fn open(root: &Path, job_id: &str) -> Result<Self> {
        let root = root.canonicalize()?;
        let store = Store::open_reader(&root)?;
        let frozen = compare::artifact(&store, &root, job_id)?;
        if frozen.identity["sequenceBinding"] != "job-input" {
            return Err(invalid(
                "thumbnail selection requires a queued render; re-render with render-start",
            ));
        }
        let sequence = frozen.snapshot.require_sequence(&frozen.sequence_id)?;
        let duration = delivery::review_duration(sequence, &frozen.manifest).map_err(|_| {
            invalid("verified review-video duration is unavailable or inconsistent")
        })?;
        let master = &frozen.identity["master"];
        let review = &frozen.identity["review"];
        let role = review["role"]
            .as_str()
            .ok_or_else(|| invalid("verified review video role missing"))?;
        verified_sdr_review(&frozen.manifest, role)?;
        Ok(Self {
            video: PathBuf::from(
                review["path"]
                    .as_str()
                    .ok_or_else(|| invalid("verified review video path missing"))?,
            ),
            master: PathBuf::from(
                master["path"]
                    .as_str()
                    .ok_or_else(|| invalid("verified master path missing"))?,
            ),
            frame_rate: sequence.frame_rate,
            duration,
            metadata: Map::from_iter([
                ("jobId".into(), json!(job_id)),
                ("artifactId".into(), frozen.identity["artifactId"].clone()),
                ("projectId".into(), json!(frozen.snapshot.project_id)),
                ("revision".into(), json!(frozen.snapshot.revision)),
                ("sequenceId".into(), json!(frozen.sequence_id)),
                (
                    "source".into(),
                    json!({"role":role,"sha256":review["sha256"],"masterSha256":master["sha256"]}),
                ),
            ]),
        })
    }

    fn frame(&self, at_ms: i64, request_index: usize) -> Result<Frame> {
        let at = RationalTime::new(at_ms, RationalRate::frames(1000)?);
        if at.checked_cmp(self.duration)?.is_ge() {
            return Err(invalid(format!(
                "thumbnail timestamp {at_ms}ms is outside the frozen review video"
            )));
        }
        let index = at.rescale_to(self.frame_rate, RoundingMode::Floor)?.value;
        Ok(Frame {
            index,
            request_index,
            at_ms,
        })
    }

    fn verify_unchanged(&self) -> Result<()> {
        let source = &self.metadata["source"];
        if source["sha256"] != media::hash_file(&self.video)?
            || (self.master != self.video
                && source["masterSha256"] != media::hash_file(&self.master)?)
        {
            return Err(invalid(
                "rendered source bytes changed during thumbnail extraction",
            ));
        }
        Ok(())
    }
}

enum PngOutput<'a> {
    Files { directory: &'a Path, start: usize },
    Inline,
}

// FFmpeg interprets percent signs anywhere in an image2 sequence path.
fn sequence_pattern(directory: &Path) -> PathBuf {
    let mut escaped = Vec::new();
    for &byte in directory.as_os_str().as_bytes() {
        if byte == b'%' {
            escaped.push(byte);
        }
        escaped.push(byte);
    }
    PathBuf::from(OsString::from_vec(escaped)).join("candidate-%02d.png")
}

fn extract_frames(
    review: &Review,
    frames: &[Frame],
    backend: &FfmpegBackend,
    output: PngOutput<'_>,
) -> Result<(Vec<RationalTime>, Vec<u8>)> {
    let first = frames[0].index;
    let offsets = frames
        .iter()
        .map(|frame| format!("eq(n,{})", frame.index - first))
        .collect::<Vec<_>>()
        .join("+");
    let end = frames[frames.len() - 1].index - first + 1;
    let edge = match output {
        PngOutput::Files { .. } => BUNDLE_EDGE,
        PngOutput::Inline => INLINE_EDGE,
    };
    // End the graph after the last requested frame even if the encoder buffers frames.
    let filter = format!(
        "trim=end_frame={end},select='{offsets}',showinfo=checksum=0,scale=w='min({edge},iw)':h='min({edge},ih)':force_original_aspect_ratio=decrease"
    );
    let mut cmd = Command::new(backend.ffmpeg_path());
    cmd.args([
        "-nostdin", "-nostats", "-v", "info", "-xerror", "-threads", "1", "-copyts", "-ss",
    ])
    .arg(seek_before(first, review.frame_rate)?)
    .args(["-protocol_whitelist", "file", "-i"])
    .arg(&review.video)
    .args([
        "-map",
        "0:v:0",
        "-an",
        "-sn",
        "-vf",
        &filter,
        "-fps_mode",
        "passthrough",
        "-frames:v",
    ])
    .arg(frames.len().to_string())
    .args(["-c:v", "png"]);
    match output {
        PngOutput::Files { directory, start } => {
            cmd.args(["-f", "image2", "-start_number"])
                .arg(start.to_string())
                .arg(sequence_pattern(directory));
        }
        PngOutput::Inline => {
            cmd.args(["-pix_fmt", "rgb24", "-f", "image2pipe", "pipe:1"]);
        }
    }
    let output = process::run(&mut cmd, Duration::from_secs(120), &mut Uncontrolled)?;
    let (clock, points) = showinfo_timing(&output.stderr)?;
    if points.len() != frames.len() {
        return Err(invalid(
            "thumbnail decoded frame count differs from the selection",
        ));
    }
    let mut timestamps = Vec::with_capacity(frames.len());
    for (pts, frame) in points.into_iter().zip(frames) {
        let actual = RationalTime::new(pts, clock);
        if !actual.same_instant(RationalTime::new(frame.index, review.frame_rate))? {
            return Err(invalid(format!(
                "thumbnail frame {} timestamp differs from the frozen output clock",
                frame.index
            )));
        }
        timestamps.push(actual);
    }
    Ok((timestamps, output.stdout))
}

/// Return a bounded PNG of the edited output without creating a thumbnail bundle.
pub(crate) fn review_frame(
    root: &Path,
    job_id: &str,
    at_ms: i64,
    backend: &FfmpegBackend,
) -> Result<(Value, Vec<u8>)> {
    if at_ms < 0 {
        return Err(invalid(
            "review-frame requires nonnegative output milliseconds",
        ));
    }
    let review = Review::open(root, job_id)?;
    let frame = review.frame(at_ms, 0)?;
    let (times, bytes) = extract_frames(
        &review,
        std::slice::from_ref(&frame),
        backend,
        PngOutput::Inline,
    )?;
    if bytes.len() as u64 > crate::mcp::MAX_IMAGE_BYTES {
        return Err(invalid("image exceeds the 2 MiB MCP media limit"));
    }
    let mut metadata = png_metadata(&bytes, INLINE_EDGE)?;
    review.verify_unchanged()?;
    metadata["kind"] = json!("review-frame");
    metadata["requestedAtMs"] = json!(at_ms);
    metadata["frameIndex"] = json!(frame.index);
    metadata["decodedTimestamp"] = json!(times[0]);
    for (key, value) in review.metadata {
        metadata[key] = value;
    }
    Ok((metadata, bytes))
}

/// Create hash-bound PNG candidates from selected times of a queued render.
pub fn create(
    root: &Path,
    job_id: &str,
    destination: &Path,
    request: &ThumbnailRequest,
    backend: &FfmpegBackend,
) -> Result<Value> {
    request.validate()?;
    let review = Review::open(root, job_id)?;
    let mut selected = Vec::with_capacity(request.at_ms.len());
    let mut unique = BTreeSet::new();
    for (position, &at_ms) in request.at_ms.iter().enumerate() {
        let frame = review.frame(at_ms, position)?;
        if !unique.insert(frame.index) {
            return Err(invalid(
                "thumbnail timestamps must select distinct output frames",
            ));
        }
        selected.push(frame);
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
        let mut start = 1;
        // Nearby frames share a seek. Widely separated selections skip ahead in the input.
        for group in selected.chunk_by(|a, b| b.index - a.index <= MAX_GROUP_GAP) {
            let (times, _) = extract_frames(
                &review,
                group,
                backend,
                PngOutput::Files {
                    directory: &stage,
                    start,
                },
            )?;
            for (frame, actual) in group.iter().zip(times) {
                let name = format!("candidate-{start:02}.png");
                let image = bundle_image(&stage.join(&name))?;
                candidates[frame.request_index] = json!({"requestedAtMs":frame.at_ms,"frameIndex":frame.index,
                    "decodedTimestamp":actual,"path":name,"image":image});
                start += 1;
            }
        }
        review.verify_unchanged()?;
        let mut report = Value::Object(review.metadata);
        report["schemaVersion"] = json!(1);
        report["candidates"] = json!(candidates);
        report["limitations"] = json!([
            "Candidates are generated from the review video and bounded to 1280 pixels per axis",
            "An SDR review preview may be scaled or tone-mapped; it does not establish master/HDR display appearance",
            "PNG color interpretation can vary by viewer and display",
            "Requested milliseconds select the containing frozen output frame; its decoded timestamp is reported exactly",
            "The render manifest is not independently authenticated; editorial suitability requires creator review"
        ]);
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
    fn verifies_complete_png_pixels_checksums_dimensions_and_end() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 3, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[23; 18])
                .unwrap();
        }
        let image = png_metadata(&png, 3).unwrap();
        assert_eq!(image["width"], 3);
        assert_eq!(image["height"], 2);
        assert_eq!(image["bytes"], png.len());
        assert!(png_metadata(&png, 2).is_err());
        assert!(png_metadata(&png[..png.len() - 1], 3).is_err());
        assert!(png_metadata(&[png.as_slice(), &[0]].concat(), 3).is_err());
        let idat = png.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
        png[idat + 4] ^= 1;
        assert!(png_metadata(&png, 3).is_err());
    }

    #[test]
    fn showinfo_requires_a_single_clock_and_complete_integer_timestamps() {
        let text = "[Parsed_showinfo_1 @ x] config in time_base: 1/30000, frame_rate: 30000/1001\n[Parsed_showinfo_1 @ x] n:   0 pts:   1001 pts_time:0.033367\n[Parsed_showinfo_1 @ x] n:   1 pts: 2002 pts_time:0.066733\n";
        let (clock, pts) = showinfo_timing(text).unwrap();
        assert_eq!(clock, RationalRate::frames(30000).unwrap());
        assert_eq!(pts, [1001, 2002]);
        assert!(showinfo_timing("[Parsed_showinfo_1 @ x] n: 0 pts: N/A").is_err());
        assert!(showinfo_timing(&format!("{text}{text}")).is_err());
        assert_eq!(
            seek_before(15, RationalRate::new(30000, 1001).unwrap()).unwrap(),
            "0.500499"
        );
    }
}
