//! Cached source navigation and constrained review media. No semantic inference.
use crate::{
    Error, Result, color, media,
    process::{self, Control},
    workflow,
};
use agentcut_core::{Asset, Project, RationalRate, RationalTime, RoundingMode};
use agentcut_render::FfmpegBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command, time::Duration};

fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
fn step() -> u32 {
    10_000
}
fn page_size() -> u32 {
    12
}
fn tile_width() -> u32 {
    256
}
fn image_budget() -> u64 {
    1_048_576
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Storyboard {
    pub asset_id: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default = "step")]
    pub sample_every_ms: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "page_size")]
    pub limit: u32,
    #[serde(default = "tile_width")]
    pub tile_width: u32,
    #[serde(default = "image_budget")]
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetPreview {
    pub asset_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub max_duration_ms: u32,
    pub max_bytes: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
struct Clock {
    origin: RationalTime,
    base: (u32, u32),
    stream: u64,
}

fn decimal_time(s: &str) -> Result<RationalTime> {
    let (sign, s) = if let Some(s) = s.strip_prefix('-') {
        (-1_i64, s)
    } else {
        (1, s)
    };
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    if fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid source origin"));
    }
    let rate = 10_u32.pow(fraction.len() as u32);
    let value = whole
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(i64::from(rate)))
        .and_then(|n| {
            n.checked_add(if fraction.is_empty() {
                0
            } else {
                fraction.parse().ok()?
            })
        })
        .and_then(|n| n.checked_mul(sign))
        .ok_or_else(|| invalid("invalid source origin"))?;
    Ok(RationalTime::new(
        value,
        RationalRate {
            numerator: rate,
            denominator: 1,
        },
    ))
}
fn base(s: &str) -> Result<(u32, u32)> {
    let (n, d) = s
        .split_once('/')
        .ok_or_else(|| invalid("video time base unavailable"))?;
    let n = n
        .parse::<u32>()
        .map_err(|_| invalid("invalid video time base"))?;
    let d = d
        .parse::<u32>()
        .map_err(|_| invalid("invalid video time base"))?;
    if n == 0 || d == 0 {
        return Err(invalid("invalid video time base"));
    }
    Ok((n, d))
}
fn clock(asset: &Asset) -> Result<Clock> {
    let probe = asset
        .extensions
        .get(color::PROBE)
        .ok_or_else(|| invalid("imported source clock unavailable; reimport source"))?;
    let stream = probe["streams"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        })
        .ok_or_else(|| invalid("source has no video stream"))?;
    Ok(Clock {
        origin: decimal_time(
            probe["format"]["start_time"]
                .as_str()
                .ok_or_else(|| invalid("source origin unavailable"))?,
        )?,
        base: base(
            stream["time_base"]
                .as_str()
                .ok_or_else(|| invalid("source time base unavailable"))?,
        )?,
        stream: stream["index"]
            .as_u64()
            .ok_or_else(|| invalid("source video stream identity unavailable"))?,
    })
}
fn time(pts: i64, b: (u32, u32)) -> RationalTime {
    RationalTime::new(
        pts,
        RationalRate {
            numerator: b.1,
            denominator: b.0,
        },
    )
}
fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}
fn seconds(t: RationalTime) -> String {
    format!("{:.9}", t.as_seconds_f64())
}
fn duration(asset: &Asset) -> Result<i64> {
    Ok(asset
        .metadata
        .duration
        .ok_or_else(|| invalid("source duration unavailable"))?
        .rescale_to(workflow::time(0).rate, RoundingMode::Floor)?
        .value)
}

