use agent_video_workbench::{
    jobs::RenderInput,
    media,
    review_import::{
        self, ReviewDecisionStatus, ReviewDraft, ReviewDraftDecision, ReviewDraftItem,
    },
    store::Store,
    studio,
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    video: PathBuf,
    manifest: PathBuf,
    draft: ReviewDraft,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut store = Store::create(&root, "Feedback fixture").unwrap();
    let initial = store.project().unwrap();
    let rate = json!({"numerator":30,"denominator":1});
    let time = |value| json!({"value":value,"rate":rate});
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":initial.project_id,"baseRevision":0,"idempotencyKey":"review-draft-setup",
        "operations":[
            {"id":"source","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video"}},
            {"id":"clip","op":"clip.add","params":{"id":"clip","asset":"source","track":"track_v1",
                "at":time(0),"sourceIn":time(0),"duration":time(90),"fit":"contain"}}
        ]})).unwrap();
    store.apply(&batch, false).unwrap();
    let frozen = store.project().unwrap();
    let input = RenderInput {
        sequence: "seq_main".into(),
        priority: 0,
        expected_revision: frozen.revision,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: None,
    };
    let job = store.enqueue("feedback-render", &input).unwrap().id;
    let artifact = format!("{job}-1");
    let folder = root.join("renders").join(&artifact);
    fs::create_dir(&folder).unwrap();
    let video = folder.join("video.mp4");
    fs::write(&video, b"verified video fixture").unwrap();
    let hash = media::hash_file(&video).unwrap();
    let manifest = folder.join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "artifactId":artifact,"projectId":frozen.project_id,"revision":frozen.revision,
            "sequenceId":"seq_main","output":"video.mp4","snapshot":frozen,
            "verification":{"sha256":hash,"decoded":true,"expectedFrames":90,
                "probe":{"streams":[{"codec_type":"video","nb_read_frames":"90",
                    "avg_frame_rate":"30/1","start_pts":0,"duration_ts":90,"time_base":"1/30"}]}},
            "delivery":{"reviewVideo":"video.mp4","files":[]}
        }))
        .unwrap(),
    )
    .unwrap();
    let result =
        json!({"artifactId":artifact,"revision":frozen.revision,"path":video,"manifest":manifest});
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), job],
    )
    .unwrap();
    let draft = ReviewDraft {
        schema_version: 1,
        draft_id: "12345678-1234-4abc-8def-123456789abc".into(),
        project_id: frozen.project_id.to_string(),
        job_id: job.clone(),
        artifact_revision: frozen.revision,
        artifact_sha256: hash,
        decision: None,
        items: vec![
            ReviewDraftItem {
                id: "note-1".into(),
                at_ms: 500,
                end_ms: 501,
                actor: "Creator".into(),
                text: "Keep <this> & the reaction".into(),
            },
            ReviewDraftItem {
                id: "note-2".into(),
                at_ms: 1000,
                end_ms: 2000,
                actor: "Zoë".into(),
                text: "Move this\nnear the opening.".into(),
            },
        ],
    };
    Fixture {
        _dir: dir,
        root,
        video,
        manifest,
        draft,
    }
}

fn load(root: &Path) -> Value {
    serde_json::to_value(Store::open(root).unwrap().project().unwrap()).unwrap()
}

