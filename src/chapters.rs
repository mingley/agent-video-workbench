//! Read-only, explicitly selected chapter drafts for a frozen render.
use crate::{Error, Result, compare, store::Store};
use agentcut_core::{RationalRate, RationalTime, RoundingMode, Sequence};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

const YOUTUBE_GUIDANCE: &str = "https://support.google.com/youtube/answer/9884579?hl=en";
const MAX_CHAPTERS: usize = 1000;
const MAX_TITLE_BYTES: usize = 512;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterSelection {
    pub marker_id: String,
    pub title: String,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn parse_integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

// FFprobe time_base is seconds per tick, while RationalRate is ticks per second.
pub(crate) fn duration_from_verified_stream(
    sequence: &Sequence,
    verification: &Value,
) -> Result<RationalTime> {
    let frames = verification["expectedFrames"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("render has no verified output frame count"))?;
    if verification["decoded"] != true {
        return Err(invalid("render does not record a completed video decode"));
    }
    let streams = verification["probe"]["streams"]
        .as_array()
        .ok_or_else(|| invalid("render has no recorded video probe"))?;
    let mut videos = streams.iter().filter(|s| s["codec_type"] == "video");
    let video = videos
        .next()
        .ok_or_else(|| invalid("render has no recorded video stream"))?;
    if videos.next().is_some() {
        return Err(invalid("render has multiple video streams"));
    }
    let rate = sequence.frame_rate.normalized()?;
    let expected_rate = format!("{}/{}", rate.numerator, rate.denominator);
    if parse_integer(&video["nb_read_frames"]) != Some(frames)
        || video["avg_frame_rate"] != expected_rate
        || parse_integer(&video["start_pts"]) != Some(0)
    {
        return Err(invalid(
            "recorded video frame count, rate or start is uncertain",
        ));
    }
    let duration_ticks = parse_integer(&video["duration_ts"])
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("recorded video stream duration is unavailable"))?;
    let time_base = video["time_base"]
        .as_str()
        .ok_or_else(|| invalid("recorded video stream time base is unavailable"))?;
    let (numerator, denominator) = time_base
        .split_once('/')
        .ok_or_else(|| invalid("recorded video stream time base is invalid"))?;
    let numerator: u32 = numerator
        .parse()
        .map_err(|_| invalid("recorded video stream time base is invalid"))?;
    let denominator: u32 = denominator
        .parse()
        .map_err(|_| invalid("recorded video stream time base is invalid"))?;
    let stream_duration =
        RationalTime::new(duration_ticks, RationalRate::new(denominator, numerator)?);
    let frame_duration = RationalTime::new(frames, rate);
    if !stream_duration.same_instant(frame_duration)? {
        return Err(invalid(
            "recorded stream duration disagrees with verified video frames",
        ));
    }
    Ok(stream_duration)
}

