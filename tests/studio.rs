use agent_video_workbench::{store::Store, studio::Edit};
use agentcut_core::OperationBatch;
use serde_json::json;

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
