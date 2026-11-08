//! Source-clock evidence for local ASR. All timestamps use 16 kHz sample ticks.
use crate::{
    Error, Result,
    process::{self, Control},
};
use agentcut_render::FfmpegBackend;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::Command,
    time::Duration,
};

pub(super) const SAMPLE_RATE: i128 = 16_000;
pub(super) const MAX_AUDIT_BYTES: u64 = 16 * 1024 * 1024;
const AUDIT_STOP_MARGIN: u64 = 4096;
const MAX_WAV_BYTES: u64 = 120 * 1024 * 1024;

pub(super) struct ClockProbe {
    pub(super) stream_index: u64,
    pub(super) origin_ticks: Option<i128>,
    pub(super) tolerance_ticks: i128,
    pub(super) zero_origin_wav: bool,
}

pub(super) struct AudioClock {
    pub(super) offset_ticks: i128,
    pub(super) samples: i128,
}

fn invalid(message: &str) -> Error {
    Error::Invalid(format!("audio clock: {message}"))
}

fn probe_error(error: Error) -> Error {
    match error {
        Error::Cancelled => Error::Cancelled,
        Error::Timeout => Error::Timeout,
        other => invalid(&format!("ffprobe failed: {other}")),
    }
}

// ffprobe prints decimal format start times. Convert exactly to the nearest
// 16 kHz tick, with ties away from zero, without f64 or unchecked arithmetic.
fn decimal_ticks(value: &str) -> Result<i128> {
    let (negative, unsigned) = if let Some(rest) = value.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = value.strip_prefix('+') {
        (false, rest)
    } else {
        (false, value)
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("invalid container start time"));
    }
    let scale = 10_i128
        .checked_pow(u32::try_from(fraction.len()).map_err(|_| invalid("invalid start time"))?)
        .ok_or_else(|| invalid("invalid start time"))?;
    let whole: i128 = whole
        .parse()
        .map_err(|_| invalid("container start time overflow"))?;
    let fractional: i128 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse()
            .map_err(|_| invalid("invalid container start time"))?
    };
    let magnitude = whole
        .checked_mul(scale)
        .and_then(|v| v.checked_add(fractional))
        .and_then(|v| v.checked_mul(SAMPLE_RATE))
        .ok_or_else(|| invalid("container start time overflow"))?;
    let rounded = magnitude
        .checked_div(scale)
        .and_then(|v| v.checked_add(i128::from(magnitude % scale >= (scale + 1) / 2)))
        .ok_or_else(|| invalid("container start time overflow"))?;
    if negative {
        rounded
            .checked_neg()
            .ok_or_else(|| invalid("container start time overflow"))
    } else {
        Ok(rounded)
    }
}

// Permit at most one source timebase tick plus one output-sample tick, and
// refuse source clocks too coarse to establish approximately 1 ms alignment.
fn tolerance_ticks(time_base: &str) -> Result<i128> {
    let (num, den) = time_base
        .split_once('/')
        .ok_or_else(|| invalid("source audio time base is missing"))?;
    let num: i128 = num
        .parse()
        .map_err(|_| invalid("invalid source audio time base"))?;
    let den: i128 = den
        .parse()
        .map_err(|_| invalid("invalid source audio time base"))?;
    if num <= 0 || den <= 0 {
        return Err(invalid("invalid source audio time base"));
    }
    let scaled = num
        .checked_mul(SAMPLE_RATE)
        .ok_or_else(|| invalid("source audio time base overflow"))?;
    let ticks = scaled
        .checked_div(den)
        .and_then(|v| v.checked_add(i128::from(scaled % den != 0)))
        .and_then(|v| v.checked_add(1))
        .ok_or_else(|| invalid("source audio time base overflow"))?;
    if ticks > 17 {
        return Err(invalid("source audio time base is too coarse"));
    }
    Ok(ticks)
}

