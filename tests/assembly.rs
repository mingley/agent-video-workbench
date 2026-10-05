use agent_video_workbench::{
    assembly::{Assemble, preflight},
    store::Store,
};
use agentcut_core::OperationBatch;
use serde_json::json;

fn seed(store: &mut Store, rate: serde_json::Value) {
    let project = store.project().unwrap();
    let batch: OperationBatch=serde_json::from_value(json!({
        "schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":0,
        "idempotencyKey":"seed-assembly","operations":[{"id":"source","op":"asset.add",
        "params":{"id":"source","uri":"originals/test","kind":"video","metadata":{
            "status":"available","duration":{"value":20,"rate":{"numerator":1,"denominator":1}},
            "video":{"size":{"width":640,"height":360},"frameRate":rate,"pixelFormat":"yuv420p10le",
            "sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
            "colorPrimaries":"bt2020","colorTransfer":"arib-std-b67","colorMatrix":"bt2020nc","colorRange":"tv"}
        }}}]
    })).unwrap();
    store.apply(&batch, false).unwrap();
}

fn edit(cuts: serde_json::Value) -> Assemble {
    serde_json::from_value(json!({"outputId":"native","name":"Natural source edit","cuts":cuts}))
        .unwrap()
}

#[test]
fn source_defaults_dry_run_replay_and_snapshot_restore_are_durable() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("project"), "Source edit").unwrap();
    seed(&mut store, json!({"numerator":60,"denominator":1}));
    let proposal = edit(json!([{"id":"whole","assetId":"source","startMs":0,"endMs":2000}]));
    store
        .assemble(&proposal, 1, "assemble-baseline", true)
        .unwrap();
    assert_eq!(store.project().unwrap().revision, 1);
    let outcome = store
        .assemble(&proposal, 1, "assemble-baseline", false)
        .unwrap();
    let p = store.project().unwrap();
    let seq = p.require_sequence("native").unwrap();
    assert_eq!((seq.canvas.width, seq.canvas.height), (640, 360));
    assert_eq!(seq.frame_rate.numerator, 60);
    assert_eq!(seq.tracks.iter().flat_map(|t| &t.items).count(), 1);
    assert_eq!(
        preflight(&p, "native").unwrap()["outputColor"]["transfer"],
        "arib-std-b67"
    );
    let root = temp.path().join("project");
    drop(store);
    let mut store = Store::open(&root).unwrap();
    assert_eq!(
        store
            .assemble(&proposal, 1, "assemble-baseline", false)
            .unwrap(),
        outcome
    );
    store.restore(1, 2, "restore-source-baseline").unwrap();
    store.restore(2, 3, "restore-assembly").unwrap();
    assert_eq!(
        preflight(&store.project().unwrap(), "native").unwrap()["outputColor"]["pixelFormat"],
        "yuv420p10le"
    );
}

#[test]
fn removing_short_pauses_and_reordering_need_explicit_edit_intent() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("project"), "Natural timing").unwrap();
    seed(&mut store, json!({"numerator":60,"denominator":1}));
    let mut proposal = edit(json!([
        {"id":"a","assetId":"source","startMs":0,"endMs":1000},
        {"id":"b","assetId":"source","startMs":1100,"endMs":2000}]));
    let before = store.project().unwrap();
    assert!(
        store
            .assemble(&proposal, 1, "tight-cut-intent", false)
            .unwrap_err()
            .to_string()
            .contains("pause")
    );
    assert_eq!(store.project().unwrap(), before);
    proposal.allow_tight_cuts = true;
    store
        .assemble(&proposal, 1, "tight-cut-intent", false)
        .unwrap();
    let mut reordered = edit(json!([
        {"id":"b","assetId":"source","startMs":3000,"endMs":4000},
        {"id":"a","assetId":"source","startMs":0,"endMs":1000}]));
    reordered.output_id = "reordered".into();
    assert!(
        store
            .assemble(&reordered, 2, "reorder-intent", false)
            .is_err()
    );
    assert_eq!(store.project().unwrap().revision, 2);
    reordered.allow_reorder = true;
    store
        .assemble(&reordered, 2, "reorder-intent", false)
        .unwrap();
}

#[test]
fn ntsc_source_coordinates_retain_exact_duration_across_rates() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("project"), "Exact cadence").unwrap();
    seed(&mut store, json!({"numerator":30000,"denominator":1001}));
    let proposal = edit(json!([{"id":"take","assetId":"source","startMs":1000,"endMs":2500}]));
    store
        .assemble(&proposal, 1, "ntsc-assembly", false)
        .unwrap();
    let p = store.project().unwrap();
    let seq = p.require_sequence("native").unwrap();
    let clip = p.require_item("native_take").unwrap();
    assert_eq!(clip.duration.value, 45);
    assert!(
        clip.source_range()
            .unwrap()
            .duration
            .same_instant(clip.duration)
            .unwrap()
    );
    assert_eq!(seq.frame_rate.denominator, 1001);
    assert!(
        clip.source_range()
            .unwrap()
            .start
            .same_instant(agentcut_core::RationalTime::new(
                1000,
                agentcut_core::RationalRate::frames(1000).unwrap()
            ))
            .unwrap()
    );
}

#[test]
fn preserve_policy_cannot_silently_ignore_a_later_crop() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("project"), "Intent").unwrap();
    seed(&mut store, json!({"numerator":60,"denominator":1}));
    store
        .assemble(
            &edit(json!([{"id":"whole","assetId":"source","startMs":0,"endMs":2000}])),
            1,
            "preserve-intent",
            false,
        )
        .unwrap();
    let p = store.project().unwrap();
    let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":2,"idempotencyKey":"crop-after-assembly","operations":[{"id":"crop","op":"item.set","target":"native_whole","params":{"property":"video.crop.left","value":0.1}}]})).unwrap();
    store.apply(&batch, false).unwrap();
    assert!(preflight(&store.project().unwrap(), "native").is_err());
}
