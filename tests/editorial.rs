use agent_video_workbench::{
    editorial::{self, EditPlan},
    store::Store,
    workflow::{Cue, Transcript},
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};

fn seed(store: &mut Store, numerator: u32, denominator: u32, cues: Vec<Cue>) {
    let p = store.project().unwrap();
    let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":0,"idempotencyKey":"seed-editorial","operations":[{"id":"source","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video","metadata":{"status":"available","duration":{"value":10,"rate":{"numerator":1,"denominator":1}},"video":{"size":{"width":640,"height":360},"frameRate":{"numerator":numerator,"denominator":denominator},"pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,"colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}}}]})).unwrap();
    store.apply(&batch, false).unwrap();
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "reviewed-fixture".into(),
                cues,
            },
            1,
            "reviewed-transcript",
        )
        .unwrap();
}
fn cue(id: &str, start: i64, end: i64) -> Cue {
    Cue {
        id: id.into(),
        start_ms: start,
        end_ms: end,
        text: format!("Reviewed {id}"),
    }
}
fn plan(cuts: Value) -> EditPlan {
    let decisions: Vec<_> = cuts
        .as_array()
        .unwrap()
        .iter()
        .map(|c| json!({"cutId":c["id"],"reason":"Retain the complete thought and reaction"}))
        .collect();
    serde_json::from_value(json!({"edit":{"outputId":"candidate","name":"Restrained candidate","cuts":cuts},"brief":{"objective":"Keep original delivery and complete thoughts"},"decisions":decisions})).unwrap()
}
fn sha(report: &Value) -> &str {
    report["planSha256"].as_str().unwrap()
}

#[test]
fn speech_risks_block_atomically_and_explicit_reviews_bind_exact_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Speech safety").unwrap();
    seed(&mut store, 30, 1, vec![cue("sentence", 1000, 2000)]);
    let mut proposed =
        plan(json!([{"id":"thought","assetId":"source","startMs":1200,"endMs":2100}]));
    let before = store.project().unwrap();
    let report = editorial::plan(&before, &proposed, 2).unwrap();
    assert_eq!(report["readyToApply"], false);
    assert_eq!(report["unreviewedRisks"], 2);
    assert_eq!(report["cuts"][0]["boundaryCues"][0]["id"], "sentence");
    assert_eq!(
        report["cuts"][0]["boundaryExpansionProposal"]["startMs"],
        750
    );
    assert_eq!(
        report["cuts"][0]["boundaryExpansionProposal"]["endMs"],
        2250
    );
    assert!(
        store
            .apply_edit_plan(&proposed, 2, "apply-reviewed", sha(&report), false)
            .unwrap_err()
            .to_string()
            .contains("unreviewed")
    );
    assert_eq!(store.project().unwrap(), before);
    proposed.decisions[0].reviews=serde_json::from_value(json!([
        {"risk":"speech-boundary","note":"Listened to the source; cue includes a preceding unrelated sentence"},
        {"risk":"speech-handle","note":"Listened across the exit; the following reaction stays in the selected take"}])).unwrap();
    assert!(
        store
            .apply_edit_plan(&proposed, 2, "apply-reviewed", sha(&report), false)
            .unwrap_err()
            .to_string()
            .contains("planSha256")
    );
    let report = editorial::plan(&before, &proposed, 2).unwrap();
    assert_eq!(report["readyToApply"], true);
    store
        .apply_edit_plan(&proposed, 2, "apply-reviewed", sha(&report), true)
        .unwrap();
    assert_eq!(store.project().unwrap(), before);
    let outcome = store
        .apply_edit_plan(&proposed, 2, "apply-reviewed", sha(&report), false)
        .unwrap();
    drop(store);
    let mut store = Store::open(&root).unwrap();
    assert_eq!(
        store
            .apply_edit_plan(&proposed, 2, "apply-reviewed", sha(&report), false)
            .unwrap(),
        outcome
    );
    assert_eq!(
        editorial::state(&store.project().unwrap(), "candidate").unwrap()["status"],
        "current"
    );
    store.restore(2, 3, "restore-before-plan").unwrap();
    store.restore(3, 4, "restore-with-plan").unwrap();
    assert_eq!(editorial::state(&store.project().unwrap(),"candidate").unwrap()["decisions"][0]["reviews"].as_array().unwrap().len(),2);
}

#[test]
fn handles_use_exact_actual_times_and_source_edges_need_no_invented_padding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("p"), "Handles").unwrap();
    seed(
        &mut store,
        30,
        1,
        vec![cue("sentence", 1000, 2000), cue("last", 9000, 10000)],
    );
    let proposed = plan(json!([{"id":"thought","assetId":"source","startMs":750,"endMs":2250}]));
    let report = editorial::plan(&store.project().unwrap(), &proposed, 2).unwrap();
    assert_eq!(report["readyToApply"], true);
    assert_eq!(report["cuts"][0]["contextWindows"][0]["sourceStartMs"], 0);
    assert_eq!(
        report["cuts"][0]["contextWindows"][0]["boundaryOffsetMs"],
        750.0
    );
    let whole = plan(json!([{"id":"whole","assetId":"source","startMs":0,"endMs":10000}]));
    assert_eq!(
        editorial::plan(&store.project().unwrap(), &whole, 2).unwrap()["readyToApply"],
        true
    );
    let mut nearer = proposed.clone();
    nearer.edit.cuts[0].start_ms = 751;
    assert_eq!(
        editorial::plan(&store.project().unwrap(), &nearer, 2).unwrap()["readyToApply"],
        false
    );
}