fn storyboard_end(q: &Storyboard, asset: &Asset) -> Result<(i64, Option<i64>)> {
    let c = clock(asset)?;
    let reported = asset
        .extensions
        .get(color::PROBE)
        .and_then(|p| p["streams"].as_array())
        .and_then(|a| a.iter().find(|s| s["index"].as_u64() == Some(c.stream)))
        .and_then(|s| {
            Some((
                decimal_time(s["start_time"].as_str()?).ok()?,
                decimal_time(s["duration"].as_str()?).ok()?,
            ))
        });
    let video_end = reported
        .map(|(start, len)| {
            start
                .checked_add(len)?
                .checked_sub(c.origin)?
                .rescale_to(workflow::time(0).rate, RoundingMode::Floor)
                .map(|t| t.value)
        })
        .transpose()?
        .filter(|v| *v > 0);
    let end = q.end_ms.unwrap_or(
        duration(asset)?
            .min(video_end.unwrap_or(i64::MAX))
            .min(86_400_000),
    );
    if video_end.is_some_and(|v| end > v) {
        return Err(invalid(
            "storyboard endMs exceeds the reported video end; the container may have an audio-only tail",
        ));
    }
    Ok((end, video_end))
}

pub fn validate_storyboard(q: &Storyboard, project: &Project) -> Result<()> {
    let asset = project.require_asset(&q.asset_id)?;
    clock(asset)?;
    color::source_filter(asset)?;
    let (end, _) = storyboard_end(q, asset)?;
    if q.start_ms < 0
        || end <= q.start_ms
        || end > duration(asset)?
        || end > 86_400_000
        || !(1000..=300_000).contains(&q.sample_every_ms)
        || !(1..=24).contains(&q.limit)
        || !(128..=512).contains(&q.tile_width)
        || !q.tile_width.is_multiple_of(16)
        || !(16_384..=2_097_152).contains(&q.max_bytes)
    {
        return Err(invalid(
            "storyboard needs an in-source scope up to 24h, sampleEveryMs 1000..300000, limit 1..24, tileWidth 128..512 divisible by 16, maxBytes 16384..2097152",
        ));
    }
    let total =
        (end - q.start_ms + i64::from(q.sample_every_ms) - 1) / i64::from(q.sample_every_ms);
    if i64::from(q.offset) >= total {
        return Err(invalid("storyboard offset is outside the sample index"));
    }
    Ok(())
}
pub fn validate_preview(q: &BudgetPreview, project: &Project) -> Result<()> {
    let asset = project.require_asset(&q.asset_id)?;
    clock(asset)?;
    color::source_filter(asset)?;
    if q.start_ms < 0
        || q.end_ms <= q.start_ms
        || q.end_ms > duration(asset)?
        || q.max_duration_ms == 0
        || q.max_duration_ms > 60_000
        || q.end_ms - q.start_ms > i64::from(q.max_duration_ms)
        || !(16_384..=67_108_864).contains(&q.max_bytes)
        || !(64..=1280).contains(&q.width)
        || !(64..=1280).contains(&q.height)
        || !q.width.is_multiple_of(2)
        || !q.height.is_multiple_of(2)
    {
        return Err(invalid(
            "budget preview needs an in-source interval, maxDurationMs 1..60000 covering that interval, maxBytes 16384..67108864, even width/height 64..1280",
        ));
    }
    Ok(())
}

fn observed_frame(log: &str, sample: i64) -> Result<(i64, (u32, u32))> {
    let tag = format!("[showinfo@sample{sample} @");
    let lines = log
        .lines()
        .filter(|line| line.starts_with(&tag))
        .collect::<Vec<_>>();
    let bases = lines
        .iter()
        .filter_map(|line| {
            line.split_once("config in time_base:")
                .map(|(_, s)| s.split(',').next().unwrap_or("").trim())
        })
        .collect::<Vec<_>>();
    let [b] = bases.as_slice() else {
        return Err(invalid(
            "decoded sample must have one unambiguous time base",
        ));
    };
    let pts = lines
        .iter()
        .filter(|l| l.contains("pts_time:"))
        .map(|line| {
            line.split_once(" pts:")
                .and_then(|(_, s)| s.split_whitespace().next()?.parse::<i64>().ok())
        })
        .collect::<Vec<_>>();
    match pts.as_slice() {
        [Some(pts)] => Ok((*pts, base(b)?)),
        _ => Err(invalid(
            "decoded sample must have exactly one observed timestamp",
        )),
    }
}

