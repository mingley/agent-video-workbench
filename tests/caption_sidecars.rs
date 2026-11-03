use agent_video_workbench::{delivery, process::Uncontrolled};
use agentcut_core::{OperationBatch, Project, RationalRate, RationalTime};
use agentcut_render::FfmpegBackend;
use serde_json::{Value, json};
use std::path::Path;

fn caption_project(start: Value, duration: Value, text: &str) -> Project {
    let project = Project::new("Sidecars", 360, 640, RationalRate::frames(30).unwrap());
    let batch: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0", "projectId":project.project_id, "baseRevision":0,
        "idempotencyKey":"caption-fixture", "operations":[
            {"id":"track", "op":"track.add", "params":{"id":"caption_track","sequence":"seq_main","type":"caption"}},
            {"id":"cue", "op":"caption.add", "params":{"id":"cue_1","track":"caption_track","at":start,"duration":duration,"text":text}}
        ]
    })).unwrap();
    agentcut_core::apply_batch(&project, &batch)
        .unwrap()
        .project
}

fn time(value: i64, numerator: u32, denominator: u32) -> Value {
    json!({"value":value,"rate":{"numerator":numerator,"denominator":denominator}})
}

#[test]
fn exports_normal_text_and_escapes_webvtt_markup_and_cue_delimiters() {
    let project = caption_project(
        time(0, 30, 1),
        time(30, 30, 1),
        "A <tag> & -->\n\nsecond line",
    );
    let (srt, vtt, mapping) =
        delivery::sidecars(project.require_sequence("seq_main").unwrap()).unwrap();
    assert_eq!(
        srt,
        "1\n00:00:00,000 --> 00:00:01,000\nA <tag> & -->\nsecond line\n\n"
    );
    assert_eq!(
        vtt,
        "WEBVTT\n\n1\n00:00:00.000 --> 00:00:01.000\nA &lt;tag&gt; &amp; --&gt;\nsecond line\n\n"
    );
    assert_eq!(mapping["cues"][0]["startMs"], 0);
    assert_eq!(mapping["cues"][0]["endMs"], 1000);
}

#[test]
fn keeps_nearest_millisecond_ties_up_and_fractional_frame_rates() {
    // Half-millisecond ties stay on the same side as the previous positive
    // floating-point rounding rule, including for common broadcast rates.
    for (rate, start, duration, expected_start, expected_end) in [
        (2000, 1, 2, 1, 2),
        (24000, 12, 24, 501, 1502),
        (30000, 15, 30, 501, 1502),
        (60000, 30, 60, 501, 1502),
    ] {
        let denominator = if rate == 2000 { 1 } else { 1001 };
        let project = caption_project(
            time(start, rate, denominator),
            time(duration, rate, denominator),
            "Speech",
        );
        let (_, _, mapping) =
            delivery::sidecars(project.require_sequence("seq_main").unwrap()).unwrap();
        assert_eq!(mapping["cues"][0]["startMs"], expected_start);
        assert_eq!(mapping["cues"][0]["endMs"], expected_end);
    }
}

#[test]
fn preserves_millisecond_boundaries_beyond_float_integer_precision() {
    let start = 9_007_199_254_740_993_i64;
    let project = caption_project(time(start, 1000, 1), time(1, 1000, 1), "Speech");
    let (_, _, mapping) =
        delivery::sidecars(project.require_sequence("seq_main").unwrap()).unwrap();
    assert_eq!(mapping["cues"][0]["startMs"], start);
    assert_eq!(mapping["cues"][0]["endMs"], start + 1);
}

#[test]
fn rejects_short_cues_that_collapse_without_writing_either_sidecar() {
    // This positive-duration cue is accepted by the core timeline, but both
    // boundaries round to the same millisecond and would be invalid WebVTT.
    let project = caption_project(time(0, 10000, 1), time(4, 10000, 1), "Very short");
    let sequence = project.require_sequence("seq_main").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new("missing-ffmpeg", "missing-ffprobe");
    let error = delivery::extras(
        Path::new("missing-video"),
        temp.path(),
        sequence,
        &backend,
        &mut Uncontrolled,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("cue_1"));
    assert!(error.contains("no duration after sidecar millisecond rounding"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);

    let visible = caption_project(time(0, 10000, 1), time(6, 10000, 1), "Visible");
    let (_, _, mapping) =
        delivery::sidecars(visible.require_sequence("seq_main").unwrap()).unwrap();
    assert_eq!(mapping["cues"][0]["startMs"], 0);
    assert_eq!(mapping["cues"][0]["endMs"], 1);
}

#[test]
fn invisible_short_cues_do_not_block_sidecars() {
    let mut project = caption_project(time(0, 10000, 1), time(4, 10000, 1), "Hidden");
    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .enabled = false;
    let (srt, vtt, mapping) =
        delivery::sidecars(project.require_sequence("seq_main").unwrap()).unwrap();
    assert!(srt.is_empty());
    assert_eq!(vtt, "WEBVTT\n\n");
    assert!(mapping["cues"].as_array().unwrap().is_empty());
}

#[test]
fn rejects_negative_invalid_and_overflowing_times_with_the_caption_id() {
    let mut project = caption_project(time(0, 30, 1), time(30, 30, 1), "Speech");
    let cue = &mut project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .items[0];
    cue.start = RationalTime::new(-1, RationalRate::frames(30).unwrap());
    let error = delivery::sidecars(project.require_sequence("seq_main").unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cue_1") && error.contains("invalid sidecar timestamp"));

    let cue = &mut project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .items[0];
    cue.start = RationalTime::new(0, RationalRate::frames(30).unwrap());
    cue.duration.value = 0;
    let error = delivery::sidecars(project.require_sequence("seq_main").unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cue_1") && error.contains("invalid sidecar duration"));

    let cue = &mut project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .items[0];
    cue.duration.value = 1;
    cue.duration.rate.numerator = 0;
    let error = delivery::sidecars(project.require_sequence("seq_main").unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cue_1") && error.contains("invalid sidecar duration"));

    let cue = &mut project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .items[0];
    cue.start = RationalTime::new(i64::MAX - 1, RationalRate::frames(1).unwrap());
    cue.duration = RationalTime::new(1, RationalRate::frames(1).unwrap());
    let error = delivery::sidecars(project.require_sequence("seq_main").unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cue_1") && error.contains("timestamp range"));

    let cue = &mut project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("caption_track")
        .unwrap()
        .items[0];
    let rate = RationalRate::frames(1_000_000_000).unwrap();
    cue.start = RationalTime::new(i64::MAX, rate);
    cue.duration = RationalTime::new(1, rate);
    let error = delivery::sidecars(project.require_sequence("seq_main").unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("cue_1") && error.contains("invalid sidecar interval"));
}
