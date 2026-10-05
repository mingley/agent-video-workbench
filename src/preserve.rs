//! Native YUV cut rendering: no RGB compositor or lossy working conversion.
use crate::{Error, Result, assembly, color};
use agentcut_core::{Asset, ItemPayload, Project, RationalRate, RoundingMode};
use agentcut_render::{
    FfmpegBackend,
    compile::{PlanInput, RenderPlan},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// Static mastering metadata is copied only after numeric validation, and only
/// when all source clips have the same metadata. Dolby Vision RPU is not copied.
pub fn hdr_metadata(asset: &Asset) -> Result<Value> {
    probe_hdr_metadata(asset.extensions.get(color::PROBE).unwrap_or(&Value::Null))
}

/// Older imports did not record frame-level HDR10 metadata. Read it from the
/// verified immutable original without editing the stored/frozen revision.
pub(crate) fn resolve_metadata(
    project: &Project,
    root: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn crate::process::Control,
) -> Result<Project> {
    let mut resolved = project.clone();
    for asset in &mut resolved.assets {
        if asset.metadata.video.as_ref().is_some_and(|v| {
            matches!(
                v.color_transfer.as_deref(),
                Some("smpte2084" | "arib-std-b67")
            )
        }) && asset
            .extensions
            .get(color::PROBE)
            .is_some_and(|p| p.get("initialVideoFrameSideData").is_none())
        {
            crate::media::verify_asset_controlled(root, asset, control)?;
            let path = root.join(&asset.uri);
            let probe = asset
                .extensions
                .get_mut(color::PROBE)
                .expect("checked probe");
            capture_metadata(backend, &path, probe, control)?;
        }
    }
    Ok(resolved)
}

pub(crate) fn capture_metadata(
    backend: &FfmpegBackend,
    path: &Path,
    probe: &mut Value,
    control: &mut dyn crate::process::Control,
) -> Result<()> {
    let result = crate::process::run(
        std::process::Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-threads",
                "1",
                "-select_streams",
                "v:0",
                "-read_intervals",
                "%+#1",
                "-show_frames",
                "-show_entries",
                "frame=side_data_list",
                "-of",
                "json",
            ])
            .arg(path),
        std::time::Duration::from_secs(60),
        control,
    )?;
    let initial: Value = serde_json::from_slice(&result.stdout)?;
    probe["initialVideoFrameSideData"] = json!(
        initial["frames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|frame| frame["side_data_list"].as_array())
            .flatten()
            .filter(|side| matches!(
                side["side_data_type"].as_str(),
                Some("Mastering display metadata" | "Content light level metadata")
            ))
            .collect::<Vec<_>>()
    );
    Ok(())
}