pub fn storyboard(
    q: &Storyboard,
    project: &Project,
    asset: &Asset,
    source: &Path,
    scratch: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<(Value, Vec<Value>)> {
    let clock = clock(asset)?;
    let (end, reported_video_end) = storyboard_end(q, asset)?;
    let total =
        (end - q.start_ms + i64::from(q.sample_every_ms) - 1) / i64::from(q.sample_every_ms);
    let last = (i64::from(q.offset) + i64::from(q.limit)).min(total);
    let count = last - i64::from(q.offset);
    let columns = count.min(4);
    let height = q.tile_width * 9 / 16;
    let mut tiles = Vec::new();
    let mut paths = Vec::new();
    let color = color::source_filter(asset)?;
    let color = if color.is_empty() {
        color
    } else {
        format!("{color},")
    };
    // Four accurately sought inputs per row bound concurrent decoder memory.
    // Trim before timestamp reset, then compose raw RGB cells before PNG
    // encoding. This avoids one process and PNG round trip per source sample.
    // Larger coded frames retain sequential extraction to avoid multiplying
    // their decoder buffers. Composition still uses the same page grid.
    let batch_size = if asset
        .metadata
        .video
        .as_ref()
        .is_some_and(|v| u64::from(v.size.width) * u64::from(v.size.height) > 1920 * 1080)
    {
        1
    } else {
        4
    };
    for row_start in (i64::from(q.offset)..last).step_by(batch_size) {
        control.check()?;
        let row_end = (row_start + batch_size as i64).min(last);
        let path = scratch.join(format!("row-{}.png", paths.len()));
        let mut cmd = Command::new(backend.ffmpeg_path());
        cmd.args([
            "-nostdin",
            "-v",
            "info",
            "-copyts",
            "-filter_complex_threads",
            "1",
        ]);
        let mut chains = Vec::new();
        for (input, index) in (row_start..row_end).enumerate() {
            let requested = q.start_ms + index * i64::from(q.sample_every_ms);
            cmd.args([
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-ss",
                &seconds(workflow::time(requested)),
                "-noautorotate",
                "-i",
            ])
            .arg(source);
            chains.push(format!(
                "[{input}:{}]trim=end_frame=1,showinfo@sample{index}=checksum=0,{color}scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=rgb24,setpts=PTS-STARTPTS[t{input}]",
                clock.stream, q.tile_width, height, q.tile_width, height
            ));
        }
        let row_count = row_end - row_start;
        let inputs = (0..row_count)
            .map(|i| format!("[t{i}]"))
            .collect::<String>();
        let stack = if row_count == 1 {
            String::new()
        } else {
            let layout = (0..row_count)
                .map(|i| format!("{}_0", i * i64::from(q.tile_width)))
                .collect::<Vec<_>>()
                .join("|");
            format!("xstack=inputs={row_count}:layout={layout}:fill=black,")
        };
        let row_width = if batch_size == 1 { 1 } else { columns } * i64::from(q.tile_width);
        chains.push(format!(
            "{inputs}{stack}pad={}:{}:0:0:color=black[row]",
            row_width, height
        ));
        cmd.args([
            "-filter_complex",
            &chains.join(";"),
            "-map",
            "[row]",
            "-frames:v",
            "1",
            "-fps_mode",
            "passthrough",
            "-map_metadata",
            "-1",
            "-threads",
            "1",
        ])
        .arg(&path);
        let output = process::run(&mut cmd, Duration::from_secs(60), control)?;
        for index in row_start..row_end {
            let requested = q.start_ms + index * i64::from(q.sample_every_ms);
            let (pts, b) = observed_frame(&output.stderr, index)?;
            let actual = time(pts, b).checked_sub(clock.origin)?;
            if actual.checked_cmp(workflow::time(requested))?.is_lt()
                || actual.checked_cmp(workflow::time(end))?.is_ge()
            {
                return Err(invalid(
                    "decoded sample is outside its requested source window; inspect a narrower or earlier scope",
                ));
            }
            let tile = tiles.len() as i64;
            tiles.push(json!({"index":index,"requestedSourceTimeMs":requested,"observedPts":pts,"timeBase":format!("{}/{}",b.0,b.1),"sourceTime":actual,"sourceTimeMs":actual.rescale_to(workflow::time(0).rate,RoundingMode::Nearest)?.value,"column":tile%columns,"row":tile/columns}));
        }
        paths.push(path);
    }
    let sheet = scratch.join("storyboard.png");
    if paths.len() == 1 {
        std::fs::copy(&paths[0], &sheet)?;
    } else {
        let mut cmd = Command::new(backend.ffmpeg_path());
        cmd.args(["-nostdin", "-v", "error", "-filter_complex_threads", "1"]);
        for path in &paths {
            cmd.args(["-threads", "1", "-i"]).arg(path);
        }
        let inputs = (0..paths.len())
            .map(|i| format!("[{i}:v]"))
            .collect::<String>();
        let layout = (0..paths.len())
            .map(|i| {
                let tile = i as i64 * batch_size as i64;
                format!(
                    "{}_{}",
                    tile % columns * i64::from(q.tile_width),
                    tile / columns * i64::from(height)
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        cmd.args([
            "-filter_complex",
            &format!(
                "{inputs}xstack=inputs={}:layout={layout}:fill=black",
                paths.len()
            ),
            "-frames:v",
            "1",
            "-threads",
            "1",
        ])
        .arg(&sheet);
        process::run(&mut cmd, Duration::from_secs(60), control)?;
    }
    for path in paths {
        std::fs::remove_file(path)?;
    }
    let bytes = std::fs::metadata(&sheet)?.len();
    if bytes > q.max_bytes {
        return Err(invalid(format!(
            "storyboard is {bytes} bytes, exceeding maxBytes {}; reduce limit or tileWidth",
            q.max_bytes
        )));
    }
    let all_transcripts = workflow::transcripts(project)?;
    let active = all_transcripts
        .iter()
        .filter(|t| t.asset_id == q.asset_id)
        .collect::<Vec<_>>();
    let transcript = if active.len() == 1 {
        let t = active[0];
        let page_start = q.start_ms + i64::from(q.offset) * i64::from(q.sample_every_ms);
        let page_end = (q.start_ms + last * i64::from(q.sample_every_ms)).min(end);
        let mut cues = t
            .cues
            .iter()
            .filter(|c| c.start_ms < page_end && c.end_ms > page_start)
            .collect::<Vec<_>>();
        cues.sort_by(|a, b| (a.start_ms, a.end_ms, &a.id).cmp(&(b.start_ms, b.end_ms, &b.id)));
        let excerpts=cues.iter().take(20).map(|c|json!({"id":c.id,"sourceStartMs":c.start_ms,"sourceEndMs":c.end_ms,"text":c.text.chars().take(512).collect::<String>(),"textTruncated":c.text.chars().count()>512})).collect::<Vec<_>>();
        json!({"status":"available","sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(t)?)),"language":t.language,"provider":t.provider,"cues":excerpts,"cuesTotal":cues.len(),"cuesTruncated":cues.len()>20,"guidance":"Transcript text requires review and does not describe all visual or audio content."})
    } else {
        json!({"status":"unavailable","reason":if active.is_empty(){"no-active-transcript"}else{"multiple-active-transcripts"}})
    };
    let files = vec![
        json!({"name":"storyboard.png","sha256":media::hash_file_controlled(&sheet,control)?}),
    ];
    Ok((
        json!({"file":"storyboard.png","mimeType":"image/png","bytes":bytes,"tileWidth":q.tile_width,"tileHeight":height,"columns":columns,"tiles":tiles,"transcript":transcript,"scopeStartMs":q.start_ms,"scopeEndMs":end,"pageStartMs":q.start_ms+i64::from(q.offset)*i64::from(q.sample_every_ms),"pageEndMs":(q.start_ms+last*i64::from(q.sample_every_ms)).min(end),"sourceDurationMsFloor":duration(asset)?,"reportedVideoEndMsFloor":reported_video_end,"totalSamples":total,"offset":q.offset,"nextOffset":if last<total {Some(last)}else{None},"sampleEveryMs":q.sample_every_ms,"containerOrigin":clock.origin,"coverage":{"kind":"sparse-observed-frames","continuousVideoInspected":false,"scopeContinues":end<duration(asset)?,"audioOnlyTailMayRemain":reported_video_end.is_some_and(|v|v<duration(asset).unwrap_or(v)),"guidance":"Sparse samples can miss brief events, small text, motion and speech. Request denser pages, full-resolution frames or a bounded preview; empty samples or unavailable transcripts do not establish absence."}}),
        files,
    ))
}

fn frame_probe(
    path: &Path,
    backend: &FfmpegBackend,
    stream: u64,
    interval: Option<(RationalTime, RationalTime)>,
    include_frames: bool,
    control: &mut dyn Control,
) -> Result<Value> {
    let mut cmd = Command::new(backend.ffprobe_path());
    cmd.args([
        "-v",
        "error",
        "-threads",
        "1",
        "-protocol_whitelist",
        "file",
        "-select_streams",
        &stream.to_string(),
    ]);
    if let Some((start, end)) = interval {
        cmd.args([
            "-read_intervals",
            &format!("{}%{}", seconds(start), seconds(end)),
        ]);
    }
    if include_frames {
        cmd.arg("-show_frames");
    }
    // Requesting frame entries itself enables ffprobe's decoder, even without
    // -show_frames. Output validation needs only headers before our one decode.
    let entries = format!(
        "{}stream=index,codec_type,time_base,start_time,duration,width,height,codec_name:format=start_time,duration",
        if include_frames {
            "frame=best_effort_timestamp,duration:"
        } else {
            ""
        }
    );
    cmd.args([
        "-show_streams",
        "-show_format",
        "-show_entries",
        &entries,
        "-of",
        "json",
    ])
    .arg(path);
    let output = process::run(&mut cmd, Duration::from_secs(120), control)?;
    crate::json::parse(&output.stdout)
}
fn frame_pts(probe: &Value) -> Result<Vec<i64>> {
    probe["frames"]
        .as_array()
        .ok_or_else(|| invalid("decoded frame index missing"))?
        .iter()
        .map(|f| {
            integer(&f["best_effort_timestamp"])
                .ok_or_else(|| invalid("decoded frame has no timestamp"))
        })
        .collect()
}

fn decoded_frame_pts(output: &[u8]) -> Result<(Vec<i64>, (u32, u32))> {
    let log =
        std::str::from_utf8(output).map_err(|_| invalid("decoded frame index is not UTF-8"))?;
    let mut time_base = None;
    let mut video_stream = false;
    let mut pts = Vec::new();
    for line in log.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(b) = line.strip_prefix("#tb 0:") {
            if time_base.replace(base(b.trim())?).is_some() {
                return Err(invalid("decoded frame index has multiple video clocks"));
            }
        } else if let Some(kind) = line.strip_prefix("#media_type 0:") {
            video_stream = kind.trim() == "video";
        } else if !line.starts_with('#') {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() < 6 {
                return Err(invalid("decoded frame index has an incomplete packet"));
            }
            let stream = fields[0]
                .parse::<u32>()
                .map_err(|_| invalid("invalid decoded stream"))?;
            let packet_pts = fields[2]
                .parse::<i64>()
                .map_err(|_| invalid("decoded frame timestamp missing"))?;
            if fields[1].parse::<i64>().is_err()
                || fields[3].parse::<u64>().is_err()
                || !fields[4].parse::<u64>().is_ok_and(|n| n > 0)
                || fields[5]
                    .strip_prefix("0x")
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .is_none()
            {
                return Err(invalid("decoded frame index has an invalid packet"));
            }
            if stream == 0 {
                pts.push(packet_pts);
            }
        }
    }
    if !video_stream || pts.is_empty() {
        return Err(invalid("decoded preview has no indexed video frames"));
    }
    Ok((
        pts,
        time_base.ok_or_else(|| invalid("decoded video clock missing"))?,
    ))
}

fn decode_preview(
    path: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<(Vec<i64>, (u32, u32))> {
    // Framecrc indexes raw decoded frames on stdout, avoiding stderr's bounded
    // diagnostic tail. Audio is decoded in the same pass; it is not stream copy.
    let output = process::run(
        Command::new(backend.ffmpeg_path())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-xerror",
                "-copyts",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-i",
            ])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-c:v",
                "rawvideo",
                "-threads:v",
                "1",
                "-c:a",
                "pcm_s16le",
                "-fps_mode",
                "passthrough",
                "-enc_time_base:v",
                "demux",
                "-f",
                "framecrc",
                "-",
            ]),
        Duration::from_secs(120),
        control,
    )?;
    decoded_frame_pts(&output.stdout)
}

pub fn preview(
    q: &BudgetPreview,
    asset: &Asset,
    source: &Path,
    scratch: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<(Value, Vec<Value>)> {
    let clock = clock(asset)?;
    let start = clock.origin.checked_add(workflow::time(q.start_ms))?;
    let end = clock.origin.checked_add(workflow::time(q.end_ms))?;
    // Read beyond the packet boundary to flush reordered B-frames, then
    // filter presentation timestamps to the actual requested interval.
    let source_probe = frame_probe(
        source,
        backend,
        clock.stream,
        Some((start, end.checked_add(workflow::time(2000))?)),
        true,
        control,
    )?;
    let source_pts = frame_pts(&source_probe)?
        .into_iter()
        .filter(|p| {
            time(*p, clock.base)
                .checked_cmp(start)
                .is_ok_and(|c| !c.is_lt())
                && time(*p, clock.base)
                    .checked_cmp(end)
                    .is_ok_and(|c| c.is_lt())
        })
        .collect::<Vec<_>>();
    if source_pts.is_empty() {
        return Err(invalid("preview interval has no decoded video frames"));
    }
    let path = scratch.join("proxy.mp4");
    let mut attempts = Vec::new();
    for (n, crf) in [24, 28, 32].into_iter().enumerate() {
        control.check()?;
        let upright = color::upright_size(
            asset
                .metadata
                .video
                .as_ref()
                .ok_or_else(|| invalid("video geometry unavailable"))?,
        );
        let divisor = [1.0, 0.75, 0.5][n];
        let width =
            ((f64::from(q.width.min(upright.width)) * divisor / 2.0).floor() as u32 * 2).max(2);
        let height =
            ((f64::from(q.height.min(upright.height)) * divisor / 2.0).floor() as u32 * 2).max(2);
        let color = color::source_filter(asset)?;
        let filter = format!(
            "{}scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1",
            if color.is_empty() {
                color
            } else {
                format!("{color},")
            }
        );
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        process::run(
            Command::new(backend.ffmpeg_path())
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-threads",
                    "1",
                    "-protocol_whitelist",
                    "file",
                    "-ss",
                    &seconds(workflow::time(q.start_ms)),
                    "-noautorotate",
                    "-i",
                ])
                .arg(source)
                .args([
                    "-map",
                    &format!("0:{}", clock.stream),
                    "-map",
                    "0:a:0?",
                    "-t",
                    &seconds(workflow::time(q.end_ms - q.start_ms)),
                    "-vf",
                    &filter,
                    "-fps_mode",
                    "passthrough",
                    "-enc_time_base:v",
                    "demux",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    &crf.to_string(),
                    "-threads",
                    "2",
                    "-x264-params",
                    "colorprim=bt709:transfer=bt709:colormatrix=bt709",
                    "-color_range",
                    "tv",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "64k",
                    "-map_metadata",
                    "-1",
                    "-metadata:s:v:0",
                    "rotate=0",
                    "-movflags",
                    "+faststart",
                ])
                .arg(&path),
            Duration::from_secs(180),
            control,
        )?;
        let bytes = std::fs::metadata(&path)?.len();
        attempts.push(json!({"widthBound":width,"heightBound":height,"crf":crf,"bytes":bytes}));
        if bytes > q.max_bytes {
            continue;
        }
        // Validate actual duration, frames and the source clock after encoding.
        let measured = frame_probe(&path, backend, 0, None, false, control)?;
        let duration = measured["format"]["duration"]
            .as_str()
            .ok_or_else(|| invalid("preview duration missing"))?;
        let actual_duration = decimal_time(duration)?;
        if actual_duration
            .checked_cmp(workflow::time(i64::from(q.max_duration_ms)))?
            .is_gt()
        {
            return Err(invalid(format!(
                "actual preview duration {duration}s exceeds maxDurationMs {}; use a shorter interval or a larger cap",
                q.max_duration_ms
            )));
        }
        let (output_pts, decoded_base) = decode_preview(&path, backend, control)?;
        if output_pts.len() != source_pts.len() {
            return Err(invalid(
                "preview dropped or duplicated source frames; source clock cannot be mapped exactly",
            ));
        }
        let stream = measured["streams"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| invalid("preview video stream missing"))?;
        if stream["codec_name"] != "h264"
            || !stream["width"]
                .as_u64()
                .is_some_and(|w| w > 0 && w <= u64::from(q.width))
            || !stream["height"]
                .as_u64()
                .is_some_and(|h| h > 0 && h <= u64::from(q.height))
        {
            return Err(invalid(
                "encoded preview codec or dimensions exceed the declared profile",
            ));
        }
        let output_base = base(
            stream["time_base"]
                .as_str()
                .ok_or_else(|| invalid("preview time base missing"))?,
        )?;
        if u64::from(output_base.0) * u64::from(decoded_base.1)
            != u64::from(decoded_base.0) * u64::from(output_base.1)
        {
            return Err(invalid(
                "decoded video clock differs from the preview container",
            ));
        }
        let offset = time(source_pts[0], clock.base)
            .checked_sub(clock.origin)?
            .checked_sub(time(output_pts[0], output_base))?;
        let tolerance = 2.0
            * (f64::from(clock.base.0) / f64::from(clock.base.1)
                + f64::from(output_base.0) / f64::from(output_base.1));
        let mut mapping = Vec::new();
        for (&src, &out) in source_pts.iter().zip(&output_pts) {
            let source_time = time(src, clock.base).checked_sub(clock.origin)?;
            let proxy_time = time(out, output_base);
            if (source_time
                .checked_sub(proxy_time)?
                .checked_sub(offset)?
                .as_seconds_f64())
            .abs()
                > tolerance
            {
                return Err(invalid(
                    "preview timestamps drift from source; refusing ambiguous mapping",
                ));
            }
            mapping.push(json!({"sourcePts":src,"proxyPts":out}));
        }
        if (offset.as_seconds_f64() - q.start_ms as f64 / 1000.0).abs() > tolerance.max(0.002) {
            return Err(invalid("preview source clock was shifted unexpectedly"));
        }
        let data = json!({"file":"proxy.mp4","mimeType":"video/mp4","quality":"SDR review preview; original media retained for final edits","notForFinal":true,"requestedSourceIntervalMs":[q.start_ms,q.end_ms],"maxDurationMs":q.max_duration_ms,"maxBytes":q.max_bytes,"bytes":bytes,"actualDuration":actual_duration,"verification":{"decoded":true,"frameCountMethod":"ffmpeg-framecrc-single-decode","videoFrames":output_pts.len(),"durationWithinBudget":true,"bytesWithinBudget":true},"width":stream["width"],"height":stream["height"],"attempts":attempts,"sourceClock":{"containerOrigin":clock.origin,"timeBase":format!("{}/{}",clock.base.0,clock.base.1)},"proxyTimeBase":format!("{}/{}",output_base.0,output_base.1),"sourceMinusProxyTime":offset,"observedFirstSourceTime":time(source_pts[0],clock.base).checked_sub(clock.origin)?,"observedLastSourceTime":time(*source_pts.last().expect("nonempty source frames"),clock.base).checked_sub(clock.origin)?,"frameMapping":mapping,"coverage":{"kind":"decoded-frame-preserving-window","semanticUnderstanding":false,"guidance":"Video frame timestamps are paired with the original. AAC is re-encoded; listen across boundaries. Preview resolution/color differ from the original. No host capability or policy is overridden."}});
        return Ok((
            data,
            vec![json!({"name":"proxy.mp4","sha256":media::hash_file_controlled(&path,control)?})],
        ));
    }
    let smallest = attempts
        .iter()
        .filter_map(|a| a["bytes"].as_u64())
        .min()
        .ok_or_else(|| invalid("preview has no measured encoding attempts"))?;
    Err(invalid(format!(
        "cannot fit preview within maxBytes {} after three bounded attempts; smallest observed attempt was {smallest} bytes; shorten the interval or increase the budget",
        q.max_bytes
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_index_keeps_video_clock_and_refuses_incomplete_evidence() {
        let log = b"#tb 0: 1/90000\n#media_type 0: video\n#tb 1: 1/48000\n#media_type 1: audio\n0, 3003, 3003, 3003, 120, 0x00000000\n1, 0, 0, 1024, 2048, 0x12345678\n0, 9009, 9009, 3003, 120, 0xabcdef12\n";
        assert_eq!(
            decoded_frame_pts(log).unwrap(),
            (vec![3003, 9009], (1, 90000))
        );
        for invalid in [
            "0, 0, 0, 1, 120, 0x00000000",
            "#tb 0: 1/0\n#media_type 0: video\n0, 0, 0, 1, 120, 0x00000000",
            "#tb 0: 1/1000\n#media_type 0: video\n0, 0, N/A, 1, 120, 0x00000000",
            "#tb 0: 1/1000\n#media_type 0: video\n0, 0, 0",
            "#tb 0: 1/1000\n#tb 0: 1/1000\n#media_type 0: video\n0, 0, 0, 1, 120, 0x00000000",
        ] {
            assert!(decoded_frame_pts(invalid.as_bytes()).is_err());
        }
    }
    #[test]
    fn exact_decimal_and_observed_frame_clock() {
        assert!((decimal_time("-0.125000").unwrap().as_seconds_f64() + 0.125).abs() < 1e-12);
        assert!(decimal_time("NaN").is_err());
        assert!(decimal_time("1.1234567890").is_err());
        let log = "[showinfo@sample7 @ 0x1] config in time_base: 1/90000, frame_rate: 30/1\n[showinfo@sample8 @ 0x2] config in time_base: 1/1000, frame_rate: 30/1\n[showinfo@sample8 @ 0x2] n:   0 pts: 900 pts_time:0.9\n[showinfo@sample7 @ 0x1] n:   0 pts: 123456 pts_time:1.37";
        assert_eq!(observed_frame(log, 7).unwrap(), (123456, (1, 90000)));
        assert_eq!(observed_frame(log, 8).unwrap(), (900, (1, 1000)));
        assert!(observed_frame(log, 9).is_err());
        assert!(observed_frame("pts_time:1.3", 7).is_err());
        assert!(
            observed_frame(
                &format!("{log}\n[showinfo@sample7 @ 0x1] n: 1 pts: 999 pts_time:0.1"),
                7
            )
            .is_err()
        );
    }
}
