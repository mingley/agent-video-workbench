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

// Imported probe metadata describes the container origin and the first
// supported media stream. Without both reported timestamps and a time base,
// FFmpeg's filter-relative timestamps cannot safely be labelled source times.
fn stream_clock(asset: &agentcut_core::Asset, kind: Kind) -> (Value, bool, Option<u64>) {
    let probe = asset.extensions.get(crate::color::PROBE);
    let stream_type = if matches!(kind, Kind::Silence) {
        "audio"
    } else {
        "video"
    };
    let stream = probe
        .and_then(|p| p["streams"].as_array())
        .and_then(|streams| {
            streams.iter().find(|stream| {
                stream["codec_type"] == stream_type
                    && !(stream_type == "video" && stream["disposition"]["attached_pic"] == 1)
            })
        });
    let number = |v: &Value| {
        v.as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let origin = probe.and_then(|p| number(&p["format"]["start_time"]));
    let stream_start = stream.and_then(|s| number(&s["start_time"]));
    let index = stream.and_then(|s| s["index"].as_u64());
    let sample_rate = stream
        .and_then(|s| s["sample_rate"].as_str())
        .and_then(|rate| rate.parse::<u32>().ok())
        .filter(|rate| *rate > 0);
    let time_base = stream.and_then(|s| s["time_base"].as_str()).filter(|rate| {
        rate.split_once('/').is_some_and(|(n, d)| {
            n.parse::<u32>().is_ok_and(|n| n > 0) && d.parse::<u32>().is_ok_and(|d| d > 0)
        })
    });
    let mapped =
        origin.is_some() && stream_start.is_some() && index.is_some() && time_base.is_some();
    let reported_start_ms = origin.zip(stream_start).map(|(o, s)| (s - o) * 1000.0);
    (
        json!({"stream":stream_type,"streamIndex":index,"containerStartSeconds":origin,
            "streamStartSeconds":stream_start,"timeBase":time_base,"sampleRateHz":sample_rate,
            "reportedStreamStartMs":reported_start_ms,
            "mapping":if mapped {"reported-container-origin"} else {"unknown"},
            "guidance":"FFmpeg seek and decoded PTS are relative to the reported container origin; missing or discontinuous timestamps can limit this mapping"}),
        mapped,
        index,
    )
}

// silencedetect derives silence starts from sample counts. Across an audio PTS
// gap that can place an event inside missing media, so only expose findings
// when decoded audio frames observed in this interval have continuous clocks.
// The subprocess reader limits stdout to 8 MiB; a missing/oversized/failed
// probe is an unknown result, never evidence of silence.
fn audio_continuity(
    source: &Path,
    clock: &Value,
    backend: &FfmpegBackend,
    start_ms: i64,
    end_ms: i64,
) -> Value {
    let unknown = |reason| json!({"status":"unknown","reason":reason});
    let Some(origin) = clock["containerStartSeconds"].as_f64() else {
        return unknown("container-origin-unavailable");
    };
    let Some(index) = clock["streamIndex"].as_u64() else {
        return unknown("audio-stream-unavailable");
    };
    let Some(rate) = clock["sampleRateHz"]
        .as_u64()
        .filter(|rate| *rate > 0 && *rate <= 768_000)
    else {
        return unknown("audio-sample-rate-unavailable");
    };
    let Some((n, d)) = clock["timeBase"].as_str().and_then(|v| v.split_once('/')) else {
        return unknown("audio-time-base-unavailable");
    };
    let Some(tick) = n
        .parse::<u32>()
        .ok()
        .zip(d.parse::<u32>().ok())
        .filter(|(n, d)| *n > 0 && *d > 0)
        .map(|(n, d)| f64::from(n) / f64::from(d))
    else {
        return unknown("audio-time-base-unavailable");
    };
    if tick > 0.01 {
        return unknown("audio-time-base-too-coarse");
    }
    let start = origin + start_ms as f64 / 1000.0;
    let end = origin + end_ms as f64 / 1000.0;
    if !start.is_finite() || !end.is_finite() {
        return unknown("audio-time-range-unrepresentable");
    }
    let output = match process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-select_streams",
                &index.to_string(),
                "-read_intervals",
                &format!("{start:.9}%{end:.9}"),
                "-show_frames",
                "-show_entries",
                "frame=best_effort_timestamp_time,nb_samples",
                "-of",
                "json",
            ])
            .arg(source),
        Duration::from_secs(600),
        &mut Uncontrolled,
    ) {
        Ok(output) => output,
        Err(Error::Invalid(message)) if message.contains("exceeds 8 MiB") => {
            return unknown("audio-frame-probe-output-limit");
        }
        Err(_) => return unknown("audio-frame-probe-failed"),
    };
    let Ok(parsed) = crate::json::parse::<Value>(&output.stdout) else {
        return unknown("audio-frame-probe-invalid");
    };
    let Some(frames) = parsed["frames"].as_array() else {
        return unknown("audio-frames-unavailable");
    };
    let Some(stream_start) = clock["streamStartSeconds"].as_f64() else {
        return unknown("audio-stream-start-unavailable");
    };
    let mut previous_end: Option<f64> = None;
    let mut first_start: Option<f64> = None;
    let mut observed = 0_u64;
    // Two ticks account for container timestamp quantization; two samples
    // allow rounding in decoded frame duration. Coarser clocks are refused.
    let tolerance = 2.0 * tick + 2.0 / rate as f64;
    for frame in frames {
        let Some(pts) = frame["best_effort_timestamp_time"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
        else {
            return unknown("audio-frame-timestamp-unavailable");
        };
        let Some(samples) = frame["nb_samples"].as_u64().filter(|v| *v > 0) else {
            return unknown("audio-frame-sample-count-unavailable");
        };
        let frame_end = pts + samples as f64 / rate as f64;
        if !frame_end.is_finite() {
            return unknown("audio-frame-time-unrepresentable");
        }
        if frame_end <= start || pts >= end {
            continue;
        }
        if let Some(previous) = previous_end
            && (pts - previous).abs() > tolerance
        {
            return json!({"status":"discontinuous","reason":"decoded-audio-pts-gap-or-overlap"});
        }
        if first_start.is_none() {
            first_start = Some(pts);
        }
        previous_end = Some(frame_end);
        observed += 1;
    }
    if observed == 0 {
        return unknown("no-decoded-audio-frames");
    }
    let first = first_start.unwrap_or(start);
    let last = previous_end.unwrap_or(end);
    // Refuse incomplete probing: independent FFmpeg/ffprobe seeks can land
    // on different subsets. A reported initial stream delay is not silence.
    if first > start.max(stream_start) + tolerance {
        return unknown("audio-probe-leading-coverage-incomplete");
    }
    if last < end - tolerance {
        return unknown("audio-probe-trailing-coverage-incomplete");
    }
    json!({"status":"contiguous-observed","frames":observed,"toleranceSeconds":tolerance,
        "observedStartMs":(first-origin)*1000.0,"observedEndMs":(last-origin)*1000.0,
        "guidance":"Only decoded frames returned by the bounded probe are checked; leading/trailing gaps and independent decoder behavior remain uncertain"})
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
    let end = match end_ms {
        Some(end) => end,
        None => start_ms.checked_add(60_000).ok_or_else(|| {
            Error::Invalid("inspection range exceeds supported source time".into())
        })?,
    };
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
    let (clock, mapped, stream_index) = stream_clock(asset, kind);
    if matches!(kind, Kind::Scenes)
        && stream_index.is_none()
        && asset
            .extensions
            .get(crate::color::PROBE)
            .and_then(|probe| probe["streams"].as_array())
            .is_some()
    {
        return Err(Error::Invalid(
            "scene inspection requires a video stream other than attached artwork".into(),
        ));
    }
    let probe_tool = if matches!(kind, Kind::Silence) {
        process::run(
            Command::new(backend.ffprobe_path()).arg("-version"),
            Duration::from_secs(10),
            &mut Uncontrolled,
        )
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    };
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"source":asset.fingerprint.sha256,"metadata":asset.metadata,"clock":clock,
                "kind":kind,"startMs":start_ms,"endMs":end,"tool":String::from_utf8_lossy(&tool.stdout),
                "probeTool":probe_tool,"probePath":backend.ffprobe_path(),"version":4})
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
        // Evidence is keyed by source bytes and may be shared by several asset
        // IDs. Bind the returned identity to this request, not the asset that
        // first populated the cache.
        result["assetId"] = json!(asset_id);
        result["cacheHit"] = json!(true);
        return Ok(result);
    }
    let mut command = Command::new(backend.ffmpeg_path());
    command.args(["-nostdin", "-v", "info"]);
    if matches!(kind, Kind::Silence | Kind::Scenes) {
        command.arg("-copyts");
    }
    command
        .args([
            "-protocol_whitelist",
            "file",
            "-ss",
            &format!("{:.3}", start_ms as f64 / 1000.0),
            "-noautorotate",
            "-i",
        ])
        .arg(&source);
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
            let duration = end - start_ms;
            let seconds = format!("{:.3}", duration as f64 / 1000.0);
            let mut evidence = Vec::new();
            let continuity = if matches!(kind, Kind::Silence) && mapped {
                Some(audio_continuity(&source, &clock, backend, start_ms, end))
            } else if matches!(kind, Kind::Silence) {
                Some(json!({"status":"unknown","reason":"source-clock-unavailable"}))
            } else {
                None
            };
            let mut unavailable = if !mapped {
                Some("source-timestamp-mapping-unavailable")
            } else if continuity
                .as_ref()
                .is_some_and(|value| value["status"] != "contiguous-observed")
            {
                Some("decoded-audio-continuity-not-established")
            } else {
                None
            };
            let mut diagnostics_truncated = false;
            let mut first_video_ms: Option<i64> = None;
            let mut leading_incomplete = false;
            if unavailable.is_none() {
                let origin = clock["containerStartSeconds"]
                    .as_f64()
                    .ok_or_else(|| Error::Invalid("source clock missing".into()))?;
                let absolute_start = origin + start_ms as f64 / 1000.0;
                let absolute_end = origin + end as f64 / 1000.0;
                if !absolute_start.is_finite() || !absolute_end.is_finite() {
                    return Err(Error::Invalid(
                        "source time range cannot be represented".into(),
                    ));
                }
                let start = format!("{absolute_start:.9}");
                let stop = format!("{absolute_end:.9}");
                let stream_map = format!(
                    "0:{}",
                    stream_index.ok_or_else(|| Error::Invalid("source stream missing".into()))?
                );
                command.args(["-t", &seconds, "-map", &stream_map]);
                if matches!(kind, Kind::Silence) {
                    command.args(["-vn", "-af", &format!("asettb=AVTB,atrim=start={start}:end={stop},asetpts=PTS-({start})/TB,silencedetect=noise=-35dB:d=0.4")]);
                } else {
                    // The first selected frame is a sentinel: if a long-GOP
                    // seek produces no frames, an empty result is unavailable,
                    // not proof that the interval has no scene changes.
                    command.args(["-an", "-vf", &format!("settb=AVTB,trim=start={start}:end={stop},setpts=PTS-({start})/TB,select='eq(n,0)+gt(scene,0.35)',metadata=print")]);
                }
                command.args(["-f", "null", "-"]);
                let output =
                    process::run(&mut command, Duration::from_secs(600), &mut Uncontrolled)?;
                diagnostics_truncated = output.stderr.len() >= 64 * 1024;
                if diagnostics_truncated {
                    unavailable = Some("detector-diagnostics-truncated");
                }
                if output.stderr.contains("Output file is empty") {
                    unavailable = Some("no-decoded-stream-samples");
                }
                let mut pending_scene: Option<(f64, i64)> = None;
                let mut observed_video_frame = false;
                let mut silence_open = false;
                let boundary_tolerance_ms = continuity
                    .as_ref()
                    .and_then(|c| c["toleranceSeconds"].as_f64())
                    .map_or(1.0, |value| value * 1000.0 + 1.0);
                for line in output.stderr.lines() {
                    let tags: &[&str] = if matches!(kind, Kind::Silence) {
                        &["silence_start:", "silence_end:"]
                    } else {
                        &["pts_time:", "lavfi.scene_score="]
                    };
                    for tag in tags {
                        if let Some((_, suffix)) = line.split_once(tag)
                            && let Some(value) = suffix
                                .split_whitespace()
                                .next()
                                .and_then(|s| s.parse::<f64>().ok())
                                .filter(|value| value.is_finite())
                        {
                            if *tag == "lavfi.scene_score=" {
                                if let Some((pts, at)) = pending_scene.take()
                                    && value > 0.35
                                {
                                    evidence.push(json!({"kind":"pts_time","value":pts,"relativeTimeMs":at,"sourceTimeMs":start_ms + at}));
                                    evidence.push(json!({"kind":"lavfi.scene_score","value":value,"sourceTimeMs":Value::Null}));
                                }
                                continue;
                            }
                            let relative_ms = value * 1000.0;
                            let is_end = *tag == "silence_end:";
                            let tolerance = if matches!(kind, Kind::Silence) {
                                boundary_tolerance_ms
                            } else {
                                1.0
                            };
                            let in_range = relative_ms >= -tolerance
                                && if is_end {
                                    relative_ms <= duration as f64 + tolerance
                                } else {
                                    relative_ms < duration as f64
                                };
                            if !in_range {
                                if *tag == "pts_time:" {
                                    pending_scene = None;
                                } else {
                                    unavailable = Some("silence-finding-outside-window");
                                }
                                continue;
                            }
                            let rounded = (relative_ms.round() as i64).max(0).min(if is_end {
                                duration
                            } else {
                                duration - 1
                            });
                            if *tag == "pts_time:" {
                                observed_video_frame = true;
                                if first_video_ms.is_none() {
                                    first_video_ms = Some(start_ms + rounded);
                                }
                                pending_scene = Some((value, rounded));
                            } else {
                                if is_end == silence_open {
                                    silence_open = !is_end;
                                } else {
                                    unavailable = Some("unpaired-silence-finding");
                                }
                                evidence.push(json!({"kind":tag.trim_end_matches([':', '=']),"value":value,
                                    "relativeTimeMs":rounded,"sourceTimeMs":start_ms + rounded,
                                    "boundaryMayBeTruncated":is_end || rounded == 0,
                                    "endCause":if is_end {Some("sound-resumption-or-decoded-stream-end")} else {None}}));
                            }
                        }
                    }
                }
                if silence_open {
                    unavailable = Some("unpaired-silence-finding");
                }
                if matches!(kind, Kind::Scenes) && !observed_video_frame {
                    unavailable = Some("no-decoded-video-frames");
                    evidence.clear();
                }
                if matches!(kind, Kind::Scenes)
                    && let Some(first) = first_video_ms
                {
                    let expected = clock["reportedStreamStartMs"]
                        .as_f64()
                        .unwrap_or(start_ms as f64)
                        .max(start_ms as f64);
                    let nominal = asset
                        .metadata
                        .video
                        .as_ref()
                        .map(|video| video.frame_rate.as_f64())
                        .filter(|rate| rate.is_finite() && *rate > 0.0);
                    let tolerance = nominal
                        .map(|rate| (2000.0 / rate).clamp(50.0, 100.0))
                        .unwrap_or(50.0);
                    if first as f64 > expected + tolerance {
                        leading_incomplete = true;
                        unavailable = Some("video-seek-leading-coverage-incomplete");
                    }
                }
                if let Some(continuity) = &continuity
                    && continuity["status"] == "contiguous-observed"
                {
                    let first = continuity["observedStartMs"]
                        .as_f64()
                        .unwrap_or(f64::INFINITY);
                    let last = continuity["observedEndMs"]
                        .as_f64()
                        .unwrap_or(f64::NEG_INFINITY);
                    let tolerance =
                        continuity["toleranceSeconds"].as_f64().unwrap_or(0.0) * 1000.0 + 1.0;
                    if evidence
                        .iter()
                        .filter_map(|item| item["sourceTimeMs"].as_i64())
                        .any(|at| (at as f64) < first - tolerance || (at as f64) > last + tolerance)
                    {
                        unavailable = Some("silence-finding-outside-probed-audio-span");
                    }
                }
                if unavailable.is_some() {
                    evidence.clear();
                }
            }
            result["analysisStatus"] = json!(if unavailable.is_some() {
                "unavailable"
            } else {
                "available"
            });
            result["unavailableReason"] = json!(unavailable);
            result["evidence"] = json!(evidence);
            let source_end_ms = asset
                .metadata
                .duration
                .map(|d| d.as_seconds_f64() * 1000.0)
                .filter(|v| v.is_finite());
            result["timing"] = clock;
            result["coverage"] = json!({"requestedRangeMs":[start_ms,end],"sourceDurationMs":source_end_ms,
                "requestedEndExceedsReportedDuration":source_end_ms.map(|d| end as f64 > d),
                "leadingCoverage":if diagnostics_truncated {"unknown"} else if leading_incomplete {"incomplete"} else {"not-measured"},
                "firstDecodedVideoTimeMs":if diagnostics_truncated {None} else {first_video_ms},"trailingCoverage":"not-measured","audioContinuity":continuity,
                "guidance":"Only decoded samples are inspected. Stream-edge coverage is unmeasured. Silence findings are unavailable when audio-frame continuity is unknown or discontinuous; a silence end may be decoded-stream EOF. A scene score compares successive decoded frames, including across any video gap, and the first frame has no prior in-window comparison."});
            result["suggestionsOnly"] = json!(true);
            result["diagnosticsMayBeTruncated"] = json!(diagnostics_truncated);
        }
        Kind::Metadata => unreachable!(),
    }
    // Unknown evidence can become available on retry (for example, after a
    // transient probe failure). Never freeze an unavailable result in cache.
    if result["analysisStatus"] == "unavailable" {
        return Ok(result);
    }
    let staged = directory.join(format!("{key}-{}.partial.json", uuid::Uuid::new_v4()));
    std::fs::write(&staged, serde_json::to_vec_pretty(&result)?)?;
    File::open(&staged)?.sync_all()?;
    std::fs::rename(staged, manifest)?;
    File::open(directory)?.sync_all()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bounded_audio_continuity_rejects_missing_gapped_or_oversized_probe_results() {
        let tmp = tempfile::tempdir().unwrap();
        let probe = tmp.path().join("ffprobe-fixture");
        let source = tmp.path().join("source");
        std::fs::write(&source, b"fixture").unwrap();
        let clock = json!({"containerStartSeconds":0.0,"streamStartSeconds":0.0,"streamIndex":0,
            "sampleRateHz":48000,"timeBase":"1/1000"});
        let set_output = |body: &str| {
            std::fs::write(&probe, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let backend = crate::media::backend("unused".into(), probe.clone());
        set_output(
            "printf '%s' '{\"frames\":[{\"best_effort_timestamp_time\":\"0\",\"nb_samples\":48000},{\"best_effort_timestamp_time\":\"1\",\"nb_samples\":48000}]}'",
        );
        let good = audio_continuity(&source, &clock, &backend, 0, 2000);
        assert_eq!(good["status"], "contiguous-observed");
        assert_eq!(good["observedStartMs"], 0.0);
        assert_eq!(good["observedEndMs"], 2000.0);
        set_output(
            "printf '%s' '{\"frames\":[{\"best_effort_timestamp_time\":\"0\",\"nb_samples\":48000},{\"best_effort_timestamp_time\":\"4\",\"nb_samples\":48000}]}'",
        );
        assert_eq!(
            audio_continuity(&source, &clock, &backend, 0, 5000)["status"],
            "discontinuous"
        );
        set_output("printf '%s' '{\"frames\":[{\"nb_samples\":48000}]}'");
        assert_eq!(
            audio_continuity(&source, &clock, &backend, 0, 2000)["reason"],
            "audio-frame-timestamp-unavailable"
        );
        set_output("head -c 8388609 /dev/zero");
        assert_eq!(
            audio_continuity(&source, &clock, &backend, 0, 2000)["reason"],
            "audio-frame-probe-output-limit"
        );
    }
}