pub(crate) fn probe_hdr_metadata(probe: &Value) -> Result<Value> {
    let mut display = None;
    let mut light = None;
    if let Some(streams) = probe["streams"].as_array() {
        for side in streams
            .iter()
            .filter(|s| s["codec_type"] == "video")
            .filter_map(|s| s["side_data_list"].as_array())
            .flatten()
            .chain(
                probe["initialVideoFrameSideData"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
        {
            if side["side_data_type"] == "Mastering display metadata" {
                let component = |key: &str, scale: u64, maximum: u64| -> Result<u64> {
                    let raw = side[key]
                        .as_str()
                        .ok_or_else(|| invalid("unreadable mastering metadata"))?;
                    let (n, d) = raw
                        .split_once('/')
                        .ok_or_else(|| invalid("invalid mastering rational"))?;
                    let n: u64 = n
                        .parse()
                        .map_err(|_| invalid("invalid mastering numerator"))?;
                    let d: u64 = d
                        .parse()
                        .map_err(|_| invalid("invalid mastering denominator"))?;
                    let scaled = n
                        .checked_mul(scale)
                        .ok_or_else(|| invalid("mastering metadata overflow"))?;
                    if d == 0 || !scaled.is_multiple_of(d) || scaled / d > maximum {
                        return Err(invalid(
                            "mastering metadata is outside the supported HDR10 range",
                        ));
                    }
                    Ok(scaled / d)
                };
                let xy = |key| component(key, 50000, 50000);
                display = Some(format!(
                    "G({},{})B({},{})R({},{})WP({},{})L({},{})",
                    xy("green_x")?,
                    xy("green_y")?,
                    xy("blue_x")?,
                    xy("blue_y")?,
                    xy("red_x")?,
                    xy("red_y")?,
                    xy("white_point_x")?,
                    xy("white_point_y")?,
                    component("max_luminance", 10000, 100000000)?,
                    component("min_luminance", 10000, 100000000)?
                ));
            } else if side["side_data_type"] == "Content light level metadata" {
                let content = side["max_content"]
                    .as_u64()
                    .filter(|n| *n <= 65535)
                    .ok_or_else(|| invalid("invalid MaxCLL"))?;
                let average = side["max_average"]
                    .as_u64()
                    .filter(|n| *n <= 65535)
                    .ok_or_else(|| invalid("invalid MaxFALL"))?;
                light = Some(format!("{content},{average}"));
            }
        }
    }
    Ok(json!({"masterDisplay":display,"maxCll":light}))
}

pub(crate) fn doctor(backend: &FfmpegBackend, directory: &Path) -> Result<Value> {
    use std::{process::Command, time::Duration};
    let path = directory.join("hdr.mp4");
    crate::process::run(
        Command::new(backend.ffmpeg_path()).args([
            "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=64x64:r=30:d=0.2",
            "-vf", "format=yuv420p10le", "-c:v", "libx265", "-preset", "ultrafast",
            "-x265-params", "pools=2:frame-threads=1:log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:range=limited",
            "-tag:v", "hvc1", "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc", "-color_range", "tv",
        ]).arg(&path),
        Duration::from_secs(30), &mut crate::process::Uncontrolled,
    )?;
    let probe = crate::process::run(
        Command::new(backend.ffprobe_path())
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(&path),
        Duration::from_secs(30),
        &mut crate::process::Uncontrolled,
    )?;
    let probe: Value = serde_json::from_slice(&probe.stdout)?;
    let video = &probe["streams"][0];
    if video["codec_name"] != "hevc"
        || video["pix_fmt"] != "yuv420p10le"
        || video["color_transfer"] != "smpte2084"
        || video["color_primaries"] != "bt2020"
        || video["color_space"] != "bt2020nc"
        || video["color_range"] != "tv"
    {
        return Err(invalid(
            "configured HEVC encoder did not produce the 10-bit HDR contract",
        ));
    }
    crate::process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-v", "error", "-xerror", "-threads", "1", "-i"])
            .arg(&path)
            .args(["-vf", color::HDR_FILTER, "-f", "null", "-"]),
        Duration::from_secs(30),
        &mut crate::process::Uncontrolled,
    )?;
    Ok(
        json!({"ready":true,"checks":["HEVC 10-bit encode","PQ/BT.2020 tags and limited range","full decode and SDR preview tone map"],"scope":"plain source-preserving cuts; physical HDR display qualification is separate"}),
    )
}

pub fn plan(
    project: &Project,
    root: &Path,
    sequence: &str,
    output: &Path,
    backend: &FfmpegBackend,
    toolchain: String,
) -> Result<RenderPlan> {
    assembly::preflight(project, sequence)?;
    let seq = project.require_sequence(sequence)?;
    let policy = assembly::policy(seq)?.ok_or_else(|| invalid("assembly policy missing"))?;
    let clips = assembly::clips(seq)?;
    let ItemPayload::Clip(first) = &clips[0].payload else {
        unreachable!()
    };
    let first = project.require_asset(&first.asset_id)?;
    let contract = assembly::output_color(first, &policy)?;
    let rate = format!(
        "{}/{}",
        seq.frame_rate.numerator, seq.frame_rate.denominator
    );
    let mut args = Vec::new();
    let mut inputs = Vec::new();
    let mut filters = Vec::new();
    let mut concat = String::new();
    let mut frames = 0_i64;
    for (index, item) in clips.iter().enumerate() {
        let ItemPayload::Clip(clip) = &item.payload else {
            unreachable!()
        };
        let asset = project.require_asset(&clip.asset_id)?;
        let path = root.join(&asset.uri);
        let count = item
            .duration
            .rescale_to(seq.frame_rate, RoundingMode::Nearest)?
            .value;
        let samples = item
            .duration
            .rescale_to(RationalRate::frames(48000)?, RoundingMode::Nearest)?
            .value;
        frames = frames
            .checked_add(count)
            .ok_or_else(|| invalid("frame count overflow"))?;
        args.extend([
            "-ss".into(),
            format!("{:.9}", clip.source_range.start.as_seconds_f64()),
            "-i".into(),
            path.to_string_lossy().into_owned(),
        ]);
        inputs.push(PlanInput {
            asset_id: asset.id.clone(),
            path,
            fingerprint: asset.fingerprint.sha256.clone(),
        });
        let mut source = Vec::new();
        let video = asset
            .metadata
            .video
            .as_ref()
            .ok_or_else(|| invalid("source video missing"))?;
        if video.sample_aspect_ratio.numerator != video.sample_aspect_ratio.denominator {
            source.push("scale=iw*sar:ih,setsar=1".to_owned());
        }
        match video.rotation_degrees.rem_euclid(360) {
            90 => source.push("transpose=1".into()),
            180 => source.push("hflip,vflip".into()),
            270 => source.push("transpose=2".into()),
            _ => {}
        }
        if policy.color == assembly::DeliveryColor::Sdr {
            if let Some(filter) = color::filter(asset)? {
                source.push(filter.into());
            }
            source.push("scale=out_color_matrix=bt709:out_range=tv,format=yuv420p".into());
        } else {
            source.push(format!("format={}", contract.pixel_format));
        }
        // The explicit output grid keeps video duration aligned with the source
        // clock. VFR/rate changes require an explicit frameRate at assembly.
        source.push(format!(
            "fps={rate}:start_time=0,trim=end_frame={count},setpts=N/({rate}*TB),setsar=1"
        ));
        filters.push(format!("[{index}:v]{}[v{index}]", source.join(",")));
        let audio = if asset.metadata.audio.is_some() {
            format!(
                "[{index}:a]aresample=48000:async=1:first_pts=0,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=whole_len={samples},atrim=end_sample={samples},asetpts=N/SR/TB[a{index}]"
            )
        } else {
            format!(
                "anullsrc=r=48000:cl=stereo,atrim=end_sample={samples},asetpts=N/SR/TB[a{index}]"
            )
        };
        filters.push(audio);
        concat.push_str(&format!("[v{index}][a{index}]"));
    }
    filters.push(format!(
        "{concat}concat=n={}:v=1:a=1[vout][aout]",
        clips.len()
    ));
    args.extend([
        "-filter_complex".into(),
        filters.join(";"),
        "-map".into(),
        "[vout]".into(),
        "-map".into(),
        "[aout]".into(),
        "-c:v".into(),
        if contract.codec == "hevc" {
            "libx265"
        } else {
            "libx264"
        }
        .into(),
        "-preset".into(),
        "medium".into(),
        "-pix_fmt".into(),
        contract.pixel_format.clone(),
    ]);
    if contract.codec == "hevc" {
        let mut params = format!(
            "pools=2:frame-threads=1:log-level=error:colorprim={}:transfer={}:colormatrix={}:range=limited",
            contract.primaries, contract.transfer, contract.matrix
        );
        if policy.quality == assembly::Quality::Lossless {
            params.push_str(":lossless=1");
        } else {
            args.extend(["-crf".into(), "16".into()]);
        }
        if contract.hdr {
            let metadata = hdr_metadata(first)?;
            if let Some(value) = metadata["masterDisplay"].as_str() {
                params.push_str(&format!(":master-display={value}"));
            }
            if let Some(value) = metadata["maxCll"].as_str() {
                params.push_str(&format!(":max-cll={value}"));
            }
        }
        args.extend([
            "-x265-params".into(),
            params,
            "-tag:v".into(),
            "hvc1".into(),
        ]);
    } else {
        args.extend([
            "-crf".into(),
            if policy.quality == assembly::Quality::Lossless {
                "0"
            } else {
                "16"
            }
            .into(),
            "-x264-params".into(),
            "colorprim=bt709:transfer=bt709:colormatrix=bt709".into(),
        ]);
    }
    let temporary = output.with_file_name("assembly.partial.mp4");
    args.extend([
        "-color_primaries".into(),
        contract.primaries,
        "-color_trc".into(),
        contract.transfer,
        "-colorspace".into(),
        contract.matrix,
        "-color_range".into(),
        "tv".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "256k".into(),
        "-ar".into(),
        "48000".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-r".into(),
        rate,
        "-fps_mode".into(),
        "cfr".into(),
        "-frames:v".into(),
        frames.to_string(),
        "-y".into(),
        temporary.to_string_lossy().into_owned(),
    ]);
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"adapter":"native-yuv-v1","args":args,"inputs":inputs,"quality":policy.quality,"toolchain":toolchain})
        )?)
    );
    Ok(RenderPlan {
        schema_version: "1.0.0".into(),
        render_id: uuid::Uuid::new_v4(),
        project_id: project.project_id,
        project_revision: project.revision,
        sequence_id: sequence.into(),
        preset: "source-preserving".into(),
        preset_version: 1,
        deterministic: true,
        output: output.into(),
        temporary_output: temporary,
        program: backend.ffmpeg_path().into(),
        args,
        inputs,
        frame_count: frames,
        duration_seconds: frames as f64 / seq.frame_rate.as_f64(),
        plan_hash: hash,
        toolchain,
        side_files: Vec::new(),
        warnings: Vec::new(),
    })
}