#[test]
fn ntsc_rounding_is_checked_even_when_requested_end_precedes_speech() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("p"), "NTSC").unwrap();
    seed(&mut store, 30000, 1001, vec![cue("incoming", 2501, 2700)]);
    let proposed = plan(json!([{"id":"take","assetId":"source","startMs":1000,"endMs":2500}]));
    let report = editorial::plan(&store.project().unwrap(), &proposed, 2).unwrap();
    assert_eq!(report["cuts"][0]["outputDuration"]["value"], 45);
    assert_eq!(report["cuts"][0]["findings"][0]["risk"], "speech-boundary");
    assert_eq!(report["cuts"][0]["requestedRangeMs"], json!([1000, 2500]));
}

#[test]
fn omitted_ranges_are_a_union_and_silent_actions_remain_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("p"), "Omissions").unwrap();
    seed(&mut store, 30, 1, vec![cue("removed", 2000, 2500)]);
    let mut proposed = plan(json!([
        {"id":"later","assetId":"source","startMs":4000,"endMs":6000},
        {"id":"first","assetId":"source","startMs":0,"endMs":1000},
        {"id":"repeat","assetId":"source","startMs":500,"endMs":1500}]));
    proposed.edit.allow_reorder = true;
    let report = editorial::plan(&store.project().unwrap(), &proposed, 2).unwrap();
    assert_eq!(report["omittedSourceRanges"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["omittedSourceRanges"][0]["sourceStart"]["value"],
        4500
    );
    assert_eq!(
        report["omittedSourceRanges"][0]["cueExcerpts"][0]["id"],
        "removed"
    );
    assert!(
        report["omittedSourceRanges"][1]["cueExcerpts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(report["cuts"][1]["findings"][0]["risk"], "reordered-source");
    assert_eq!(report["readyToApply"], false);
}

#[test]
fn changed_timing_or_transcript_invalidates_review_but_project_rename_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("p"), "Review state").unwrap();
    seed(&mut store, 30, 1, vec![cue("sentence", 1000, 2000)]);
    let proposed = plan(json!([{"id":"thought","assetId":"source","startMs":0,"endMs":3000}]));
    let report = editorial::plan(&store.project().unwrap(), &proposed, 2).unwrap();
    store
        .apply_edit_plan(&proposed, 2, "apply-current-plan", sha(&report), false)
        .unwrap();
    let p = store.project().unwrap();
    let rename:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":3,"idempotencyKey":"unrelated-rename","operations":[{"id":"name","op":"project.rename","params":{"name":"Renamed"}}]})).unwrap();
    store.apply(&rename, false).unwrap();
    assert_eq!(
        editorial::state(&store.project().unwrap(), "candidate").unwrap()["status"],
        "current"
    );
    let mut changed = store.project().unwrap();
    changed.require_sequence_mut("candidate").unwrap().tracks[0].items[0]
        .duration
        .value -= 1;
    assert_eq!(
        editorial::state(&changed, "candidate").unwrap()["status"],
        "stale"
    );
    store
        .transcript_import(
            Transcript {
                asset_id: "source".into(),
                language: "en".into(),
                provider: "correction".into(),
                cues: vec![cue("sentence", 1000, 2100)],
            },
            4,
            "correct-source-cue",
        )
        .unwrap();
    assert_eq!(
        agent_video_workbench::assembly::preflight(&store.project().unwrap(), "candidate").unwrap()
            ["editPlan"]["status"],
        "stale"
    );
}

#[test]
fn missing_decisions_invalid_reviews_and_stale_revision_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&tmp.path().join("p"), "Validation").unwrap();
    seed(&mut store, 30, 1, vec![]);
    let mut proposed = plan(json!([{"id":"thought","assetId":"source","startMs":0,"endMs":1000}]));
    let snapshot = store.project().unwrap();
    assert_eq!(
        editorial::plan(&snapshot, &proposed, 1).unwrap_err().code(),
        "E_REVISION_CONFLICT"
    );
    proposed.decisions.clear();
    assert!(editorial::plan(&snapshot, &proposed, 2).is_err());
    proposed.decisions=serde_json::from_value(json!([{"cutId":"thought","reason":"Keep action","reviews":[{"risk":"speech-boundary","note":" "}]}])).unwrap();
    assert!(editorial::plan(&snapshot, &proposed, 2).is_err());
    assert_eq!(store.project().unwrap(), snapshot);
}

#[test]
fn service_returns_complete_scoped_and_repeatable_context_requests() {
    use agent_video_workbench::{
        media,
        service::{Request, Service},
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("p");
    let mut store = Store::create(&root, "Source context").unwrap();
    seed(&mut store, 30, 1, vec![]);
    let proposed = plan(json!([{"id":"take","assetId":"source","startMs":1000,"endMs":2500}]));
    let service = Service {
        root: Some(tmp.path().into()),
        backend: media::backend("ffmpeg".into(), "ffprobe".into()),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    let request = json!({"command":"plan-edit","project":"p","expectedRevision":2,"plan":proposed});
    let report = service
        .execute(serde_json::from_value(request.clone()).unwrap())
        .unwrap();
    assert_eq!(
        service
            .execute(serde_json::from_value(request).unwrap())
            .unwrap(),
        report
    );
    for window in report["cuts"][0]["contextWindows"].as_array().unwrap() {
        let parsed: Request = serde_json::from_value(window["previewRequest"].clone()).unwrap();
        let Request::AnalyzeStart {
            project,
            task,
            expected_revision,
            no_launch,
            ..
        } = parsed
        else {
            panic!("not a proxy job")
        };
        assert_eq!(project, std::path::Path::new("p"));
        assert_eq!(expected_revision, 2);
        assert!(no_launch);
        agent_video_workbench::analysis::validate(
            &agent_video_workbench::analysis::Input {
                task,
                provider: None,
            },
            &store.project().unwrap(),
        )
        .unwrap();
    }
    assert_eq!(store.project().unwrap().revision, 2);
}
