use agent_video_workbench::{
    Error,
    jobs::{self, RenderInput},
    store::Store,
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn input() -> RenderInput {
    RenderInput {
        sequence: "seq_main".into(),
        priority: 0,
        expected_revision: 0,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: None,
    }
}

fn analysis_input(task: Value) -> RenderInput {
    let mut config = input();
    config.expected_revision = 1;
    config.analysis = Some(serde_json::from_value(json!({"task": task})).unwrap());
    config
}

#[test]
fn invalid_analysis_is_rejected_atomically_and_valid_jobs_replay_after_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("p");
    let mut store = Store::create(&root, "analysis queue").unwrap();
    let project = store.project().unwrap();
    let seed: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0", "projectId":project.project_id, "baseRevision":0,
        "idempotencyKey":"seed-analysis", "operations":[{"id":"add-source", "op":"asset.add",
            "params":{"id":"source", "uri":"originals/test", "kind":"video", "metadata":{
                "status":"available", "duration":{"value":10,"rate":{"numerator":1,"denominator":1}},
                "video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},
                    "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0}
            }}}]
    })).unwrap();
    store.apply(&seed, false).unwrap();
    let before = store.project().unwrap();
    let history = store.history().unwrap();
    let invalid = [
        (
            "retry-key",
            json!({"kind":"proxy","assetId":"missing","startMs":0,"endMs":1000,"width":320,"height":180}),
        ),
        (
            "bad-range",
            json!({"kind":"frame-index","assetId":"source","startMs":0,"endMs":10002}),
        ),
        (
            "bad-canvas",
            json!({"kind":"proxy","assetId":"source","startMs":0,"endMs":1000,"width":321,"height":180}),
        ),
        (
            "missing-provider",
            json!({"kind":"provider","assetId":"source","task":"semantic","parameters":{}}),
        ),
    ];
    for (key, task) in invalid {
        assert!(store.enqueue(key, &analysis_input(task)).is_err(), "{key}");
        assert_eq!(store.project().unwrap(), before);
        assert_eq!(store.history().unwrap(), history);
        assert!(store.jobs().unwrap().is_empty());
    }
    let connection = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM job_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);

    // A failed request did not reserve its key. A valid retry creates one job.
    let valid = analysis_input(
        json!({"kind":"proxy","assetId":"source","startMs":0,"endMs":1000,"width":320,"height":180}),
    );
    let queued = store.enqueue("retry-key", &valid).unwrap();
    assert_eq!(queued.state, "queued");
    assert_eq!(store.jobs().unwrap().len(), 1);

    let renamed: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0", "projectId":before.project_id, "baseRevision":1,
        "idempotencyKey":"rename-after-queue", "operations":[{"id":"rename","op":"project.rename","params":{"name":"later edit"}}]
    })).unwrap();
    store.apply(&renamed, false).unwrap();
    assert_eq!(store.project().unwrap().revision, 2);
    assert_eq!(store.enqueue("retry-key", &valid).unwrap().id, queued.id);
    assert_eq!(store.jobs().unwrap().len(), 1);
    assert!(matches!(
        store.enqueue("different-key", &valid),
        Err(Error::Conflict { .. })
    ));
    drop(store);
    let mut reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.enqueue("retry-key", &valid).unwrap().id, queued.id);
}

#[test]
fn queued_priority_is_stable_and_batch_validation_is_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("p");
    let mut store = Store::create(&root, "priority").unwrap();
    let mut missing = input();
    missing.sequence = "missing".into();
    assert!(
        store
            .enqueue_batch("invalid-batch", &[input(), missing])
            .is_err()
    );
    assert!(store.jobs().unwrap().is_empty());
    let connection = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    connection.execute_batch("CREATE TABLE test_claims(id TEXT); CREATE TRIGGER trace_claim AFTER UPDATE OF state ON jobs WHEN NEW.state='running' BEGIN INSERT INTO test_claims VALUES(NEW.id); END;").unwrap();
    let mut high = input();
    high.priority = 20;
    let mut low = input();
    low.priority = -20;
    let low = store.enqueue("low-priority", &low).unwrap();
    let normal = store.enqueue("normal-priority", &input()).unwrap();
    let high = store.enqueue("high-priority", &high).unwrap();
    jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap();
    let mut statement = connection
        .prepare("SELECT id FROM test_claims ORDER BY rowid")
        .unwrap();
    let order: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(order, vec![high.id, normal.id, low.id]);
}
#[test]
fn queue_replays_conflicts_cancels_and_retries_without_new_logical_job() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("p");
    let mut store = Store::create(&root, "queue").unwrap();
    let a = store.enqueue("render-1", &input()).unwrap();
    assert_eq!(store.enqueue("render-1", &input()).unwrap().id, a.id);
    let mut different = input();
    different.sequence = "different".into();
    assert!(matches!(
        store.enqueue("render-1", &different),
        Err(Error::KeyConflict)
    ));
    assert_eq!(store.cancel_job(&a.id).unwrap().state, "cancelled");
    assert_eq!(store.retry_job(&a.id).unwrap().state, "queued");
    assert!(store.retry_job(&a.id).is_err());
    store.cancel_job(&a.id).unwrap();
    let mut stale = input();
    stale.expected_revision = 10;
    assert!(matches!(
        store.enqueue("render-2", &stale),
        Err(Error::Conflict { .. })
    ));
    drop(store);
    assert_eq!(
        Store::open(&root).unwrap().job(&a.id).unwrap().state,
        "cancelled"
    );
}

#[test]
fn owner_lock_prevents_recovery_of_a_live_worker() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("p");
    let store = Store::create(&root, "queue").unwrap();
    store.start_job("sync", 0).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("worker.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(
        jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap()["worker"],
        "already-running"
    );
    assert_eq!(store.job("sync").unwrap().state, "running");
    drop(lock);
    jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap();
    assert_eq!(store.job("sync").unwrap().state, "interrupted");
}

#[test]
fn worker_failure_is_durable_and_retries_increment_attempts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("p");
    let mut store = Store::create(&root, "queue").unwrap();
    let mut config = input();
    config.ffmpeg = Path::new("/does/not/exist").into();
    let job = store.enqueue("failure", &config).unwrap();
    jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap();
    assert_eq!(store.job(&job.id).unwrap().state, "failed");
    assert_eq!(store.job(&job.id).unwrap().attempt, 1);
    store.retry_job(&job.id).unwrap();
    jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap();
    assert_eq!(store.job(&job.id).unwrap().attempt, 2);
}
