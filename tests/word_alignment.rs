use agent_video_workbench::{
    media,
    store::Store,
    studio::Edit,
    word_alignment::{self, AlignedWord, AlignmentProposal, AlignmentReview},
    workflow::{self, Cue, Transcript},
};
use agentcut_core::OperationBatch;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    store: Store,
    original: PathBuf,
    source_sha: String,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    let mut store = Store::create(&root, "Reviewed alignment").unwrap();
    let bytes = b"owned original fixture";
    let source_sha = format!("{:x}", Sha256::digest(bytes));
    let original = root.join("originals").join(&source_sha);
    std::fs::write(&original, bytes).unwrap();
    let project = store.project().unwrap();
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":project.project_id,"baseRevision":0,"idempotencyKey":"alignment-source",
        "operations":[
            {"id":"source","op":"asset.add","params":{"id":"source","uri":format!("originals/{source_sha}"),
                "kind":"video","fingerprint":{"strategy":"sha256","sizeBytes":bytes.len(),"sha256":source_sha},
                "metadata":{"status":"available","duration":{"value":6000,"rate":{"numerator":1000,"denominator":1}}}}},
            {"id":"track","op":"track.add","params":{"id":"captions","sequence":"seq_main","type":"caption"}},
            {"id":"caption","op":"caption.add","params":{"id":"existing_caption","track":"captions",
                "at":{"value":0,"rate":{"numerator":1000,"denominator":1}},
                "duration":{"value":1000,"rate":{"numerator":1000,"denominator":1}},"text":"Original caption stays"}}
        ]})).unwrap();
    store.apply(&batch, false).unwrap();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "source-provider".into(),
                cues: vec![
                    Cue {
                        id: "c1".into(),
                        start_ms: 0,
                        end_ms: 1000,
                        text: "Hello  coffee!".into(),
                    },
                    Cue {
                        id: "c2".into(),
                        start_ms: 1100,
                        end_ms: 2000,
                        text: "Go go".into(),
                    },
                ],
            },
            1,
            "seed-transcript",
        )
        .unwrap();
    store
        .studio(
            &Edit::TranscriptCorrect {
                asset_id: "source".into(),
                cues: BTreeMap::from([("c1".into(), "Hello  café!".into())]),
                sequences: vec![],
            },
            2,
            "correct-transcript",
            false,
        )
        .unwrap();
    Fixture {
        _directory: directory,
        root,
        store,
        original,
        source_sha,
    }
}

fn word(
    id: &str,
    cue: &str,
    text: &str,
    start_byte: u32,
    end_byte: u32,
    start_ms: i64,
    end_ms: i64,
) -> AlignedWord {
    AlignedWord {
        id: id.into(),
        cue_id: cue.into(),
        text: text.into(),
        text_start_byte: start_byte,
        text_end_byte: end_byte,
        start_ms,
        end_ms,
        speaker: Some("Speaker 1".into()),
        confidence: Some(0.8),
    }
}

fn proposal(f: &Fixture) -> AlignmentProposal {
    let selected = workflow::transcripts(&f.store.project().unwrap())
        .unwrap()
        .remove(0);
    AlignmentProposal {
        schema_version: 1,
        asset_id: "source".into(),
        source_sha256: f.source_sha.clone(),
        transcript_sha256: word_alignment::transcript_sha256(&selected).unwrap(),
        provider: "human-reviewed fixture".into(),
        provider_version: "1".into(),
        words: vec![
            word("w1", "c1", "Hello", 0, 5, 100, 350),
            word("w2", "c1", "café!", 7, 13, 400, 900),
            word("w3", "c2", "Go", 0, 2, 1150, 1300),
            word("w4", "c2", "go", 3, 5, 1400, 1600),
        ],
    }
}

fn review() -> AlignmentReview {
    AlignmentReview {
        reviewer: "creator".into(),
        note: "Listened to both selected cues".into(),
    }
}