fn probe_value(report: &Value) -> Result<ClockProbe> {
    let streams = report["streams"]
        .as_array()
        .ok_or_else(|| invalid("source streams are missing"))?;
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio")
        .ok_or_else(|| invalid("source audio stream is missing"))?;
    let stream_index = audio["index"]
        .as_u64()
        .ok_or_else(|| invalid("source audio stream index is missing"))?;
    let tolerance_ticks = tolerance_ticks(
        audio["time_base"]
            .as_str()
            .ok_or_else(|| invalid("source audio time base is missing"))?,
    )?;
    let start = report["format"]["start_time"].as_str().filter(|s| *s != "N/A");
    let origin_ticks = start.map(decimal_ticks).transpose()?;
    // WAV does not carry an absolute stream start time. Only an audio-only,
    // one-stream WAV may defer the zero-origin decision to the observed PTS.
    let is_wav = report["format"]["format_name"]
        .as_str()
        .is_some_and(|names| names.split(',').any(|name| name == "wav"));
    let zero_origin_wav = origin_ticks.is_none()
        && is_wav
        && streams.len() == 1
        && streams[0]["codec_type"] == "audio";
    if origin_ticks.is_none() && !zero_origin_wav {
        return Err(invalid("container start time is unavailable"));
    }
    Ok(ClockProbe {
        stream_index,
        origin_ticks,
        tolerance_ticks,
        zero_origin_wav,
    })
}

pub(super) fn probe_audio_clock(
    source: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<ClockProbe> {
    let output = process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-show_format",
                "-show_streams",
                "-show_entries",
                "format=format_name,start_time:stream=index,codec_type,time_base",
                "-of",
                "json",
            ])
            .arg(source),
        Duration::from_secs(120),
        control,
    )
    .map_err(probe_error)?;
    let report: Value =
        crate::json::parse(&output.stdout).map_err(|_| invalid("invalid source probe"))?;
    probe_value(&report)
}

fn first_input_pts(stderr: &str) -> Result<i128> {
    let mut observed = None;
    for line in stderr.lines().filter(|line| line.contains("ashowinfo@asr_clock")) {
        let Some((_, body)) = line.split_once(']') else {
            continue;
        };
        let mut frame = None;
        let mut pts = None;
        for token in body.split_whitespace() {
            if let Some(value) = token.strip_prefix("n:") {
                frame = Some(value);
            } else if let Some(value) = token.strip_prefix("pts:") {
                pts = Some(value);
            }
        }
        if frame == Some("0") {
            let pts: i128 = pts
                .ok_or_else(|| invalid("first source PTS is missing"))?
                .parse()
                .map_err(|_| invalid("first source PTS is invalid"))?;
            if observed.replace(pts).is_some() {
                return Err(invalid("multiple first source PTS records"));
            }
        }
    }
    observed.ok_or_else(|| invalid("first source PTS is unavailable"))
}

struct Audit {
    first_pts: i128,
    samples: i128,
}

