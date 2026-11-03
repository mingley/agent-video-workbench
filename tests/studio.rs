use agent_video_workbench::{store::Store, studio::Edit};
use agentcut_core::OperationBatch;
use serde_json::json;

fn verified_timing(sha256: String) -> serde_json::Value {
    json!({"sha256":sha256,"decoded":true,"expectedFrames":90,
        "probe":{"streams":[{"codec_type":"video","nb_read_frames":"90", "avg_frame_rate":"30/1",
            "start_pts":0,"duration_ts":90,"time_base":"1/30"}]}})
}

fn seed(store: &mut Store) {
    let p = store.project().unwrap();
    let mut ops = vec![
        json!({"id":"source","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video","fingerprint":{"strategy":"sha256","sizeBytes":0,"modifiedUnixNs":null,"sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}}}),
    ];
    for (id, at, source) in [("a", 0, 0), ("b", 30, 30), ("c", 60, 60)] {
        let time = |value| json!({"value":value,"rate":{"numerator":30,"denominator":1}});
        ops.push(json!({"id":id,"op":"clip.add","params":{"id":id,"asset":"source","track":"track_v1","at":time(at),"sourceIn":time(source),"duration":time(30)}}));
    }
    let b:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":0,"idempotencyKey":"seed-studio","operations":ops})).unwrap();
    store.apply(&b, false).unwrap();
}

#[test]
fn review_range_uses_verified_stream_duration_and_preserves_replay_without_timing() {
    use agent_video_workbench::media;
    use std::fs;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Exact review bounds").unwrap();
    let project = store.project().unwrap();
    let rate = json!({"numerator":30000,"denominator":1001});
    let batch: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":0,
        "idempotencyKey":"range-timing-setup","operations":[
            {"id":"source","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video"}},
            {"id":"seq","op":"sequence.add","params":{"id":"ntsc","name":"NTSC","width":360,"height":640,"frameRate":rate}},
            {"id":"track","op":"track.add","params":{"id":"ntsc_video","sequence":"ntsc","type":"video"}},
            {"id":"clip","op":"clip.add","params":{"id":"take","asset":"source","track":"ntsc_video",
                "at":{"value":0,"rate":rate},"sourceIn":{"value":0,"rate":rate},
                "duration":{"value":45,"rate":rate},"fit":"contain"}},
            {"id":"duration","op":"sequence.set","target":"ntsc",
                "params":{"duration":{"value":46,"rate":rate}}}
        ]
    })).unwrap();
    store.apply(&batch, false).unwrap();
    let artifact = root.join("artifact.bin");
    let manifest = root.join("artifact.json");
    fs::write(&artifact, b"verified fixture").unwrap();
    let mut recorded = json!({"snapshot":store.project().unwrap(),"sequenceId":"ntsc",
        "verification":{"sha256":media::hash_file(&artifact).unwrap(),"decoded":true,"expectedFrames":46,
            "probe":{"streams":[{"codec_type":"video","nb_read_frames":"46", "avg_frame_rate":"30000/1001",
                "start_pts":0,"duration_ts":46046,"time_base":"1/30000"}]}}});
    fs::write(&manifest, serde_json::to_vec(&recorded).unwrap()).unwrap();
    store.start_job("reviewed-job", 1).unwrap();
    store
        .finish_job(
            "reviewed-job",
            &json!({"path":artifact,"manifest":manifest}),
        )
        .unwrap();
    let review = |id: &str, end_ms| Edit::ReviewAdd {
        id: id.into(),
        job_id: "reviewed-job".into(),
        at_ms: 1500,
        end_ms,
        actor: "creator".into(),
        text: "Review the ending".into(),
    };

    // 46 × 1001 / 30000 seconds is 1534.866...ms; 1534 is within
    // the video even though the clip content itself ended at 1501.5ms.
    let edit = review("inside", 1534);
    let accepted = store.studio(&edit, 1, "in-bounds", false).unwrap();
    assert_eq!(store.project().unwrap().revision, 2);
    let error = store
        .studio(&review("outside", 1535), 2, "out-of-bounds", false)
        .unwrap_err();
    assert!(error.to_string().contains("beyond artifact duration"));
    assert_eq!(store.project().unwrap().revision, 2);

    // Removing recorded evidence prevents a new range from being asserted,
    // but a previously committed idempotent outcome still replays unchanged.
    recorded["verification"]["probe"] = serde_json::Value::Null;
    fs::write(&manifest, serde_json::to_vec(&recorded).unwrap()).unwrap();
    let error = store
        .studio(&review("unknown", 1530), 2, "missing-timing", false)
        .unwrap_err();
    assert!(error.to_string().contains("re-render"));
    assert_eq!(
        store.studio(&edit, 1, "in-bounds", false).unwrap(),
        accepted
    );
    assert_eq!(store.project().unwrap().revision, 2);
    assert_eq!(
        store.project().unwrap().extensions["avw.reviews"][0]["endMs"],
        1534
    );
}