#[test]
fn imports_every_note_atomically_with_source_anchors_and_stable_replay() {
    let f = fixture();
    let legacy_json = serde_json::to_value(&f.draft).unwrap();
    assert!(legacy_json.get("decision").is_none());
    let legacy: ReviewDraft = serde_json::from_value(legacy_json).unwrap();
    assert!(legacy.decision.is_none());
    let before = load(&f.root);
    let preview = review_import::import(&f.root, &f.draft, 1, "import-draft", true).unwrap();
    assert_eq!(preview["outcome"]["revision"], 2);
    assert_eq!(load(&f.root), before);
    let imported = review_import::import(&f.root, &f.draft, 1, "import-draft", false).unwrap();
    assert_eq!(imported, preview);
    assert_eq!(imported["items"].as_array().unwrap().len(), 2);
    let store = Store::open(&f.root).unwrap();
    let project = store.project().unwrap();
    let notes = project.extensions["avw.reviews"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["text"], "Keep <this> & the reaction");
    assert_eq!(notes[0]["draftItemId"], "note-1");
    assert_eq!(notes[0]["draftId"], f.draft.draft_id);
    assert_eq!(notes[0]["artifactRevision"], 1);
    assert_eq!(notes[0]["artifactSha256"], f.draft.artifact_sha256);
    assert_eq!(notes[0]["sourceRanges"][0]["assetId"], "source");
    assert_eq!(notes[0]["sourceRangeCoverage"], "complete");
    assert_eq!(notes[1]["text"], "Move this\nnear the opening.");
    let mapped = studio::remap_reviews(&project, "seq_main").unwrap();
    assert_eq!(mapped["items"][0]["mappingBasis"], "source-range");
    assert_eq!(mapped["items"][0]["mapping"], "resolved");
    drop(store);

    // An exact retry returns the original outcome even if the master later
    // becomes unavailable; a changed request under the key still conflicts.
    let original = fs::read(&f.video).unwrap();
    fs::write(&f.video, b"changed").unwrap();
    assert_eq!(
        review_import::import(&f.root, &f.draft, 1, "import-draft", false).unwrap(),
        imported
    );
    let mut altered = f.draft.clone();
    altered.items[0].text.push('!');
    assert!(matches!(
        review_import::import(&f.root, &altered, 1, "import-draft", false),
        Err(agent_video_workbench::Error::KeyConflict)
    ));
    fs::write(&f.video, original).unwrap();
    assert_eq!(Store::open(&f.root).unwrap().project().unwrap().revision, 2);
}

