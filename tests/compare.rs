use agent_video_workbench::{assembly::Assemble, compare, jobs::RenderInput, media, store::Store};
use agentcut_core::{OperationBatch, RationalTime};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn seed(store: &mut Store, rate: Value) {
    seed_hash(
        store,
        rate,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );
}

fn seed_hash(store: &mut Store, rate: Value, hash: &str) {
    let project = store.project().unwrap();
    let ops: Vec<_> = ["source", "alias"].into_iter().map(|id| json!({
        "id":format!("add-{id}"),"op":"asset.add","params":{"id":id,"uri":"originals/test","kind":"video",
            "fingerprint":{"strategy":"sha256","sizeBytes":1,"sha256":hash},
            "metadata":{"status":"available","duration":{"value":10,"rate":{"numerator":1,"denominator":1}},
                "video":{"size":{"width":640,"height":360},"frameRate":rate,"pixelFormat":"yuv420p",
                    "sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
                    "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}}
    })).collect();
    let batch: OperationBatch = serde_json::from_value(
        json!({"schemaVersion":"1.0.0","projectId":project.project_id,
        "baseRevision":project.revision,"idempotencyKey":format!("seed-comparison-{}",project.revision),"operations":ops}),
    )
    .unwrap();
    store.apply(&batch, false).unwrap();
}

fn assemble(store: &mut Store, id: &str, cuts: Value) {
    let edit: Assemble =
        serde_json::from_value(json!({"outputId":id,"name":id,"allowReorder":true,
        "allowTightCuts":true,"cuts":cuts}))
        .unwrap();
    let revision = store.project().unwrap().revision;
    store
        .assemble(&edit, revision, &format!("assemble-{id}"), false)
        .unwrap();
}

fn queued_fixture(store: &mut Store, root: &Path, sequence: &str) -> (String, std::path::PathBuf) {
    let snapshot = store.project().unwrap();
    let input = RenderInput {
        sequence: sequence.into(),
        priority: 0,
        expected_revision: snapshot.revision,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: None,
    };
    let job = store
        .enqueue(&format!("render-{sequence}"), &input)
        .unwrap();
    let artifact = format!("{}-1", job.id);
    let folder = root.join("renders").join(&artifact);
    fs::create_dir(&folder).unwrap();
    let video = folder.join("video.mp4");
    let preview = folder.join("review-sdr.mp4");
    fs::write(&video, format!("master-{sequence}")).unwrap();
    fs::write(&preview, format!("preview-{sequence}")).unwrap();
    let manifest = folder.join("manifest.json");
    fs::write(&manifest, serde_json::to_vec(&json!({"artifactId":artifact,"projectId":snapshot.project_id,
        "revision":snapshot.revision,"sequenceId":sequence,"output":"video.mp4","snapshot":snapshot,
        "verification":{"sha256":media::hash_file(&video).unwrap()},
        "delivery":{"reviewVideo":"review-sdr.mp4","files":[{"path":"review-sdr.mp4","sha256":media::hash_file(&preview).unwrap()}]}})).unwrap()).unwrap();
    let result = json!({"artifactId":artifact,"revision":snapshot.revision,"path":video,"manifest":manifest});
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), job.id],
    )
    .unwrap();
    (job.id, manifest)
}

fn seconds(value: &Value) -> f64 {
    serde_json::from_value::<RationalTime>(value.clone())
        .unwrap()
        .as_seconds_f64()
}

fn same_time(value: &Value, ticks: i64, rate: u32) {
    use agentcut_core::RationalRate;
    let actual: RationalTime = serde_json::from_value(value.clone()).unwrap();
    assert!(
        actual
            .same_instant(RationalTime::new(
                ticks,
                RationalRate::frames(rate).unwrap()
            ))
            .unwrap()
    );
}

#[test]
fn frozen_comparison_partitions_overlaps_and_detects_relative_reordering() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Comparison").unwrap();
    seed(&mut store, json!({"numerator":30,"denominator":1}));
    assemble(
        &mut store,
        "baseline",
        json!([
        {"id":"a","assetId":"source","startMs":0,"endMs":2000},
        {"id":"b","assetId":"source","startMs":4000,"endMs":6000}]),
    );
    let (baseline, _) = queued_fixture(&mut store, &root, "baseline");
    assemble(
        &mut store,
        "candidate",
        json!([
        {"id":"b","assetId":"source","startMs":4000,"endMs":6000},
        {"id":"a","assetId":"source","startMs":1000,"endMs":3000}]),
    );
    let (candidate, _) = queued_fixture(&mut store, &root, "candidate");
    let before = store.project().unwrap();
    let history = store.history().unwrap();
    let result = compare::compare_outputs(&root, &baseline, &candidate).unwrap();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["baseline"]["sequenceBinding"], "job-input");
    assert_eq!(result["scope"]["notCompared"][0], "audio");
    assert_eq!(result["omitted"].as_array().unwrap().len(), 1);
    assert_eq!(result["omitted"][0]["mapping"], "unpaired");
    assert_eq!(result["added"][0]["mapping"], "unpaired");
    assert_eq!(
        (
            seconds(&result["omitted"][0]["sourceStart"]),
            seconds(&result["omitted"][0]["sourceEnd"])
        ),
        (0.0, 1.0)
    );
    assert_eq!(
        (
            seconds(&result["added"][0]["sourceStart"]),
            seconds(&result["added"][0]["sourceEnd"])
        ),
        (2.0, 3.0)
    );
    let retained = result["retained"].as_array().unwrap();
    assert_eq!(retained.len(), 2);
    assert_eq!(
        (
            seconds(&retained[0]["sourceStart"]),
            seconds(&retained[0]["sourceEnd"])
        ),
        (1.0, 2.0)
    );
    assert_eq!(
        seconds(&retained[0]["baselinePlacements"][0]["outputStart"]),
        1.0
    );
    assert_eq!(
        seconds(&retained[0]["candidatePlacements"][0]["outputStart"]),
        2.0
    );
    assert!(retained.iter().all(|part| part["reordered"] == true));
    assert_eq!(store.project().unwrap(), before);
    assert_eq!(store.history().unwrap(), history);
    assert_eq!(
        compare::compare_outputs(&root, &baseline, &candidate).unwrap(),
        result
    );
}