#[test]
fn selective_restore_preserves_a_second_omission_and_later_edits_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    let mut store = Store::create(&root, "Before").unwrap();
    seed(&mut store);
    store
        .studio(
            &Edit::Omit {
                id: "first".into(),
                item_id: "a".into(),
                reason: "first sentence".into(),
            },
            1,
            "omit-first",
            false,
        )
        .unwrap();
    store
        .studio(
            &Edit::Omit {
                id: "second".into(),
                item_id: "b".into(),
                reason: "second sentence".into(),
            },
            2,
            "omit-second",
            false,
        )
        .unwrap();
    let p = store.project().unwrap();
    let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":3,"idempotencyKey":"later-style","operations":[{"id":"crop","op":"item.set","target":"c","params":{"property":"video.crop.left","value":0.1}},{"id":"rename","op":"project.rename","params":{"name":"Later"}}]})).unwrap();
    store.apply(&batch, false).unwrap();
    drop(store);
    let mut store = Store::open(&root).unwrap();
    let edit = Edit::RestoreOmission {
        id: "first".into(),
        at_ms: 0,
    };
    let result = store.studio(&edit, 4, "restore-first", false).unwrap();
    assert_eq!(
        store.studio(&edit, 4, "restore-first", false).unwrap(),
        result
    );
    let p = store.project().unwrap();
    assert_eq!(p.name, "Later");
    assert!(p.find_item("b").is_none());
    let c = p.require_item("c").unwrap();
    assert_eq!(c.start.value, 30);
    assert_eq!(c.payload.as_clip().unwrap().video.crop.left, 0.1);
    assert!(p.find_item("restored_first_a").is_some());
    assert!(store.studio(&edit, 5, "duplicate-restore", false).is_err());
}
#[test]
fn variants_have_independent_entity_ids_and_frozen_source_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("p"), "Variants").unwrap();
    seed(&mut store);
    let edit = Edit::Variant {
        source_revision: 1,
        sequence: "seq_main".into(),
        id: "square".into(),
        name: "Square".into(),
        width: 360,
        height: 360,
        language: None,
        captions: Default::default(),
        translation_provenance: None,
    };
    store.studio(&edit, 1, "create-square", false).unwrap();
    store
        .studio(
            &Edit::Omit {
                id: "remove".into(),
                item_id: "a".into(),
                reason: "shorter".into(),
            },
            2,
            "shorten-parent",
            false,
        )
        .unwrap();
    let p = store.project().unwrap();
    assert!(p.find_item("a").is_none());
    assert!(p.find_item("square_a").is_some());
    assert_eq!(p.require_sequence("square").unwrap().canvas.height, 360);
}
#[test]
fn unsafe_ripple_across_a_continuous_layer_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("p"), "Layers").unwrap();
    seed(&mut store);
    store
        .studio(
            &Edit::Layer {
                sequence: "seq_main".into(),
                id: "broll".into(),
                asset_id: "source".into(),
                at_ms: 0,
                source_start_ms: 0,
                duration_ms: 2000,
                fit: "cover".into(),
            },
            1,
            "add-broll",
            false,
        )
        .unwrap();
    assert!(
        store
            .studio(
                &Edit::Omit {
                    id: "first".into(),
                    item_id: "a".into(),
                    reason: "sentence".into()
                },
                2,
                "unsafe-omit",
                false
            )
            .is_err()
    );
    assert_eq!(store.project().unwrap().revision, 2);
    assert!(store.project().unwrap().find_item("a").is_some());
    store
        .studio(
            &Edit::ReplaceLayer {
                item_id: "broll".into(),
                asset_id: "source".into(),
                source_start_ms: 1000,
            },
            2,
            "replace-broll",
            false,
        )
        .unwrap();
    assert!(
        !store
            .project()
            .unwrap()
            .require_item("broll")
            .unwrap()
            .payload
            .as_clip()
            .unwrap()
            .audio
            .enabled
    );
}