#[test]
fn out_of_range_second_note_rolls_back_and_the_key_can_be_reused() {
    let f = fixture();
    let before = load(&f.root);
    let mut bad = f.draft.clone();
    bad.items[1].end_ms = 3001;
    bad.decision = Some(ReviewDraftDecision {
        status: ReviewDecisionStatus::Approved,
        actor: "Creator".into(),
    });
    let error = review_import::import(&f.root, &bad, 1, "atomic-draft", false).unwrap_err();
    assert!(error.to_string().contains("note-2") && error.to_string().contains("beyond"));
    assert_eq!(load(&f.root), before);
    let result = review_import::import(&f.root, &f.draft, 1, "atomic-draft", false).unwrap();
    assert_eq!(result["outcome"]["revision"], 2);
    assert_eq!(
        Store::open(&f.root).unwrap().project().unwrap().extensions["avw.reviews"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn stale_forged_or_changed_evidence_is_rejected_without_a_commit() {
    let f = fixture();
    let before = load(&f.root);
    let mut forged = f.draft.clone();
    forged.artifact_revision += 1;
    assert!(review_import::import(&f.root, &forged, 1, "forged-revision", false).is_err());
    forged = f.draft.clone();
    forged.artifact_sha256 = "0".repeat(64);
    assert!(review_import::import(&f.root, &forged, 1, "forged-digest", false).is_err());
    forged = f.draft.clone();
    forged.project_id = uuid::Uuid::new_v4().to_string();
    assert!(review_import::import(&f.root, &forged, 1, "wrong-project", false).is_err());
    assert!(matches!(
        review_import::import(&f.root, &f.draft, 0, "stale-head", false),
        Err(agent_video_workbench::Error::Conflict {
            expected: 0,
            current: 1
        })
    ));
    let original = fs::read(&f.video).unwrap();
    fs::write(&f.video, b"modified artifact").unwrap();
    assert!(review_import::import(&f.root, &f.draft, 1, "tampered-master", false).is_err());
    fs::write(&f.video, original).unwrap();
    let old_manifest = fs::read(&f.manifest).unwrap();
    let mut manifest: Value = serde_json::from_slice(&old_manifest).unwrap();
    manifest["snapshot"]["name"] = json!("forged snapshot");
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(review_import::import(&f.root, &f.draft, 1, "tampered-snapshot", false).is_err());
    fs::write(&f.manifest, old_manifest).unwrap();
    assert_eq!(load(&f.root), before);
}

#[test]
fn duplicate_or_modified_prior_items_never_overwrite_and_new_drafts_are_independent() {
    let f = fixture();
    let first = review_import::import(&f.root, &f.draft, 1, "first-draft", false).unwrap();
    let before = load(&f.root);
    let mut changed = f.draft.clone();
    changed.items[0].text = "Changed note".into();
    let error = review_import::import(&f.root, &changed, 2, "changed-draft", false).unwrap_err();
    assert!(error.to_string().contains("already imported"));
    assert_eq!(load(&f.root), before);
    changed.draft_id = "22345678-1234-4abc-8def-123456789abc".into();
    let second = review_import::import(&f.root, &changed, 2, "second-draft", false).unwrap();
    assert_ne!(
        first["items"][0]["reviewId"],
        second["items"][0]["reviewId"]
    );
    assert_eq!(
        Store::open(&f.root).unwrap().project().unwrap().extensions["avw.reviews"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn validates_bounded_untrusted_drafts_before_opening_a_project() {
    let f = fixture();
    let missing = f.root.join("missing");
    let mut bad = f.draft.clone();
    bad.items[1].id = bad.items[0].id.clone();
    assert!(
        review_import::import(&missing, &bad, 1, "valid-key", false)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    bad = f.draft.clone();
    bad.draft_id = "not-a-uuid".into();
    assert!(
        review_import::import(&missing, &bad, 1, "valid-key", false)
            .unwrap_err()
            .to_string()
            .contains("UUID")
    );
    bad = f.draft.clone();
    bad.items[0].text = "é".repeat(2001);
    assert!(
        review_import::import(&missing, &bad, 1, "valid-key", false)
            .unwrap_err()
            .to_string()
            .contains("4000")
    );
    bad = f.draft.clone();
    bad.items[0].actor = " ".into();
    assert!(bad.validate().is_err());
    bad = f.draft.clone();
    bad.items[0].end_ms = bad.items[0].at_ms;
    assert!(bad.validate().is_err());
    bad = f.draft.clone();
    bad.items = vec![];
    assert!(bad.validate().is_err());
    bad = f.draft.clone();
    bad.items = (0..101)
        .map(|n| ReviewDraftItem {
            id: format!("n{n}"),
            ..f.draft.items[0].clone()
        })
        .collect();
    assert!(bad.validate().is_err());
    assert!(
        review_import::import(&missing, &f.draft, 1, "short", false)
            .unwrap_err()
            .to_string()
            .contains("idempotency")
    );
}

#[test]
fn verifies_shorter_preview_and_refuses_missing_timing() {
    let f = fixture();
    let old_manifest = fs::read(&f.manifest).unwrap();
    let mut manifest: Value = serde_json::from_slice(&old_manifest).unwrap();
    let preview = f.video.with_file_name("review-sdr.mp4");
    fs::write(&preview, b"verified preview fixture").unwrap();
    manifest["delivery"] = json!({"reviewVideo":"review-sdr.mp4","files":[{
        "path":"review-sdr.mp4","sha256":media::hash_file(&preview).unwrap(),
        "verification":{"decoded":true,"expectedFrames":60,"probe":{"streams":[{
            "codec_type":"video","nb_read_frames":"60","avg_frame_rate":"30/1",
            "start_pts":0,"duration_ts":60,"time_base":"1/30"}]}}}]});
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let before = load(&f.root);
    let mut outside = f.draft.clone();
    outside.items[1].end_ms = 2001;
    assert!(
        review_import::import(&f.root, &outside, 1, "preview-bound", false)
            .unwrap_err()
            .to_string()
            .contains("beyond")
    );
    assert_eq!(load(&f.root), before);
    manifest["delivery"]["files"][0]["verification"]["probe"] = Value::Null;
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(
        review_import::import(&f.root, &f.draft, 1, "missing-timing", false)
            .unwrap_err()
            .to_string()
            .contains("timing")
    );
    assert_eq!(load(&f.root), before);
    fs::write(&f.manifest, &old_manifest).unwrap();
}

#[test]
fn supports_legacy_synchronous_render_when_its_artifact_identity_is_consistent() {
    let f = fixture();
    let mut manifest: Value = serde_json::from_slice(&fs::read(&f.manifest).unwrap()).unwrap();
    manifest["artifactId"] = json!(f.draft.job_id);
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let db = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    let text: String = db
        .query_row(
            "SELECT result FROM jobs WHERE id=?1",
            [&f.draft.job_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut result: Value = serde_json::from_str(&text).unwrap();
    result["artifactId"] = json!(f.draft.job_id);
    db.execute(
        "UPDATE jobs SET input=NULL,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), f.draft.job_id],
    )
    .unwrap();
    let report = review_import::import(&f.root, &f.draft, 1, "sync-feedback", false).unwrap();
    assert_eq!(report["outcome"]["revision"], 2);
    assert_eq!(report["items"].as_array().unwrap().len(), 2);
}

#[test]
fn concurrent_retries_with_one_key_commit_exactly_once() {
    use std::sync::{Arc, Barrier};
    let f = fixture();
    let mut draft = f.draft.clone();
    draft.decision = Some(ReviewDraftDecision {
        status: ReviewDecisionStatus::Approved,
        actor: "Creator".into(),
    });
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let root = f.root.clone();
            let draft = draft.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                review_import::import(&root, &draft, 1, "concurrent-draft", false)
            })
        })
        .collect::<Vec<_>>();
    let mut results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap());
    assert_eq!(results.next().unwrap(), results.next().unwrap());
    let project = Store::open(&f.root).unwrap().project().unwrap();
    assert_eq!(project.revision, 2);
    assert_eq!(
        project.extensions["avw.reviews"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        project.extensions["avw.reviewDecisions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn decision_only_needs_verified_artifact_but_not_timing_and_preserves_history() {
    let f = fixture();
    let old_manifest = fs::read(&f.manifest).unwrap();
    let mut manifest: Value = serde_json::from_slice(&old_manifest).unwrap();
    manifest["verification"]["probe"] = Value::Null;
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut draft = f.draft.clone();
    draft.items.clear();
    draft.decision = Some(ReviewDraftDecision {
        status: ReviewDecisionStatus::Approved,
        actor: "Reviewer <script>".into(),
    });
    let preview = review_import::import(&f.root, &draft, 1, "decision-only", true).unwrap();
    assert_eq!(preview["decision"]["status"], "approved");
    assert_eq!(Store::open(&f.root).unwrap().project().unwrap().revision, 1);
    let imported = review_import::import(&f.root, &draft, 1, "decision-only", false).unwrap();
    assert_eq!(imported, preview);
    let project = Store::open(&f.root).unwrap().project().unwrap();
    assert!(!project.extensions.contains_key("avw.reviews"));
    let decisions = project.extensions["avw.reviewDecisions"]
        .as_array()
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["status"], "approved");
    assert_eq!(decisions[0]["actor"], "Reviewer <script>");
    assert_eq!(decisions[0]["jobId"], f.draft.job_id);
    assert_eq!(decisions[0]["artifactRevision"], f.draft.artifact_revision);
    assert_eq!(decisions[0]["artifactSha256"], f.draft.artifact_sha256);
    assert_eq!(decisions[0]["recordedRevision"], 2);
    let mapped = studio::remap_reviews(&project, "seq_main").unwrap();
    assert_eq!(mapped["decisions"], json!(decisions));
    assert!(mapped["items"].as_array().unwrap().is_empty());
    fs::write(&f.video, b"changed").unwrap();
    assert_eq!(
        review_import::import(&f.root, &draft, 1, "decision-only", false).unwrap(),
        imported
    );
    fs::write(&f.manifest, old_manifest).unwrap();
}

#[test]
fn later_decisions_are_new_events_and_duplicate_decisions_roll_back_new_notes() {
    let f = fixture();
    let mut first = f.draft.clone();
    first.items.clear();
    first.decision = Some(ReviewDraftDecision {
        status: ReviewDecisionStatus::Approved,
        actor: "Creator".into(),
    });
    let approved = review_import::import(&f.root, &first, 1, "decision-first", false).unwrap();
    let before = load(&f.root);
    let mut changed = first.clone();
    changed.items = vec![f.draft.items[0].clone()];
    changed.decision.as_mut().unwrap().status = ReviewDecisionStatus::ChangesRequested;
    let error = review_import::import(&f.root, &changed, 2, "decision-second", false).unwrap_err();
    assert!(error.to_string().contains("already imported"));
    assert_eq!(load(&f.root), before);
    changed.draft_id = "22345678-1234-4abc-8def-123456789abc".into();
    let requested = review_import::import(&f.root, &changed, 2, "decision-second", false).unwrap();
    assert_eq!(requested["decision"]["status"], "changes-requested");
    assert_ne!(
        requested["decision"]["decisionId"],
        approved["decision"]["decisionId"]
    );
    let project = Store::open(&f.root).unwrap().project().unwrap();
    let decisions = studio::remap_reviews(&project, "seq_main").unwrap()["decisions"].clone();
    assert_eq!(decisions.as_array().unwrap().len(), 2);
    assert_eq!(decisions[0]["status"], "approved");
    assert_eq!(decisions[1]["status"], "changes-requested");
    assert_eq!(
        decisions[0]["artifactSha256"],
        decisions[1]["artifactSha256"]
    );
    assert_eq!(
        project.extensions["avw.reviews"].as_array().unwrap().len(),
        1
    );
}
