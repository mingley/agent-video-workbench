use agent_video_workbench::{host_readiness, media, service::Service};
use std::path::{Path, PathBuf};

fn service(root: PathBuf) -> Service {
    Service {
        root: Some(root),
        backend: media::backend(
            "/missing/readiness-ffmpeg".into(),
            "/missing/readiness-ffprobe".into(),
        ),
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    }
}

#[test]
fn reports_independent_filesystem_and_media_results_without_claiming_host_support() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let service = service(root.path().into());
    let report = host_readiness::check(&service, Path::new("workspace")).unwrap();
    assert_eq!(report["status"], "needs-attention");
    assert_eq!(report["workspace"]["path"], workspace.to_str().unwrap());
    assert_eq!(report["workspace"]["writable"]["status"], "pass");
    assert_eq!(report["workspace"]["freeSpace"]["status"], "measured");
    let free = report["workspace"]["freeSpace"]["availableBytesDecimal"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    if free <= 9_007_199_254_740_991 {
        assert_eq!(
            report["workspace"]["freeSpace"]["availableBytes"].as_u64(),
            Some(free)
        );
    } else {
        assert!(report["workspace"]["freeSpace"]["availableBytes"].is_null());
    }
    assert_eq!(report["media"]["status"], "fail");
    assert_eq!(report["media"]["code"], "E_IO");
    assert!(
        report["capabilities"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "transcript-search")
    );
    for field in [
        "modelImageConsumption",
        "modelAudioConsumption",
        "accountCompatibility",
    ] {
        assert_eq!(report["host"][field]["status"], "unknown");
    }
    assert_eq!(report["workspace"]["persistence"]["status"], "unknown");
    assert_eq!(
        report["workspace"]["filesystemLocking"]["status"],
        "unknown"
    );
    assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 0);
}

#[test]
fn refuses_nonexistent_files_and_paths_outside_the_scoped_workspace() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let service = service(root.path().into());
    let file = root.path().join("file");
    std::fs::write(&file, b"keep me").unwrap();
    assert!(
        host_readiness::check(&service, &file)
            .unwrap_err()
            .to_string()
            .contains("existing directory")
    );
    assert!(
        host_readiness::check(&service, &root.path().join("missing"))
            .unwrap_err()
            .to_string()
            .contains("existing directory")
    );
    assert!(
        host_readiness::check(&service, outside.path())
            .unwrap_err()
            .to_string()
            .contains("outside the authorized service root")
    );
    let link = root.path().join("outside-link");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(
        host_readiness::check(&service, &link)
            .unwrap_err()
            .to_string()
            .contains("outside the authorized service root")
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"keep me");
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn reports_unwritable_workspace_without_hiding_the_media_result() {
    let service = service("/proc".into());
    let report = host_readiness::check(&service, Path::new("/proc")).unwrap();
    assert_eq!(report["workspace"]["writable"]["status"], "fail");
    assert_eq!(report["workspace"]["writable"]["stage"], "create");
    assert_eq!(report["media"]["status"], "fail");
    assert_eq!(report["status"], "needs-attention");
}