#[test]
fn otio_roundtrip_keeps_supported_source_order_and_exact_times() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("p"), "Interchange").unwrap();
    seed(&mut store);
    let export =
        agent_video_workbench::interchange::export(&store.project().unwrap(), "seq_main").unwrap();
    store
        .interchange_import(&export["document"], "roundtrip", 1, "otio-roundtrip", false)
        .unwrap();
    let p = store.project().unwrap();
    let original = p.require_sequence("seq_main").unwrap();
    let copied = p.require_sequence("roundtrip").unwrap();
    for (a, b) in original.tracks[0].items.iter().zip(&copied.tracks[0].items) {
        assert_eq!(a.start, b.start);
        assert_eq!(a.duration, b.duration);
        assert_eq!(a.source_range(), b.source_range());
        assert_eq!(
            a.payload.as_clip().unwrap().asset_id,
            b.payload.as_clip().unwrap().asset_id
        );
    }
}

#[test]
fn transcript_selection_requires_explicit_review_when_corrections_cannot_align() {
    use agent_video_workbench::workflow::{Cue, Transcript};
    use std::collections::BTreeMap;
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("p"), "transcripts").unwrap();
    seed(&mut store);
    let transcript = |end| Transcript {
        asset_id: "source".into(),
        language: "en".into(),
        provider: "test".into(),
        cues: vec![Cue {
            id: "word".into(),
            start_ms: 0,
            end_ms: end,
            text: "wrong".into(),
        }],
    };
    store
        .transcript_import(transcript(1000), 1, "original-transcript")
        .unwrap();
    store
        .studio(
            &Edit::TranscriptCorrect {
                asset_id: "source".into(),
                cues: BTreeMap::from([("word".into(), "Mingley".into())]),
                sequences: vec![],
            },
            2,
            "correct-transcript",
            false,
        )
        .unwrap();
    store
        .transcript_import(transcript(900), 3, "new-transcript")
        .unwrap();
    let project = store.project().unwrap();
    let version = project
        .extensions
        .iter()
        .find(|(k, v)| k.starts_with("avw.analysis.transcript.") && v["cues"][0]["endMs"] == 900)
        .unwrap()
        .0
        .strip_prefix("avw.analysis.transcript.")
        .unwrap()
        .to_owned();
    let mut edit = Edit::TranscriptSelect {
        asset_id: "source".into(),
        analysis_version: version,
        preserve_corrections: true,
        sequences: vec![],
    };
    assert!(
        store
            .studio(&edit, 4, "unaligned-selection", false)
            .is_err()
    );
    assert_eq!(store.project().unwrap().revision, 4);
    if let Edit::TranscriptSelect {
        preserve_corrections,
        ..
    } = &mut edit
    {
        *preserve_corrections = false;
    }
    store.studio(&edit, 4, "reviewed-selection", false).unwrap();
    let project = store.project().unwrap();
    assert_eq!(
        project.extensions["avw.transcripts.v1"][0]["cues"][0]["text"],
        "wrong"
    );
    assert_eq!(
        project.extensions["avw.corrections.source"][0]["changes"][0]["correctedText"],
        "Mingley"
    );
}