#[test]
fn repeated_sources_are_ambiguous_and_asset_aliases_remain_distinct() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Repeated source").unwrap();
    seed(&mut store, json!({"numerator":30,"denominator":1}));
    assemble(
        &mut store,
        "baseline",
        json!([
        {"id":"a","assetId":"source","startMs":0,"endMs":1000},
        {"id":"b","assetId":"source","startMs":0,"endMs":1000}]),
    );
    let (baseline, _) = queued_fixture(&mut store, &root, "baseline");
    assemble(
        &mut store,
        "candidate",
        json!([
        {"id":"a","assetId":"source","startMs":0,"endMs":1000},
        {"id":"b","assetId":"alias","startMs":0,"endMs":1000}]),
    );
    let (candidate, _) = queued_fixture(&mut store, &root, "candidate");
    let result = compare::compare_outputs(&root, &baseline, &candidate).unwrap();
    let part = &result["retained"][0];
    assert_eq!(part["mapping"], "ambiguous");
    assert_eq!(
        (
            part["baselineCount"].as_u64(),
            part["candidateCount"].as_u64()
        ),
        (Some(2), Some(1))
    );
    assert!(part["reordered"].is_null());
    assert_eq!(result["added"][0]["assetId"], "alias");
    assert!(result["omitted"].as_array().unwrap().is_empty());
}

#[test]
fn reused_asset_id_with_different_source_hash_is_not_conflated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Reused source ID").unwrap();
    let rate = json!({"numerator":30,"denominator":1});
    seed(&mut store, rate.clone());
    assemble(
        &mut store,
        "baseline",
        json!([{"id":"a","assetId":"source","startMs":0,"endMs":1000}]),
    );
    let (baseline, _) = queued_fixture(&mut store, &root, "baseline");
    store.restore(0, 2, "restore-before-source").unwrap();
    seed_hash(
        &mut store,
        rate,
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    );
    assemble(
        &mut store,
        "candidate",
        json!([{"id":"a","assetId":"source","startMs":0,"endMs":1000}]),
    );
    let (candidate, _) = queued_fixture(&mut store, &root, "candidate");
    let result = compare::compare_outputs(&root, &baseline, &candidate).unwrap();
    assert!(result["retained"].as_array().unwrap().is_empty());
    assert_eq!(result["omitted"][0]["assetId"], "source");
    assert_eq!(result["added"][0]["assetId"], "source");
    assert_ne!(
        result["omitted"][0]["sourceSha256"],
        result["added"][0]["sourceSha256"]
    );
}