fn read_audit(path: &Path, tolerance: i128, control: &mut dyn Control) -> Result<Audit> {
    if !(1..=17).contains(&tolerance) {
        return Err(invalid("invalid PTS tolerance"));
    }
    control.check()?;
    crate::media::regular_file(path, "audio clock audit")?;
    let file = File::open(path).map_err(|_| invalid("cannot open audit"))?;
    if file
        .metadata()
        .map_err(|_| invalid("cannot inspect audit"))?
        .len()
        >= MAX_AUDIT_BYTES - AUDIT_STOP_MARGIN
    {
        return Err(invalid("audit reached its size limit and may be truncated"));
    }
    let mut reader = BufReader::new(file.take(MAX_AUDIT_BYTES + 1));
    let mut line = String::new();
    let mut bytes = 0_u64;
    let mut headers = [false; 5];
    let mut first = None;
    let mut previous = None;
    let mut samples = 0_i128;
    let mut records = 0_u64;

    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .map_err(|_| invalid("cannot read audit"))?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(count).map_err(|_| invalid("audit size overflow"))?)
            .ok_or_else(|| invalid("audit size overflow"))?;
        if bytes >= MAX_AUDIT_BYTES - AUDIT_STOP_MARGIN || line.len() > 4096 || !line.ends_with('\n') {
            return Err(invalid("audit is too large or incomplete"));
        }
        let row = line.trim();
        if row.starts_with('#') {
            let expected = [
                "#tb 0: 1/16000",
                "#media_type 0: audio",
                "#codec_id 0: pcm_s16le",
                "#sample_rate 0: 16000",
                "#channel_layout_name 0: mono",
            ];
            if let Some(index) = expected.iter().position(|header| *header == row) {
                if headers[index] || records != 0 {
                    return Err(invalid("duplicate or late audit header"));
                }
                headers[index] = true;
            } else if row.starts_with("#tb ")
                || row.starts_with("#media_type ")
                || row.starts_with("#codec_id ")
                || row.starts_with("#sample_rate ")
                || row.starts_with("#channel_layout_name ")
            {
                return Err(invalid("unexpected audit stream or format"));
            }
            continue;
        }
        if !headers.iter().all(|seen| *seen) {
            return Err(invalid("incomplete audit headers"));
        }
        let fields: Vec<_> = row.split(',').map(str::trim).collect();
        if fields.len() != 6 || fields[0] != "0" {
            return Err(invalid("invalid audit packet"));
        }
        let dts: i128 = fields[1]
            .parse()
            .map_err(|_| invalid("invalid audit DTS"))?;
        let pts: i128 = fields[2]
            .parse()
            .map_err(|_| invalid("invalid audit PTS"))?;
        let duration: i128 = fields[3]
            .parse()
            .map_err(|_| invalid("invalid audit duration"))?;
        let size: i128 = fields[4]
            .parse()
            .map_err(|_| invalid("invalid audit packet size"))?;
        let checksum = fields[5]
            .strip_prefix("0x")
            .filter(|s| s.len() == 8)
            .ok_or_else(|| invalid("invalid audit checksum"))?;
        if u32::from_str_radix(checksum, 16).is_err()
            || dts != pts
            || duration <= 0
            || duration.checked_mul(2) != Some(size)
            || previous.is_some_and(|last| pts <= last)
        {
            return Err(invalid("invalid audit packet"));
        }
        let origin = *first.get_or_insert(pts);
        let expected = origin
            .checked_add(samples)
            .ok_or_else(|| invalid("audit PTS overflow"))?;
        let difference = pts
            .checked_sub(expected)
            .ok_or_else(|| invalid("audit PTS overflow"))?;
        if difference < -tolerance || difference > tolerance {
            return Err(invalid("source audio contains a gap, overlap, or clock drift"));
        }
        samples = samples
            .checked_add(duration)
            .ok_or_else(|| invalid("audio sample count overflow"))?;
        previous = Some(pts);
        records = records
            .checked_add(1)
            .ok_or_else(|| invalid("audit record count overflow"))?;
        if records.is_multiple_of(1024) {
            control.check()?;
        }
    }
    control.check()?;
    Ok(Audit {
        first_pts: first.ok_or_else(|| invalid("audit has no audio packets"))?,
        samples,
    })
}