pub fn output(project: &Project, sequence: &str) -> Result<assembly::OutputColor> {
    let seq = project.require_sequence(sequence)?;
    if let Some(policy) = assembly::policy(seq)? {
        let items = assembly::clips(seq)?;
        let ItemPayload::Clip(clip) = &items[0].payload else {
            unreachable!()
        };
        assembly::output_color(project.require_asset(&clip.asset_id)?, &policy)
    } else {
        Ok(assembly::OutputColor::sdr())
    }
}

pub fn review_preview(
    master: &Path,
    directory: &Path,
    project: &Project,
    sequence: &str,
    backend: &FfmpegBackend,
    control: &mut dyn crate::process::Control,
) -> Result<Value> {
    use std::{process::Command, time::Duration};
    let contract = output(project, sequence)?;
    let mut review = project.clone();
    let seq = review.require_sequence_mut(sequence)?;
    let factor = (1280.0 / f64::from(seq.canvas.width))
        .min(1280.0 / f64::from(seq.canvas.height))
        .min(1.0);
    seq.canvas.width = ((f64::from(seq.canvas.width) * factor / 2.0).floor() as u32) * 2;
    seq.canvas.height = ((f64::from(seq.canvas.height) * factor / 2.0).floor() as u32) * 2;
    seq.extensions.remove(assembly::POLICY);
    let prefix = if contract.hdr {
        color::HDR_FILTER
    } else {
        "zscale=dither=error_diffusion,format=yuv420p"
    };
    let filter = format!(
        "{prefix},scale={}:{}:out_color_matrix=bt709:out_range=tv,setsar=1",
        seq.canvas.width, seq.canvas.height
    );
    let path = directory.join("review-sdr.mp4");
    crate::process::run(
        Command::new(backend.ffmpeg_path())
            .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
            .arg(master)
            .args([
                "-vf",
                &filter,
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "18",
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
                "-color_range",
                "tv",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
            ])
            .arg(&path),
        Duration::from_secs(8 * 3600),
        control,
    )?;
    let frames = seq
        .content_duration()?
        .rescale_to(seq.frame_rate, RoundingMode::Nearest)?
        .value;
    let verification = crate::media::verify(&path, backend, frames, &review, sequence, control)?;
    std::fs::File::open(&path)?.sync_all()?;
    Ok(verification)
}