#[test]
fn artifact_and_snapshot_tampering_refuse_and_unbound_sync_is_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Frozen identity").unwrap();
    seed(&mut store, json!({"numerator":30,"denominator":1}));
    assemble(
        &mut store,
        "baseline",
        json!([{"id":"a","assetId":"source","startMs":0,"endMs":2000}]),
    );
    let (job, manifest) = queued_fixture(&mut store, &root, "baseline");
    let bytes = fs::read(&manifest).unwrap();
    let video = manifest.parent().unwrap().join("video.mp4");
    let preview = manifest.parent().unwrap().join("review-sdr.mp4");
    let original_video = fs::read(&video).unwrap();
    let original_preview = fs::read(&preview).unwrap();
    fs::write(&video, b"tampered master").unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    fs::write(&video, original_video).unwrap();
    fs::write(&preview, b"tampered preview").unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    fs::write(&preview, original_preview).unwrap();
    let mut altered: Value = serde_json::from_slice(&bytes).unwrap();
    altered["snapshot"]["name"] = json!("forged snapshot");
    fs::write(&manifest, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    altered = serde_json::from_slice(&bytes).unwrap();
    altered["sequenceId"] = json!("seq_main");
    fs::write(&manifest, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    altered = serde_json::from_slice(&bytes).unwrap();
    altered["delivery"]["reviewVideo"] = json!("unrecognized-preview.mp4");
    fs::write(&manifest, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    fs::write(&manifest, &bytes).unwrap();

    // Attempt IDs belong to the job record, not arbitrary mutable manifests.
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    let original_result = store.job(&job).unwrap().result.unwrap();
    let mut forged_result = original_result.clone();
    forged_result["artifactId"] = json!("forged-1");
    let mut forged_manifest: Value = serde_json::from_slice(&bytes).unwrap();
    forged_manifest["artifactId"] = json!("forged-1");
    fs::write(&manifest, serde_json::to_vec(&forged_manifest).unwrap()).unwrap();
    db.execute(
        "UPDATE jobs SET result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&forged_result).unwrap(), job],
    )
    .unwrap();
    assert!(compare::compare_outputs(&root, &job, &job).is_err());
    db.execute(
        "UPDATE jobs SET result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&original_result).unwrap(), job],
    )
    .unwrap();
    fs::write(&manifest, &bytes).unwrap();

    // A synchronous render job does not persist its chosen sequence separately
    // from the mutable manifest, so it cannot support a trusted comparison.
    let revision = store.project().unwrap().revision;
    store.start_job("sync", revision).unwrap();
    let folder = root.join("renders/sync");
    fs::create_dir(&folder).unwrap();
    let video = folder.join("video.mp4");
    fs::write(&video, b"sync").unwrap();
    let manifest = folder.join("manifest.json");
    fs::write(&manifest,serde_json::to_vec(&json!({"artifactId":"sync","projectId":store.project().unwrap().project_id,
        "revision":revision,"sequenceId":"baseline","output":"video.mp4","snapshot":store.project().unwrap(),
        "verification":{"sha256":media::hash_file(&video).unwrap()}})).unwrap()).unwrap();
    store
        .finish_job(
            "sync",
            &json!({"artifactId":"sync","revision":revision,"path":video,"manifest":manifest}),
        )
        .unwrap();
    let result = compare::compare_outputs(&root, "sync", &job).unwrap();
    assert_eq!(result["status"], "unsupported");
    assert_eq!(result["baseline"]["sequenceBinding"], "manifest-only");
    assert!(result.get("retained").is_none());
}

#[test]
fn cross_rate_source_and_output_coordinates_remain_exact() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Fractional rate").unwrap();
    seed(&mut store, json!({"numerator":30000,"denominator":1001}));
    assemble(
        &mut store,
        "baseline",
        json!([{"id":"a","assetId":"source","startMs":1000,"endMs":2500}]),
    );
    let (baseline, _) = queued_fixture(&mut store, &root, "baseline");
    let mut candidate: Assemble =
        serde_json::from_value(json!({"outputId":"candidate","name":"candidate",
        "cuts":[{"id":"a","assetId":"source","startMs":1500,"endMs":2500}]}))
        .unwrap();
    candidate.frame_rate = Some(agent_video_workbench::assembly::FrameRate {
        numerator: 30,
        denominator: 1,
    });
    store
        .assemble(&candidate, 2, "assemble-candidate", false)
        .unwrap();
    let (candidate, _) = queued_fixture(&mut store, &root, "candidate");
    let result = compare::compare_outputs(&root, &baseline, &candidate).unwrap();
    assert_eq!(result["status"], "complete");
    same_time(&result["retained"][0]["sourceStart"], 1500, 1000);
    same_time(
        &result["retained"][0]["baselinePlacements"][0]["outputStart"],
        500,
        1000,
    );
    same_time(
        &result["retained"][0]["candidatePlacements"][0]["outputStart"],
        0,
        1000,
    );
    same_time(&result["retained"][0]["sourceEnd"], 2500, 1000);
    same_time(&result["omitted"][1]["sourceStart"], 2500, 1000);
    same_time(&result["omitted"][1]["sourceEnd"], 5003, 2000);
}

#[test]
fn composed_output_returns_explicit_unsupported_without_claiming_no_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Unsupported composition").unwrap();
    seed(&mut store, json!({"numerator":30,"denominator":1}));
    let (job, _) = queued_fixture(&mut store, &root, "seq_main");
    let result = compare::compare_outputs(&root, &job, &job).unwrap();
    assert_eq!(result["status"], "unsupported");
    assert!(
        result["limitations"][0]
            .as_str()
            .unwrap()
            .contains("plain-cut")
    );
    assert!(result.get("retained").is_none());
}

#[test]
fn dense_repetitions_refuse_instead_of_returning_unbounded_or_partial_results() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let mut store = Store::create(&root, "Dense repetition").unwrap();
    seed(&mut store, json!({"numerator":30,"denominator":1}));
    let cuts: Vec<_> = (0..100)
        .map(|i| {
            json!({"id":format!("cut{i}"),"assetId":"source",
        "startMs":i,"endMs":2000+i})
        })
        .collect();
    assemble(&mut store, "repeated", json!(cuts));
    let (job, _) = queued_fixture(&mut store, &root, "repeated");
    let error = compare::compare_outputs(&root, &job, &job).unwrap_err();
    assert!(error.to_string().contains("4000 interval placements"));
}