fn safe_title(title: &str) -> bool {
    !title.trim().is_empty()
        && title.len() <= MAX_TITLE_BYTES
        && !title.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{2028}' | '\u{2029}' | '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

fn timestamp(seconds: i64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Draft chapter timestamps from selected markers only, without changing the project.
///
/// User-provided titles, not marker names or comments, are published in the draft.
/// Whole-second timestamps round down; the exact marker positions are retained.
pub fn draft(root: &Path, job_id: &str, selections: &[ChapterSelection]) -> Result<Value> {
    if selections.len() > MAX_CHAPTERS {
        return Err(invalid(format!(
            "at most {MAX_CHAPTERS} chapters may be selected"
        )));
    }
    let root = root.canonicalize()?;
    let store = Store::open(&root)?;
    let frozen = compare::artifact(&store, &root, job_id)?;
    if frozen.identity["sequenceBinding"] != "job-input" {
        return Err(invalid(
            "render does not bind the sequence in a queued job; re-render with render-start",
        ));
    }
    let sequence = frozen.snapshot.require_sequence(&frozen.sequence_id)?;
    let duration = duration_from_verified_stream(sequence, &frozen.manifest["verification"])?;
    let zero = RationalTime::zero(RationalRate::frames(1)?);
    let mut chapters = Vec::with_capacity(selections.len());
    let mut seconds_list = Vec::with_capacity(selections.len());
    let mut text = String::new();
    let mut duplicate_ids = HashSet::new();
    let mut any_duplicate = false;
    let mut any_rounded = false;
    for selection in selections {
        if !safe_title(&selection.title) {
            return Err(invalid(format!(
                "chapter title must contain 1..={MAX_TITLE_BYTES} bytes of single-line text without control or bidirectional formatting characters"
            )));
        }
        let marker = sequence
            .markers
            .iter()
            .find(|m| m.id == selection.marker_id)
            .ok_or_else(|| {
                invalid(format!(
                    "marker {} is absent from the frozen sequence",
                    selection.marker_id
                ))
            })?;
        if marker.time.checked_cmp(zero)?.is_lt() || marker.time.checked_cmp(duration)?.is_ge() {
            return Err(invalid(format!(
                "marker {} is outside the frozen video stream",
                selection.marker_id
            )));
        }
        if !duplicate_ids.insert(&selection.marker_id) {
            any_duplicate = true;
        }
        let seconds = marker
            .time
            .rescale_to(RationalRate::frames(1)?, RoundingMode::Floor)?
            .value;
        let rounded = !marker
            .time
            .same_instant(RationalTime::new(seconds, RationalRate::frames(1)?))?;
        any_rounded |= rounded;
        let formatted = timestamp(seconds);
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&formatted);
        text.push(' ');
        text.push_str(&selection.title);
        seconds_list.push(seconds);
        chapters.push(json!({"markerId":selection.marker_id,"title":selection.title,
            "exactTime":marker.time,"timestampSeconds":seconds,"timestamp":formatted,"roundedDown":rounded}));
    }
    let mut issues = Vec::new();
    if chapters.len() < 3 {
        issues.push(json!({"code":"at-least-three","message":"At least three chapter timestamps are required."}));
    }
    if seconds_list.first().copied() != Some(0) {
        issues.push(json!({"code":"first-at-zero","message":"The first published timestamp must be 00:00."}));
    }
    if any_duplicate {
        issues.push(
            json!({"code":"duplicate-marker","message":"A marker was selected more than once."}),
        );
    }
    if seconds_list.windows(2).any(|pair| pair[0] >= pair[1]) {
        issues.push(json!({"code":"not-strictly-increasing","message":"Published timestamps must be strictly increasing; selection order or whole-second rounding may cause a collision."}));
    }
    if seconds_list
        .windows(2)
        .any(|pair| pair[1].saturating_sub(pair[0]) < 10)
    {
        issues.push(json!({"code":"chapter-under-ten-seconds","message":"Each chapter must be at least ten seconds; check adjacent published timestamps."}));
    }
    if let Some(last) = seconds_list.last() {
        let minimum_end = last
            .checked_add(10)
            .ok_or_else(|| invalid("chapter timestamp is too large"))?;
        if duration
            .checked_cmp(RationalTime::new(minimum_end, RationalRate::frames(1)?))?
            .is_lt()
        {
            issues.push(json!({"code":"last-chapter-under-ten-seconds","message":"The last published timestamp leaves less than ten seconds in the verified video stream."}));
        }
    }
    Ok(
        json!({"status":"draft","render":frozen.identity,"duration":duration,
        "timingEvidence":"Recorded decoded video stream duration and frame count; the render manifest is not independently authenticated",
        "rounding":{"mode":"floor-to-whole-second","anyRounded":any_rounded,
            "note":"Published timestamps may be up to but less than one second earlier than the selected markers."},
        "chapters":chapters,"text":text,
        "youtube":{"documentedTimestampChecks":if issues.is_empty() { "pass" } else { "fail" },
            "diagnostics":issues,"accountAndVideoEligibility":"unknown","source":YOUTUBE_GUIDANCE,
            "note":"These checks do not establish eligibility, publication or platform acceptance; recheck current platform guidance."}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_fractional_rate_duration_exactly() {
        let rate = RationalRate::new(30_000, 1001).unwrap();
        let sequence = Sequence::new("test", "test", 640, 360, rate, 48_000);
        let verification = json!({"decoded":true,"expectedFrames":300,"probe":{"streams":[{
            "codec_type":"video","nb_read_frames":"300","avg_frame_rate":"30000/1001",
            "start_pts":0,"duration_ts":300300,"time_base":"1/30000"}]}});
        let duration = duration_from_verified_stream(&sequence, &verification).unwrap();
        assert!(
            duration
                .same_instant(RationalTime::new(1001, RationalRate::frames(100).unwrap()))
                .unwrap()
        );
        let mut malformed = verification;
        malformed["probe"]["streams"][0]["duration_ts"] = json!(300000);
        assert!(duration_from_verified_stream(&sequence, &malformed).is_err());
    }
}