#[test]
fn interval_feedback_maps_across_cuts_without_hiding_removed_or_repeated_material() {
    use agent_video_workbench::{media, studio};
    use agentcut_core::{RationalRate, RationalTime, TrackType};
    use std::fs;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Feedback ranges").unwrap();
    seed(&mut store);
    // A tiny local artifact and its frozen snapshot exercise the same hash and
    // job boundary as ReviewAdd, without requiring a media backend.
    let artifact = root.join("reviewed.bin");
    let manifest = root.join("reviewed.json");
    fs::write(&artifact, b"verified review fixture").unwrap();
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "snapshot":store.project().unwrap(),"sequenceId":"seq_main",
            "verification":verified_timing(media::hash_file(&artifact).unwrap())
        }))
        .unwrap(),
    )
    .unwrap();
    store.start_job("reviewed-job", 1).unwrap();
    store
        .finish_job(
            "reviewed-job",
            &json!({"path":artifact,"manifest":manifest}),
        )
        .unwrap();
    store
        .studio(
            &Edit::ReviewAdd {
                id: "note".into(),
                job_id: "reviewed-job".into(),
                at_ms: 501,
                end_ms: 2501,
                actor: "creator".into(),
                text: "Keep all three moments".into(),
            },
            1,
            "add-range-review",
            false,
        )
        .unwrap();
    let project = store.project().unwrap();
    let review = &project.extensions["avw.reviews"][0];
    assert_eq!(review["sourceRangeCoverage"], "complete");
    assert_eq!(review["sourceRanges"].as_array().unwrap().len(), 3);
    let source_start: RationalTime =
        serde_json::from_value(review["sourceRanges"][0]["start"].clone()).unwrap();
    assert!(
        source_start
            .same_instant(RationalTime::new(501, RationalRate::frames(1000).unwrap()))
            .unwrap()
    );
    let full = studio::remap_reviews(&project, "seq_main").unwrap();
    assert_eq!(full["items"][0]["mapping"], "resolved");
    assert_eq!(full["items"][0]["mappingBasis"], "source-range");
    assert_eq!(full["items"][0]["placements"][0]["atMs"], 501);

    // Splitting a source range into adjacent clips still covers it completely.
    let mut split = project.clone();
    let track = &mut split.require_sequence_mut("seq_main").unwrap().tracks[0];
    let mut second = track.items[1].clone();
    track.items[1].duration.value = 15;
    track.items[1]
        .payload
        .as_clip_mut()
        .unwrap()
        .source_range
        .duration
        .value = 15;
    second.id = "b_second".into();
    second.start.value = 45;
    second.duration.value = 15;
    let source = &mut second.payload.as_clip_mut().unwrap().source_range;
    source.start.value = 45;
    source.duration.value = 15;
    track.items.push(second);
    let split_report = studio::remap_reviews(&split, "seq_main").unwrap();
    assert_eq!(split_report["items"][0]["mapping"], "resolved");
    assert_eq!(
        split_report["items"][0]["rangeMappings"][1]["placements"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(split_report["items"][0]["arrangement"], "preserved");

    // Every source range can survive while the reviewed interval is no
    // longer continuous or has changed order; never report it as unchanged.
    let mut separated = project.clone();
    let track = &mut separated.require_sequence_mut("seq_main").unwrap().tracks[0];
    track.items[1].start.value += 30;
    track.items[2].start.value += 30;
    let report = studio::remap_reviews(&separated, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "rearranged");
    assert_eq!(report["items"][0]["arrangement"], "changed");
    assert!(
        report["items"][0]["rangeMappings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["mapping"] == "resolved")
    );
    let mut reordered = project.clone();
    let track = &mut reordered.require_sequence_mut("seq_main").unwrap().tracks[0];
    track.items[0].start.value = 60;
    track.items[1].start.value = 0;
    track.items[2].start.value = 30;
    assert_eq!(
        studio::remap_reviews(&reordered, "seq_main").unwrap()["items"][0]["mapping"],
        "rearranged"
    );
    let mut shifted = project.clone();
    for item in &mut shifted.require_sequence_mut("seq_main").unwrap().tracks[0].items {
        item.start.value += 30;
    }
    let report = studio::remap_reviews(&shifted, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "resolved");
    assert_eq!(report["items"][0]["arrangement"], "preserved");

    // An inactive solo track must not suppress the active video track.
    let mut inactive_solo = project.clone();
    let sequence = inactive_solo.require_sequence_mut("seq_main").unwrap();
    let mut extra = sequence.tracks[0].clone();
    extra.id = "disabled_solo".into();
    extra.enabled = false;
    extra.solo = true;
    extra.items.clear();
    sequence.tracks.push(extra);
    assert_eq!(
        studio::remap_reviews(&inactive_solo, "seq_main").unwrap()["items"][0]["mapping"],
        "resolved"
    );

    // A separate audio layer cannot stand in for a visual source range;
    // a disabled audio clip contributes no placement at all.
    let mut layered = project.clone();
    let sequence = layered.require_sequence_mut("seq_main").unwrap();
    let mut audio = sequence.tracks[0].clone();
    audio.id = "audio_copy".into();
    audio.track_type = TrackType::Audio;
    for item in &mut audio.items {
        item.id = format!("audio_{}", item.id);
    }
    sequence.tracks.push(audio);
    let aligned = studio::remap_reviews(&layered, "seq_main").unwrap();
    assert_eq!(aligned["items"][0]["mapping"], "resolved");
    assert_eq!(
        aligned["items"][0]["rangeMappings"][0]["placements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for item in &mut layered
        .require_sequence_mut("seq_main")
        .unwrap()
        .tracks
        .last_mut()
        .unwrap()
        .items
    {
        item.payload.as_clip_mut().unwrap().audio.enabled = false;
    }
    let silent = studio::remap_reviews(&layered, "seq_main").unwrap();
    assert_eq!(silent["items"][0]["mapping"], "resolved");
    assert_eq!(
        silent["items"][0]["rangeMappings"][0]["placements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A speed-altered duplicate could contain another destination; retain the
    // normal-speed placement but avoid asserting uniqueness.
    let track = layered
        .require_sequence_mut("seq_main")
        .unwrap()
        .tracks
        .last_mut()
        .unwrap();
    track.track_type = TrackType::Video;
    track.items[0].payload.as_clip_mut().unwrap().audio.enabled = true;
    track.items[0]
        .payload
        .as_clip_mut()
        .unwrap()
        .video
        .speed
        .rate = 2.0;
    let uncertain = studio::remap_reviews(&layered, "seq_main").unwrap();
    assert_eq!(uncertain["items"][0]["mapping"], "partial");
    assert_eq!(
        uncertain["items"][0]["rangeMappings"][0]["limitations"],
        json!(["non-normal-speed"])
    );

    // Repeating the same source frames makes the destination ambiguous.
    let mut repeat = project.clone();
    let track = &mut repeat.require_sequence_mut("seq_main").unwrap().tracks[0];
    let mut second = track.items[0].clone();
    second.id = "a_repeat".into();
    second.start.value = 90;
    track.items.push(second);
    assert_eq!(
        studio::remap_reviews(&repeat, "seq_main").unwrap()["items"][0]["mapping"],
        "ambiguous"
    );

    // Existing records without ranges continue to use their original point.
    let mut legacy = project.clone();
    let record = legacy
        .extensions
        .get_mut("avw.reviews")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap();
    record.remove("sourceRanges");
    record.remove("sourceRangeCoverage");
    let old = studio::remap_reviews(&legacy, "seq_main").unwrap();
    assert_eq!(old["items"][0]["mappingBasis"], "legacy-point");
    assert_eq!(old["items"][0]["mapping"], "resolved");

    let omit = |id: &str, item_id: &str| Edit::Omit {
        id: id.into(),
        item_id: item_id.into(),
        reason: "reviewed removal".into(),
    };
    store
        .studio(&omit("remove-b", "b"), 2, "remove-middle", false)
        .unwrap();
    let partial = studio::remap_reviews(&store.project().unwrap(), "seq_main").unwrap();
    assert_eq!(partial["items"][0]["mapping"], "partial");
    assert_eq!(partial["items"][0]["placements"][0]["atMs"], 501);
    assert_eq!(
        partial["items"][0]["rangeMappings"][1]["mapping"],
        "removed_or_unmapped"
    );
    assert_eq!(
        partial["items"][0]["rangeMappings"][2]["placements"][0]["atMs"],
        1000
    );
    store
        .studio(&omit("remove-a", "a"), 3, "remove-start", false)
        .unwrap();
    let partial = studio::remap_reviews(&store.project().unwrap(), "seq_main").unwrap();
    assert_eq!(partial["items"][0]["mapping"], "partial");
    assert!(
        partial["items"][0]["placements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    store
        .studio(&omit("remove-c", "c"), 4, "remove-last", false)
        .unwrap();
    assert_eq!(
        studio::remap_reviews(&store.project().unwrap(), "seq_main").unwrap()["items"][0]["mapping"],
        "removed_or_unmapped"
    );
    store.restore(2, 5, "restore-reviewed-edit").unwrap();
    drop(store);
    let store = Store::open(&root).unwrap();
    assert_eq!(
        studio::remap_reviews(&store.project().unwrap(), "seq_main").unwrap()["items"][0]["mapping"],
        "resolved"
    );
}

#[test]
fn review_flags_unsupported_source_coverage_only_where_it_intersects() {
    use agent_video_workbench::{media, studio};
    use agentcut_core::{
        BlendMode, HandlePolicy, ItemPayload, RationalRate, RationalTime, SequenceRefData,
        TrackType, Transform2d, Transition, TransitionAlignment,
    };
    use std::fs;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Unsupported review sources").unwrap();
    seed(&mut store);
    let mut snapshot = store.project().unwrap();
    let sequence = snapshot.require_sequence_mut("seq_main").unwrap();
    sequence.tracks[0].items[1]
        .payload
        .as_clip_mut()
        .unwrap()
        .video
        .speed
        .rate = 2.0;
    let mut silent = sequence.tracks[0].clone();
    silent.id = "silent_audio".into();
    silent.track_type = TrackType::Audio;
    for item in &mut silent.items {
        item.id = format!("silent_{}", item.id);
        item.payload.as_clip_mut().unwrap().audio.enabled = false;
    }
    sequence.tracks.push(silent);
    let mut nested = sequence.tracks[0].clone();
    nested.id = "nested_track".into();
    nested.items.truncate(1);
    nested.items[0].id = "nested_item".into();
    let nested_source = nested.items[0].payload.as_clip().unwrap().source_range;
    nested.items[0].payload = ItemPayload::Sequence(SequenceRefData {
        sequence_id: "child".into(),
        source_range: nested_source,
        transform: Transform2d::identity(),
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
    });
    sequence.tracks.push(nested);
    sequence.transitions.push(Transition {
        id: "fade".into(),
        left_item_id: "a".into(),
        right_item_id: "b".into(),
        capability: "video.transition.crossfade".into(),
        version: 1,
        duration: RationalTime::new(15, RationalRate::frames(30).unwrap()),
        alignment: TransitionAlignment::Center,
        handle_policy: HandlePolicy::Reject,
        parameters: Default::default(),
        extensions: Default::default(),
    });
    let artifact = root.join("artifact.bin");
    let manifest = root.join("artifact.json");
    fs::write(&artifact, b"fixture").unwrap();
    fs::write(&manifest,serde_json::to_vec(&json!({"snapshot":snapshot,"sequenceId":"seq_main", "verification":verified_timing(media::hash_file(&artifact).unwrap())})).unwrap()).unwrap();
    store.start_job("fixture-job", 1).unwrap();
    store
        .finish_job("fixture-job", &json!({"path":artifact,"manifest":manifest}))
        .unwrap();
    let review = |id: &str, at_ms, end_ms| Edit::ReviewAdd {
        id: id.into(),
        job_id: "fixture-job".into(),
        at_ms,
        end_ms,
        actor: "creator".into(),
        text: "Review this section".into(),
    };
    store
        .studio(&review("complex", 501, 1501), 1, "complex-review", false)
        .unwrap();
    store
        .studio(&review("ordinary", 2000, 2500), 2, "ordinary-review", false)
        .unwrap();
    let project = store.project().unwrap();
    let captured = &project.extensions["avw.reviews"];
    assert_eq!(captured[0]["sourceRangeCoverage"], "partial");
    assert_eq!(
        captured[0]["sourceRangeLimitations"],
        json!(["nested-sequence", "non-normal-speed", "transition-handles"])
    );
    assert_eq!(captured[0]["sourceRanges"].as_array().unwrap().len(), 1);
    assert_eq!(captured[1]["sourceRangeCoverage"], "complete");
    assert_eq!(captured[1]["sourceRangeLimitations"], json!([]));
    let report = studio::remap_reviews(&project, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "partial");
    assert_eq!(report["items"][1]["mapping"], "resolved");
}

#[test]
fn review_distinguishes_video_and_audio_even_when_they_share_a_source() {
    use agent_video_workbench::{media, studio};
    use agentcut_core::{AudioMetadata, TrackType};
    use std::fs;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Media identity").unwrap();
    seed(&mut store);
    let mut snapshot = store.project().unwrap();
    snapshot.assets[0].metadata.audio = Some(AudioMetadata {
        sample_rate: 48000,
        channels: 2,
        channel_layout: "stereo".into(),
        sample_format: "fltp".into(),
    });
    let artifact = root.join("artifact.bin");
    let manifest = root.join("artifact.json");
    fs::write(&artifact, b"fixture").unwrap();
    fs::write(&manifest,serde_json::to_vec(&json!({"snapshot":snapshot,"sequenceId":"seq_main", "verification":verified_timing(media::hash_file(&artifact).unwrap())})).unwrap()).unwrap();
    store.start_job("fixture-job", 1).unwrap();
    store
        .finish_job("fixture-job", &json!({"path":artifact,"manifest":manifest}))
        .unwrap();
    store
        .studio(
            &Edit::ReviewAdd {
                id: "media-note".into(),
                job_id: "fixture-job".into(),
                at_ms: 100,
                end_ms: 900,
                actor: "creator".into(),
                text: "Keep this moment".into(),
            },
            1,
            "media-review",
            false,
        )
        .unwrap();
    let mut project = store.project().unwrap();
    project.assets[0].metadata.audio = Some(AudioMetadata {
        sample_rate: 48000,
        channels: 2,
        channel_layout: "stereo".into(),
        sample_format: "fltp".into(),
    });
    let review = &project.extensions["avw.reviews"][0];
    assert_eq!(review["sourceRanges"][0]["mediaKind"], "video");
    assert_eq!(review["sourceRanges"][1]["mediaKind"], "audio");
    let initial = studio::remap_reviews(&project, "seq_main").unwrap();
    assert_eq!(initial["items"][0]["mapping"], "resolved");
    assert_eq!(
        initial["items"][0]["rangeMappings"][1]["placements"][0]["itemId"],
        "a"
    );

    // Preserve the soundtrack on a dedicated track after removing the picture.
    let mut audio_only = project.clone();
    let sequence = audio_only.require_sequence_mut("seq_main").unwrap();
    let mut audio = sequence.tracks[0].clone();
    audio.id = "audio_only".into();
    audio.track_type = TrackType::Audio;
    audio.items.retain(|item| item.id == "a");
    audio.items[0].id = "a_audio".into();
    sequence.tracks[0].items.retain(|item| item.id != "a");
    sequence.tracks.push(audio);
    let report = studio::remap_reviews(&audio_only, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "partial");
    assert_eq!(
        report["items"][0]["rangeMappings"][0]["mapping"],
        "removed_or_unmapped"
    );
    assert_eq!(
        report["items"][0]["rangeMappings"][1]["mapping"],
        "resolved"
    );

    // Preserve the visual clip while removing its audio: the converse is also partial.
    let mut video_only = project.clone();
    video_only.require_sequence_mut("seq_main").unwrap().tracks[0].items[0]
        .payload
        .as_clip_mut()
        .unwrap()
        .audio
        .enabled = false;
    let report = studio::remap_reviews(&video_only, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "partial");
    assert_eq!(
        report["items"][0]["rangeMappings"][0]["mapping"],
        "resolved"
    );
    assert_eq!(
        report["items"][0]["rangeMappings"][1]["mapping"],
        "removed_or_unmapped"
    );

    // Moving the retained audio to a separate aligned audio track is valid.
    let mut separated = project.clone();
    let sequence = separated.require_sequence_mut("seq_main").unwrap();
    let mut audio = sequence.tracks[0].clone();
    audio.id = "separated_audio".into();
    audio.track_type = TrackType::Audio;
    audio.items.retain(|item| item.id == "a");
    audio.items[0].id = "a_audio".into();
    sequence.tracks[0].items[0]
        .payload
        .as_clip_mut()
        .unwrap()
        .audio
        .enabled = false;
    sequence.tracks.push(audio);
    assert_eq!(
        studio::remap_reviews(&separated, "seq_main").unwrap()["items"][0]["mapping"],
        "resolved"
    );

    // Range records written before media-kind identity remain readable, but
    // matching source times alone cannot certify video/audio preservation.
    let mut untyped = project.clone();
    for range in untyped.extensions.get_mut("avw.reviews").unwrap()[0]["sourceRanges"]
        .as_array_mut()
        .unwrap()
    {
        range.as_object_mut().unwrap().remove("mediaKind");
    }
    let report = studio::remap_reviews(&untyped, "seq_main").unwrap();
    assert_eq!(report["items"][0]["mapping"], "partial");
    assert_eq!(
        report["items"][0]["rangeMappings"][0]["limitations"],
        json!(["media-kind-unknown"])
    );
}
