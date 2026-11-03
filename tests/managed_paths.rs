use agent_video_workbench::{
    analysis::{self, Input, Task},
    inspect, mcp, media,
    process::Uncontrolled,
    service::Service,
    store::Store,
};
use agentcut_core::OperationBatch;
use agentcut_render::FfmpegBackend;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    project: PathBuf,
    outside: PathBuf,
    marker: PathBuf,
    backend: FfmpegBackend,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let project = workspace.join("project");
    let mut store = Store::create(&project, "Managed paths").unwrap();
    let source = b"generated source bytes";
    let sha = format!("{:x}", Sha256::digest(source));
    fs::write(project.join("originals").join(&sha), source).unwrap();
    let current = store.project().unwrap();
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":current.project_id,"baseRevision":0,"idempotencyKey":"add-source",
        "operations":[{"id":"source","op":"asset.add","params":{
            "id":"source","uri":format!("originals/{sha}"),"kind":"video",
            "fingerprint":{"strategy":"sha256","sizeBytes":source.len(),"sha256":sha},
            "metadata":{"status":"available","duration":{"value":10,"rate":{"numerator":1,"denominator":1}},
                "video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},
                    "pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0,
                    "colorPrimaries":"bt709","colorTransfer":"bt709","colorMatrix":"bt709","colorRange":"tv"}}
        }}]})).unwrap();
    store.apply(&batch, false).unwrap();
    drop(store);
    let marker = temp.path().join("tool-called");
    let ffmpeg = temp.path().join("ffmpeg");
    fs::write(&ffmpeg, format!("#!/bin/sh\nprintf x >> '{}'\nif [ \"$1\" = -version ]; then echo fixture; exit 0; fi\nfor last do :; done\nprintf image > \"$last\"\n", marker.display())).unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
    let ffprobe = temp.path().join("ffprobe");
    fs::write(
        &ffprobe,
        "#!/bin/sh\nprintf '%s' '{\"frames\":[{\"best_effort_timestamp_time\":\"0.000\"}]}'\n",
    )
    .unwrap();
    fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o755)).unwrap();
    let backend = media::backend(ffmpeg, ffprobe);
    Fixture {
        _temp: temp,
        workspace,
        project,
        outside,
        marker,
        backend,
    }
}

fn analysis_input() -> Input {
    Input {
        task: Task::FrameIndex {
            asset_id: "source".into(),
            start_ms: 0,
            end_ms: 1000,
        },
        provider: None,
    }
}

fn analyze(f: &Fixture) -> agent_video_workbench::Result<serde_json::Value> {
    let project = Store::open(&f.project)?.project()?;
    analysis::run(
        &f.project,
        &project,
        &analysis_input(),
        &f.backend,
        &mut Uncontrolled,
    )
}

fn frame(f: &Fixture) -> agent_video_workbench::Result<serde_json::Value> {
    inspect::inspect(
        &f.project,
        "source",
        inspect::Kind::Frame,
        0,
        Some(1000),
        &f.backend,
    )
}

#[test]
fn frame_and_analysis_refuse_redirected_project_directories_before_tool_execution() {
    for directory in ["analysis", "originals", "cache"] {
        let f = fixture();
        fs::remove_dir_all(f.project.join(directory)).unwrap();
        symlink(&f.outside, f.project.join(directory)).unwrap();
        let error = analyze(&f).unwrap_err().to_string();
        assert!(
            error.contains(&format!("project {directory} must be a real directory")),
            "{error}"
        );
        assert!(!f.marker.exists());
        assert_eq!(fs::read_dir(&f.outside).unwrap().count(), 0);
        if directory != "cache" {
            let error = frame(&f).unwrap_err().to_string();
            assert!(
                error.contains(&format!("project {directory} must be a real directory")),
                "{error}"
            );
            assert!(!f.marker.exists());
            assert_eq!(fs::read_dir(&f.outside).unwrap().count(), 0);
        }
    }
}

