use agent_video_workbench::{
    Error,
    jobs::{self, RenderInput},
    store::Store,
};
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
