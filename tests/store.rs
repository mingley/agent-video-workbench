use agent_video_workbench::{Error, store::Store};
use agentcut_core::{Operation, OperationBatch};
use serde_json::json;

fn rename(store: &Store, key: &str, name: &str) -> OperationBatch {
    let p = store.project().unwrap();
    serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":p.revision,"idempotencyKey":format!("request-{key}"),"operations":[{"id":"rename","op":"project.rename","params":{"name":name}}]})).unwrap()
}

#[test]
fn durable_retry_restore_and_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project");
    let mut store = Store::create(&path, "original").unwrap();
    let batch = rename(&store, "first", "edited");
    let outcome = store.apply(&batch, false).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.apply(&batch, false).unwrap(), outcome);
    assert_eq!(store.history().unwrap().len(), 2);
    let mut different = batch.clone();
    different.description = "different author intent".into();
    assert!(matches!(
        store.apply(&different, false),
        Err(Error::KeyConflict)
    ));
    store.restore(0, 1, "undo").unwrap();
    assert_eq!(store.project().unwrap().name, "original");
    store.restore(1, 2, "redo").unwrap();
    assert_eq!(store.project().unwrap().name, "edited");
    assert_eq!(store.project().unwrap().revision, 3);
    assert_eq!(store.history().unwrap().len(), 4);
}

#[test]
fn invalid_batch_and_dry_run_leave_no_history_or_request() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::create(&temp.path().join("p"), "original").unwrap();
    let valid = rename(&store, "test", "new");
    let mut invalid = valid.clone();
    invalid
        .operations
        .push(Operation::new("bad", "does.not.exist"));
    assert!(store.apply(&invalid, false).is_err());
    assert_eq!(store.project().unwrap().revision, 0);
    store.apply(&valid, true).unwrap();
    assert_eq!(store.history().unwrap().len(), 1);
    store.apply(&valid, false).unwrap();
    assert_eq!(store.project().unwrap().revision, 1);
}

#[test]
fn concurrent_writers_cannot_both_commit_same_base() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let store = Store::create(&path, "original").unwrap();
    let a = rename(&store, "a", "first");
    let b = rename(&store, "b", "second");
    drop(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [a, b]
        .into_iter()
        .map(|batch| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(&path).unwrap();
                barrier.wait();
                store.apply(&batch, false)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(Error::Conflict { .. })))
            .count(),
        1
    );
    assert_eq!(Store::open(&path).unwrap().history().unwrap().len(), 2);
}

#[test]
fn failed_request_insert_rolls_back_snapshot_and_head() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("p");
    let mut store = Store::create(&path, "original").unwrap();
    let connection = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_request BEFORE INSERT ON requests BEGIN SELECT RAISE(ABORT,'simulated disk failure'); END;").unwrap();
    assert!(store.apply(&rename(&store, "test", "new"), false).is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.project().unwrap().revision, 0);
    assert_eq!(store.history().unwrap().len(), 1);
}
