use serde_json::{Value, json};
use std::process::Command;

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_avw"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn result_pointer_extracts_strings_numbers_objects_and_errors() {
    let caps = cli(&["--result-pointer", "/version", "capabilities"]);
    assert!(caps.status.success());
    assert_eq!(
        String::from_utf8(caps.stdout).unwrap().trim(),
        env!("CARGO_PKG_VERSION")
    );
    let limit = cli(&[
        "capabilities",
        "--result-pointer",
        "/limits/outputDurationSeconds",
    ]);
    assert!(limit.status.success());
    assert_eq!(String::from_utf8(limit.stdout).unwrap().trim(), "3600");
    let limits = cli(&["--result-pointer", "/limits", "capabilities"]);
    let result: Value = serde_json::from_slice(&limits.stdout).unwrap();
    assert_eq!(result["outputDurationSeconds"], 3600);
    let missing = cli(&["--result-pointer", "/absent", "capabilities"]);
    assert!(!missing.status.success());
    let result: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(result["error"]["code"], "E_RESULT_POINTER");
    assert_eq!(result["result"]["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn invalid_pointer_is_rejected_before_mutation_and_errors_stay_structured() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let path = project.to_str().unwrap();
    let invalid = cli(&["--result-pointer", "id", "create", path, "--name", "test"]);
    assert!(!invalid.status.success());
    assert!(!project.exists());
    let invalid_escape = cli(&[
        "--result-pointer",
        "/bad~escape",
        "create",
        path,
        "--name",
        "test",
    ]);
    assert!(!invalid_escape.status.success());
    assert!(!project.exists());
    let missing = cli(&["--result-pointer", "/revision", "status", path]);
    assert!(!missing.status.success());
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(error["ok"], json!(false));
    assert!(error["error"]["code"].is_string());
}
