use agent_video_workbench::{jobs, storage, store::Store, studio::Edit};
use serde_json::json;
use std::{
    fs::{File, FileTimes},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, SystemTime},
};

fn age(path: &std::path::Path) {
    File::open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120)))
        .unwrap();
}

#[test]
fn automatic_retention_keeps_successful_nested_artifacts_and_originals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "retention").unwrap();
    let scratch = root.join("cache/disposable");
    std::fs::write(&scratch, b"derived").unwrap();
    age(&scratch);
    let original = root.join("originals/keep");
    std::fs::write(&original, b"original").unwrap();
    age(&original);
    let analysis = root.join("analysis/completed");
    std::fs::create_dir(&analysis).unwrap();
    let artifact = analysis.join("report.json");
    std::fs::write(&artifact, b"{}").unwrap();
    age(&analysis);
    store.start_job("retained", 0).unwrap();
    store
        .finish_job("retained", &json!({"path":artifact}))
        .unwrap();
    store
        .studio(
            &Edit::Retention {
                enabled: true,
                grace_seconds: 60,
            },
            0,
            "enable-retention",
            false,
        )
        .unwrap();
    let preview = storage::gc(&root, 60, true).unwrap();
    assert_eq!(preview["candidates"].as_array().unwrap().len(), 1);
    assert!(scratch.exists());
    let result = jobs::worker(&root, Arc::new(AtomicBool::new(false)), 0).unwrap();
    assert_eq!(result["retention"]["dryRun"], false);
    assert!(!scratch.exists());
    assert!(original.exists() && artifact.exists());
    assert_eq!(Store::open(&root).unwrap().project().unwrap().revision, 1);
}

#[test]
fn workspace_maintenance_defers_busy_projects_and_collects_idle_ones() {
    let temp = tempfile::tempdir().unwrap();
    let busy = temp.path().join("busy");
    let idle = temp.path().join("idle");
    Store::create(&busy, "busy").unwrap();
    Store::create(&idle, "idle").unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(busy.join("worker.lock"))
        .unwrap();
    lock.lock().unwrap();
    let scratch = idle.join("cache/disposable");
    std::fs::write(&scratch, b"derived").unwrap();
    age(&scratch);
    let result = storage::maintain(temp.path(), 60, false).unwrap();
    assert_eq!(result["deferredProjects"], 1);
    assert!(!scratch.exists());
    assert!(busy.join("project.sqlite").exists());
}
