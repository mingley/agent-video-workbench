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
        expected_revision: 0,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
    }
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