#[test]
fn reviewed_alignment_is_atomic_paginated_and_preserves_corrected_text_and_caption() {
    let mut f = fixture();
    let p = proposal(&f);
    let before = f.store.project().unwrap();
    let old_history = f.store.history().unwrap();
    let report = word_alignment::preview(&f.root, &before, &p, 3).unwrap();
    assert_eq!(report["sourceBytesVerified"], true);
    assert_eq!(report["wordCount"], 4);
    assert_eq!(report["selectedCueCount"], 2);
    assert_eq!(report["unalignedCueCount"], 0);
    let dry = f
        .store
        .accept_word_alignment(&p, &review(), 3, "accept-alignment", true)
        .unwrap();
    assert_eq!(dry.revision, 4);
    assert_eq!(f.store.project().unwrap(), before);
    assert_eq!(f.store.history().unwrap(), old_history);
    let accepted = f
        .store
        .accept_word_alignment(&p, &review(), 3, "accept-alignment", false)
        .unwrap();
    assert_eq!(accepted, dry);
    let mut changed_review = review();
    changed_review.note = "A different review".into();
    assert!(matches!(
        f.store
            .accept_word_alignment(&p, &changed_review, 3, "accept-alignment", false),
        Err(agent_video_workbench::Error::KeyConflict)
    ));
    let after = f.store.project().unwrap();
    assert_eq!(after.sequences, before.sequences);
    assert_eq!(
        workflow::transcripts(&after).unwrap()[0].cues[0].text,
        "Hello  café!"
    );
    assert_eq!(
        after.extensions["avw.corrections.source"],
        before.extensions["avw.corrections.source"]
    );
    let first = word_alignment::get(&f.root, &after, "source", Some(4), 0, 2).unwrap();
    assert_eq!(first["status"], "current");
    assert_eq!(first["sourceBytesVerified"], true);
    assert_eq!(first["items"][1]["text"], "café!");
    assert_eq!(first["items"][1]["textStartByte"], 7);
    assert_eq!(first["nextOffset"], 2);
    let second = word_alignment::get(&f.root, &after, "source", Some(4), 2, 2).unwrap();
    assert_eq!(second["items"][0]["text"], "Go");
    assert_eq!(second["items"][1]["text"], "go");
    assert!(second["nextOffset"].is_null());
    assert!(word_alignment::get(&f.root, &after, "source", None, 2, 2).is_err());
    assert!(word_alignment::get(&f.root, &after, "source", Some(3), 2, 2).is_err());
    assert!(word_alignment::get(&f.root, &after, "source", Some(4), 0, 101).is_err());

    // A project backup retains the accepted record, provenance, and original.
    let backup = f._directory.path().join("backup");
    media::backup(&f.root, &backup).unwrap();
    let copied = Store::open(&backup).unwrap();
    let copied_project = copied.project().unwrap();
    let copied_report =
        word_alignment::get(&backup, &copied_project, "source", Some(4), 0, 100).unwrap();
    assert_eq!(copied_report["status"], "current");
    assert_eq!(copied_report["review"]["reviewer"], "creator");
    assert_eq!(copied_report["items"], json!(p.words));
    assert_eq!(
        workflow::transcripts(&copied_project).unwrap()[0].cues[0].text,
        "Hello  café!"
    );

    // Later correction retains the accepted record but changes its freshness;
    // replay of the original accepted intent remains idempotent.
    f.store
        .studio(
            &Edit::TranscriptCorrect {
                asset_id: "source".into(),
                cues: BTreeMap::from([("c1".into(), "Hello  coffee!".into())]),
                sequences: vec![],
            },
            4,
            "later-correction",
            false,
        )
        .unwrap();
    let head = f.store.project().unwrap();
    let stale = word_alignment::get(&f.root, &head, "source", Some(5), 0, 10).unwrap();
    assert_eq!(stale["status"], "stale");
    assert_eq!(
        stale["staleReasons"],
        json!(["selected-transcript-changed"])
    );
    assert!(
        word_alignment::preview(&f.root, &head, &p, 5)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(
        f.store
            .accept_word_alignment(&p, &review(), 3, "accept-alignment", false)
            .unwrap(),
        accepted
    );
    assert_eq!(f.store.project().unwrap(), head);
}

#[test]
fn invalid_text_offsets_coverage_and_timing_never_commit_or_reserve_key() {
    let mut f = fixture();
    let before = f.store.project().unwrap();
    let good = proposal(&f);
    let mut bad = Vec::new();
    let mut missing = good.clone();
    missing.words.remove(1);
    bad.push(missing);
    let mut split = good.clone();
    split.words[1].text_start_byte = 11;
    bad.push(split); // middle of UTF-8 é
    let mut overlap = good.clone();
    overlap.words[1].start_ms = 300;
    bad.push(overlap);
    let mut outside = good.clone();
    outside.words[3].end_ms = 2001;
    bad.push(outside);
    let mut duplicate = good.clone();
    duplicate.words[1].id = "w1".into();
    bad.push(duplicate);
    let mut wrong_text = good.clone();
    wrong_text.words[1].text = "coffee!".into();
    bad.push(wrong_text);
    let mut reordered = good.clone();
    reordered.words.swap(0, 1);
    bad.push(reordered);
    let mut cross_cue_reordered = good.clone();
    cross_cue_reordered.words.swap(1, 2);
    bad.push(cross_cue_reordered);
    let mut unknown_cue = good.clone();
    unknown_cue.words[0].cue_id = "unknown".into();
    bad.push(unknown_cue);
    let mut invalid_confidence = good.clone();
    invalid_confidence.words[1].confidence = Some(1.1);
    bad.push(invalid_confidence);
    let mut changed_source = good.clone();
    changed_source.source_sha256 = "0".repeat(64);
    bad.push(changed_source);
    let mut changed_transcript = good.clone();
    changed_transcript.transcript_sha256 = "1".repeat(64);
    bad.push(changed_transcript);
    let mut missing_review = review();
    missing_review.note.clear();
    assert!(
        f.store
            .accept_word_alignment(&good, &missing_review, 3, "reusable-key", false)
            .is_err()
    );
    for invalid_key in ["short".to_owned(), "x".repeat(201), "        ".to_owned()] {
        assert!(
            f.store
                .accept_word_alignment(&good, &review(), 3, &invalid_key, false)
                .is_err()
        );
    }
    for (index, item) in bad.iter().enumerate() {
        assert!(
            f.store
                .accept_word_alignment(item, &review(), 3, "reusable-key", false)
                .is_err(),
            "accepted invalid case {index}"
        );
        assert_eq!(f.store.project().unwrap(), before);
    }
    // The failed validations leave the key available for a corrected request.
    assert!(
        f.store
            .accept_word_alignment(&good, &review(), 3, "reusable-key", false)
            .is_ok()
    );
}

#[test]
fn changed_original_is_not_reported_current_and_cannot_be_accepted() {
    let mut f = fixture();
    let p = proposal(&f);
    f.store
        .accept_word_alignment(&p, &review(), 3, "accepted-alignment", false)
        .unwrap();
    std::fs::write(&f.original, b"changed bytes").unwrap();
    let project = f.store.project().unwrap();
    let result = word_alignment::get(&f.root, &project, "source", Some(4), 0, 10).unwrap();
    assert_eq!(result["status"], "unavailable");
    assert_eq!(result["sourceBytesVerified"], false);
    assert!(word_alignment::preview(&f.root, &project, &p, 4).is_err());
    assert!(
        f.store
            .accept_word_alignment(&p, &review(), 4, "new-alignment", false)
            .is_err()
    );
    assert_eq!(f.store.project().unwrap(), project);
}

#[test]
fn changed_source_identity_or_duration_makes_alignment_stale() {
    let mut f = fixture();
    let p = proposal(&f);
    f.store
        .accept_word_alignment(&p, &review(), 3, "accepted-alignment", false)
        .unwrap();
    let current = f.store.project().unwrap();

    let mut changed_duration = current.clone();
    changed_duration.assets[0].metadata.duration = Some(agentcut_core::RationalTime::new(
        5999,
        agentcut_core::RationalRate::frames(1000).unwrap(),
    ));
    let result = word_alignment::get(&f.root, &changed_duration, "source", Some(4), 0, 10).unwrap();
    assert_eq!(result["status"], "stale");
    assert_eq!(result["staleReasons"], json!(["source-duration-changed"]));

    let mut changed_identity = current;
    changed_identity.assets[0].fingerprint.sha256 = Some("0".repeat(64));
    let result = word_alignment::get(&f.root, &changed_identity, "source", Some(4), 0, 10).unwrap();
    assert_eq!(result["status"], "stale");
    assert_eq!(result["staleReasons"], json!(["source-identity-changed"]));
    // Matching bytes of the current asset cannot verify a different bound hash.
    assert_eq!(result["sourceBytesVerified"], false);
}

#[test]
fn aligning_a_subset_of_whole_cues_is_explicit() {
    let f = fixture();
    let mut p = proposal(&f);
    p.words.truncate(2);
    let result = word_alignment::preview(&f.root, &f.store.project().unwrap(), &p, 3).unwrap();
    assert_eq!(result["selectedCueCount"], 1);
    assert_eq!(result["unalignedCueCount"], 1);
}