fn wav_samples(
    path: &Path,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<i128> {
    crate::media::regular_file(path, "audio clock WAV")?;
    if std::fs::metadata(path)
        .map_err(|_| invalid("cannot inspect WAV"))?
        .len()
        >= MAX_WAV_BYTES - AUDIT_STOP_MARGIN
    {
        return Err(invalid("WAV reached its size limit and may be truncated"));
    }
    let output = process::run(
        Command::new(backend.ffprobe_path())
            .args([
                "-v",
                "error",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-show_format",
                "-show_streams",
                "-show_entries",
                "format=format_name:stream=codec_type,codec_name,sample_rate,channels,time_base,duration_ts",
                "-of",
                "json",
            ])
            .arg(path),
        Duration::from_secs(120),
        control,
    )
    .map_err(probe_error)?;
    let report: Value =
        crate::json::parse(&output.stdout).map_err(|_| invalid("invalid WAV probe"))?;
    let streams = report["streams"]
        .as_array()
        .ok_or_else(|| invalid("WAV stream is missing"))?;
    if report["format"]["format_name"] != "wav" || streams.len() != 1 {
        return Err(invalid("unexpected WAV container"));
    }
    let stream = &streams[0];
    if stream["codec_type"] != "audio"
        || stream["codec_name"] != "pcm_s16le"
        || stream["sample_rate"] != "16000"
        || stream["channels"].as_u64() != Some(1)
        || stream["time_base"] != "1/16000"
    {
        return Err(invalid("WAV is not 16 kHz mono signed PCM"));
    }
    let duration = &stream["duration_ts"];
    let samples = duration
        .as_i64()
        .map(i128::from)
        .or_else(|| duration.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("WAV sample count is unavailable"))?;
    Ok(samples)
}

pub(super) fn audit_audio_clock(
    wav: &Path,
    audit: &Path,
    stderr: &str,
    probe: &ClockProbe,
    backend: &FfmpegBackend,
    control: &mut dyn Control,
) -> Result<AudioClock> {
    let first_input = first_input_pts(stderr)?;
    let observed = read_audit(audit, probe.tolerance_ticks, control)?;
    let delta = observed
        .first_pts
        .checked_sub(first_input)
        .ok_or_else(|| invalid("first PTS overflow"))?;
    if delta < -probe.tolerance_ticks || delta > probe.tolerance_ticks {
        return Err(invalid("resampled audio starts on an ambiguous clock"));
    }
    if wav_samples(wav, backend, control)? != observed.samples {
        return Err(invalid("WAV and audit sample counts differ"));
    }
    let origin = match probe.origin_ticks {
        Some(origin) => origin,
        None if probe.zero_origin_wav && first_input == 0 && observed.first_pts == 0 => 0,
        None => return Err(invalid("container origin is ambiguous")),
    };
    Ok(AudioClock {
        offset_ticks: relative_offset(observed.first_pts, origin)?,
        samples: observed.samples,
    })
}

fn relative_offset(first_pts: i128, origin: i128) -> Result<i128> {
    let offset = first_pts
        .checked_sub(origin)
        .ok_or_else(|| invalid("source offset overflow"))?;
    if offset < 0 {
        return Err(invalid("source audio starts before the container origin"));
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HEADERS: &str = "#tb 0: 1/16000\n#media_type 0: audio\n#codec_id 0: pcm_s16le\n#sample_rate 0: 16000\n#channel_layout_name 0: mono\n";

    #[test]
    fn decimal_origin_and_source_tolerance_are_checked() {
        assert_eq!(decimal_ticks("5.125").unwrap(), 82_000);
        assert_eq!(decimal_ticks("-0.00003125").unwrap(), -1);
        assert_eq!(decimal_ticks("0.00003125").unwrap(), 1);
        assert!(decimal_ticks("NaN").is_err());
        assert!(decimal_ticks("1.1234567890").is_err());
        assert!(decimal_ticks("999999999999999999999999999999999999999").is_err());
        assert_eq!(tolerance_ticks("1/1000").unwrap(), 17);
        assert_eq!(tolerance_ticks("1/48000").unwrap(), 2);
        assert!(tolerance_ticks("1/999").is_err());
        assert!(tolerance_ticks("0/1").is_err());
    }

    #[test]
    fn no_origin_is_only_allowed_for_single_stream_wav() {
        let wav = json!({"format":{"format_name":"wav"},"streams":[
            {"index":0,"codec_type":"audio","time_base":"1/16000"}
        ]});
        let parsed = probe_value(&wav).unwrap();
        assert!(parsed.zero_origin_wav);
        assert!(parsed.origin_ticks.is_none());
        let mut ambiguous = wav.clone();
        ambiguous["format"]["format_name"] = json!("matroska");
        assert!(probe_value(&ambiguous).is_err());
        ambiguous = wav;
        ambiguous["streams"].as_array_mut().unwrap().push(
            json!({"index":1,"codec_type":"video","time_base":"1/1000"}),
        );
        assert!(probe_value(&ambiguous).is_err());
    }

    fn audit_file(body: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(body.as_bytes()).unwrap();
        file
    }

    #[test]
    fn framecrc_checks_cumulative_clock_and_packet_shape() {
        let content = format!("{HEADERS}0, 1000, 1000, 100, 200, 0x00000000\n0, 1102, 1102, 100, 200, 0x00000000\n");
        let file = audit_file(&content);
        let audit = read_audit(file.path(), 2, &mut crate::process::Uncontrolled).unwrap();
        assert_eq!((audit.first_pts, audit.samples), (1000, 200));
        let drift = format!("{HEADERS}0, 1000, 1000, 100, 200, 0x00000000\n0, 1103, 1103, 100, 200, 0x00000000\n");
        assert!(read_audit(audit_file(&drift).path(), 2, &mut crate::process::Uncontrolled).is_err());
        let cumulative = format!("{HEADERS}0, 1000, 1000, 100, 200, 0x00000000\n0, 1102, 1102, 100, 200, 0x00000000\n0, 1204, 1204, 100, 200, 0x00000000\n");
        assert!(read_audit(audit_file(&cumulative).path(), 2, &mut crate::process::Uncontrolled).is_err());
        let bad_size = format!("{HEADERS}0, 1000, 1000, 100, 201, 0x00000000\n");
        assert!(read_audit(audit_file(&bad_size).path(), 2, &mut crate::process::Uncontrolled).is_err());
        let missing_header = "0, 1000, 1000, 100, 200, 0x00000000\n";
        assert!(read_audit(audit_file(missing_header).path(), 2, &mut crate::process::Uncontrolled).is_err());
    }

    #[test]
    fn framecrc_streams_more_than_the_subprocess_stdout_limit() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(HEADERS.as_bytes()).unwrap();
        let mut pts = 0_i128;
        for _ in 0..170_000 {
            writeln!(file, "0, {pts:12}, {pts:12}, {:8}, {:8}, 0x00000000", 341, 682).unwrap();
            pts += 341;
        }
        assert!(file.as_file().metadata().unwrap().len() > 8 * 1024 * 1024);
        let parsed = read_audit(file.path(), 2, &mut crate::process::Uncontrolled).unwrap();
        assert_eq!(parsed.samples, pts);
    }

    #[test]
    fn capped_audit_is_rejected_as_possibly_truncated() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file()
            .set_len(MAX_AUDIT_BYTES - AUDIT_STOP_MARGIN)
            .unwrap();
        let error = read_audit(file.path(), 2, &mut crate::process::Uncontrolled)
            .err()
            .unwrap();
        assert!(error.to_string().contains("may be truncated"));
    }

    #[test]
    fn capped_wav_is_rejected_as_possibly_truncated() {
        // Size validation occurs before launching ffprobe.
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file()
            .set_len(MAX_WAV_BYTES - AUDIT_STOP_MARGIN)
            .unwrap();
        let backend = crate::media::backend("unused".into(), "unused".into());
        let error = wav_samples(file.path(), &backend, &mut crate::process::Uncontrolled)
            .unwrap_err();
        assert!(error.to_string().contains("may be truncated"));
    }

    #[test]
    fn source_audio_must_not_start_before_origin() {
        assert_eq!(relative_offset(16000, 0).unwrap(), 16000);
        assert_eq!(relative_offset(0, 0).unwrap(), 0);
        assert!(relative_offset(-1, 0).is_err());
        assert!(relative_offset(i128::MIN, 1).is_err());
    }

    #[test]
    fn first_marker_must_be_present_unique_and_numeric() {
        let marker = "[ashowinfo@asr_clock @ 0x1] n:0 pts:123 pts_time:0.0076875\n";
        assert_eq!(first_input_pts(marker).unwrap(), 123);
        assert!(first_input_pts("no first marker").is_err());
        assert!(first_input_pts(&format!("{marker}{marker}")).is_err());
        assert!(first_input_pts("[ashowinfo@asr_clock @ 0x1] n:0 pts:NOPTS\n").is_err());
    }

    #[test]
    fn framecrc_checks_cancellation_while_streaming() {
        struct Cancel(usize);
        impl Control for Cancel {
            fn check(&mut self) -> Result<()> {
                self.0 += 1;
                if self.0 > 1 {
                    Err(Error::Cancelled)
                } else {
                    Ok(())
                }
            }
        }
        let mut content = String::from(HEADERS);
        for pts in 0..2048 {
            content.push_str(&format!("0, {pts}, {pts}, 1, 2, 0x00000000\n"));
        }
        let file = audit_file(&content);
        assert!(matches!(
            read_audit(file.path(), 2, &mut Cancel(0)),
            Err(Error::Cancelled)
        ));
    }
}