#[test]
fn mcp_frame_refuses_redirected_analysis_without_creating_external_artifacts() {
    let f = fixture();
    fs::remove_dir_all(f.project.join("analysis")).unwrap();
    symlink(&f.outside, f.project.join("analysis")).unwrap();
    let service = Service {
        root: Some(f.workspace.clone()),
        backend: f.backend,
        executable: "avw".into(),
        asr: None,
        downloads: Default::default(),
        provider: None,
    };
    let request = mcp::MediaRequest::Frame {
        project: "project".into(),
        asset_id: "source".into(),
        at_ms: 0,
    };
    let error = mcp::media_request(&service, request, "2025-11-25")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("project analysis must be a real directory"),
        "{error}"
    );
    assert!(!f.marker.exists());
    assert_eq!(fs::read_dir(&f.outside).unwrap().count(), 0);
}

#[test]
fn cached_frame_rejects_symlinked_manifest_and_image() {
    let f = fixture();
    let first = frame(&f).unwrap();
    assert_eq!(first["cacheHit"], false);
    assert_eq!(frame(&f).unwrap()["cacheHit"], true);
    let image = PathBuf::from(first["path"].as_str().unwrap());
    let manifest = image.with_extension("json");
    let external_manifest = f.outside.join("manifest.json");
    fs::copy(&manifest, &external_manifest).unwrap();
    fs::remove_file(&manifest).unwrap();
    symlink(&external_manifest, &manifest).unwrap();
    assert!(
        frame(&f)
            .unwrap_err()
            .to_string()
            .contains("cached inspection manifest")
    );
    fs::remove_file(&manifest).unwrap();
    fs::copy(&external_manifest, &manifest).unwrap();
    let external_image = f.outside.join("image.png");
    fs::copy(&image, &external_image).unwrap();
    fs::remove_file(&image).unwrap();
    symlink(&external_image, &image).unwrap();
    assert!(
        frame(&f)
            .unwrap_err()
            .to_string()
            .contains("cached inspection frame")
    );
}

#[test]
fn cached_analysis_rejects_symlinked_directory_result_manifest_and_attachment() {
    let f = fixture();
    let first = analyze(&f).unwrap();
    assert_eq!(first["cacheHit"], false);
    assert_eq!(analyze(&f).unwrap()["cacheHit"], true);
    let result = PathBuf::from(first["path"].as_str().unwrap());
    let manifest = PathBuf::from(first["manifest"].as_str().unwrap());
    for (path, description) in [
        (&manifest, "cached analysis manifest"),
        (&result, "cached analysis result"),
    ] {
        let external = f.outside.join(path.file_name().unwrap());
        fs::copy(path, &external).unwrap();
        fs::remove_file(path).unwrap();
        symlink(&external, path).unwrap();
        assert!(analyze(&f).unwrap_err().to_string().contains(description));
        fs::remove_file(path).unwrap();
        fs::copy(&external, path).unwrap();
    }
    let external = f.outside.join("attachment.bin");
    fs::write(&external, b"outside attachment").unwrap();
    let attachment = result.parent().unwrap().join("attachment.bin");
    symlink(&external, &attachment).unwrap();
    let mut manifest_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    manifest_json["files"] =
        json!([{"name":"attachment.bin","sha256":media::hash_file(&external).unwrap()}]);
    fs::write(&manifest, serde_json::to_vec(&manifest_json).unwrap()).unwrap();
    assert!(
        analyze(&f)
            .unwrap_err()
            .to_string()
            .contains("cached analysis attachment")
    );
    let cache_dir = result.parent().unwrap();
    let relocated = f.outside.join("cache-dir");
    fs::rename(cache_dir, &relocated).unwrap();
    symlink(&relocated, cache_dir).unwrap();
    assert!(
        analyze(&f)
            .unwrap_err()
            .to_string()
            .contains("cached analysis must be a real directory")
    );
}

#[test]
fn store_rejects_symlinked_database_without_changing_external_project() {
    let f = fixture();
    let external = f.outside.join("external-project");
    drop(Store::create(&external, "external").unwrap());
    let external_db = external.join("project.sqlite");
    let before = fs::read(&external_db).unwrap();
    fs::remove_file(f.project.join("project.sqlite")).unwrap();
    symlink(&external_db, f.project.join("project.sqlite")).unwrap();
    let error = Store::open(&f.project).err().unwrap().to_string();
    assert!(
        error.contains("project.sqlite must be a regular file"),
        "{error}"
    );
    assert_eq!(fs::read(&external_db).unwrap(), before);
    assert_eq!(
        Store::open(&external).unwrap().project().unwrap().name,
        "external"
    );
}
