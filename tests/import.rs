use agent_video_workbench::{Error, media, store::Store};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    source: PathBuf,
    marker: PathBuf,
    backend: agentcut_render::FfmpegBackend,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    Store::create(&project, "Import validation").unwrap();
    let source = temp.path().join("generated.mp4");
    fs::write(&source, b"generated import fixture, mock probe only").unwrap();
    let marker = temp.path().join("probed");
    let tool = temp.path().join("ffprobe");
    let metadata = json!({
        "format":{"format_name":"mov,mp4", "duration":"12", "start_time":"0"},
        "streams":[{"codec_type":"video", "width":64, "height":64,
            "pix_fmt":"yuv420p", "avg_frame_rate":"30/1", "r_frame_rate":"30/1",
            "sample_aspect_ratio":"1:1", "duration":"12", "start_time":"0",
            "color_primaries":"bt709", "color_transfer":"bt709", "color_space":"bt709", "color_range":"tv"}]
    });
    fs::write(
        &tool,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nprintf '%s' '{}'\n",
            marker.display(),
            metadata
        ),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    let backend = media::backend("unused-ffmpeg".into(), tool);
    Fixture {
        _temp: temp,
        project,
        source,
        marker,
        backend,
    }
}

fn assert_not_staged(f: &Fixture) {
    assert!(!f.marker.exists());
    assert!(
        fs::read_dir(f.project.join("cache"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        fs::read_dir(f.project.join("originals"))
            .unwrap()
            .next()
            .is_none()
    );
    let store = Store::open(&f.project).unwrap();
    assert_eq!(store.project().unwrap().revision, 0);
    assert!(store.imports().unwrap().is_empty());
}

#[test]
fn invalid_key_and_stale_revision_fail_before_copy_or_probe() {
    let f = fixture();
    let error = media::import(&f.project, &f.source, "source", 0, "short", &f.backend).unwrap_err();
    assert!(error.to_string().contains("at least 8"));
    assert_not_staged(&f);
    let missing = f.source.with_extension("missing");
    assert!(matches!(
        media::import(
            &f.project,
            &missing,
            "source",
            5,
            "stale-import",
            &f.backend
        ),
        Err(Error::Conflict {
            expected: 5,
            current: 0
        })
    ));
    assert_not_staged(&f);
}

#[test]
fn oversized_font_fails_without_staging() {
    let f = fixture();
    let font = f.source.with_extension("ttf");
    fs::File::create(&font)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    let error = media::import(&f.project, &font, "font", 0, "font-import", &f.backend).unwrap_err();
    assert!(error.to_string().contains("font/LUT exceeds"));
    assert_not_staged(&f);
}

#[test]
fn committed_import_replay_accepts_its_old_revision_and_detects_changed_bytes() {
    let f = fixture();
    let first = media::import(
        &f.project,
        &f.source,
        "source",
        0,
        "valid-import",
        &f.backend,
    )
    .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(
        media::import(
            &f.project,
            &f.source,
            "source",
            0,
            "valid-import",
            &f.backend
        )
        .unwrap(),
        first
    );
    let before = Store::open(&f.project).unwrap().project().unwrap();
    fs::write(&f.source, b"changed fixture content").unwrap();
    assert!(matches!(
        media::import(
            &f.project,
            &f.source,
            "source",
            0,
            "valid-import",
            &f.backend
        ),
        Err(Error::KeyConflict)
    ));
    let store = Store::open(&f.project).unwrap();
    assert_eq!(store.project().unwrap(), before);
    assert_eq!(store.history().unwrap().len(), 2);
}

#[test]
fn redirected_import_directories_are_rejected_without_touching_target() {
    for slot in ["cache", "originals"] {
        let f = fixture();
        let target = f._temp.path().join("outside");
        fs::create_dir(&target).unwrap();
        fs::remove_dir(f.project.join(slot)).unwrap();
        symlink(&target, f.project.join(slot)).unwrap();
        let error = media::import(
            &f.project,
            &f.source,
            "source",
            0,
            "safe-import",
            &f.backend,
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be a real directory"));
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        assert!(!f.marker.exists());
        assert_eq!(
            Store::open(&f.project).unwrap().project().unwrap().revision,
            0
        );
    }
}

#[test]
fn an_existing_original_symlink_is_rejected() {
    let f = fixture();
    let bytes = fs::read(&f.source).unwrap();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    symlink(&f.source, f.project.join("originals").join(&sha)).unwrap();
    let error = media::import(
        &f.project,
        &f.source,
        "source",
        0,
        "safe-import",
        &f.backend,
    )
    .unwrap_err();
    assert!(error.to_string().contains("regular file"));
    assert_eq!(fs::read(&f.source).unwrap(), bytes);
    assert_eq!(
        Store::open(&f.project).unwrap().project().unwrap().revision,
        0
    );
}
