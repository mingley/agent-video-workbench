use agent_video_workbench::{
    jobs::RenderInput,
    media,
    store::Store,
    thumbnails::{self, ThumbnailRequest},
};
use agentcut_render::FfmpegBackend;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    job: String,
    video: PathBuf,
    manifest: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Thumbnails").unwrap();
    let snapshot = store.project().unwrap();
    let job = store
        .enqueue(
            "queued-thumbnail-render",
            &RenderInput {
                sequence: "seq_main".into(),
                priority: 0,
                expected_revision: 0,
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
                asr: None,
                analysis: None,
            },
        )
        .unwrap()
        .id;
    let artifact = format!("{job}-1");
    let dir = root.join("renders").join(&artifact);
    fs::create_dir_all(&dir).unwrap();
    let video = dir.join("video.mp4");
    fs::write(&video, b"fake frozen video").unwrap();
    let manifest = dir.join("manifest.json");
    let verification = json!({"decoded":true,"expectedFrames":30,"sha256":media::hash_file(&video).unwrap(),
        "probe":{"streams":[{"codec_type":"video","avg_frame_rate":"30/1","nb_read_frames":"30",
            "start_pts":0,"duration_ts":15360,"time_base":"1/15360","color_primaries":"bt709",
            "color_transfer":"bt709","color_space":"bt709","color_range":"tv"}]}});
    fs::write(&manifest, serde_json::to_vec(&json!({"artifactId":artifact,"projectId":snapshot.project_id,
        "revision":snapshot.revision,"sequenceId":"seq_main","output":"video.mp4","snapshot":snapshot,
        "verification":verification,"delivery":{"reviewVideo":"video.mp4","files":[]}})).unwrap()).unwrap();
    let result = json!({"artifactId":artifact,"revision":0,"path":video,"manifest":manifest});
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), job],
    )
    .unwrap();
    Fixture {
        _temp: temp,
        root,
        job,
        video,
        manifest,
    }
}

fn request(times: &[i64]) -> ThumbnailRequest {
    ThumbnailRequest {
        schema_version: 1,
        at_ms: times.into(),
    }
}

fn backend() -> FfmpegBackend {
    FfmpegBackend::new("/no/such/ffmpeg", "/no/such/ffprobe")
}

#[test]
fn invalid_requests_are_rejected_before_reading_projects() {
    for value in [
        ThumbnailRequest {
            schema_version: 2,
            at_ms: vec![0],
        },
        request(&[]),
        request(&[-1]),
        request(&[0, 100, 200, 300, 400, 500, 600, 700, 800]),
    ] {
        let error = thumbnails::create(
            Path::new("/no/such/project"),
            "missing",
            Path::new("/no/such/output"),
            &value,
            &backend(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "E_INVALID_REQUEST", "{error}");
    }
    assert!(
        serde_json::from_value::<ThumbnailRequest>(
            json!({"schemaVersion":1,"atMs":[0],"extra":true})
        )
        .is_err()
    );
}

#[test]
fn out_of_range_duplicate_frames_and_existing_destinations_refuse_before_encoding() {
    let f = fixture();
    for (times, expected) in [(&[1000][..], "outside"), (&[34, 35][..], "distinct")] {
        let output = f._temp.path().join("output");
        let error =
            thumbnails::create(&f.root, &f.job, &output, &request(times), &backend()).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert!(!output.exists());
    }
    let output = f._temp.path().join("existing");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"untouched").unwrap();
    let error =
        thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"untouched");
}

#[test]
fn refuses_tampered_video_and_uncertain_timing_without_creating_destination() {
    let f = fixture();
    let output = f._temp.path().join("output");
    fs::write(&f.video, b"changed").unwrap();
    let error =
        thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).unwrap_err();
    assert!(error.to_string().contains("bytes changed"));
    assert!(!output.exists());
    fs::write(&f.video, b"fake frozen video").unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.manifest).unwrap()).unwrap();
    manifest["verification"]["probe"]["streams"][0]["duration_ts"] = json!(15359);
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error =
        thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).unwrap_err();
    assert!(error.to_string().contains("duration is unavailable"));
    assert!(!output.exists());
}

#[test]
fn refuses_to_label_unverified_hdr_as_an_sdr_review_thumbnail() {
    let f = fixture();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.manifest).unwrap()).unwrap();
    manifest["verification"]["probe"]["streams"][0]["color_transfer"] = json!("smpte2084");
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let output = f._temp.path().join("output");
    let error =
        thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).unwrap_err();
    assert!(error.to_string().contains("Rec.709 SDR"));
    assert!(!output.exists());
}

#[test]
fn encoder_failure_cleans_staging_and_preserves_existing_artifacts() {
    let f = fixture();
    let output = f._temp.path().join("output");
    let before = media::hash_file(&f.video).unwrap();
    assert!(thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).is_err());
    assert!(!output.exists());
    assert_eq!(media::hash_file(&f.video).unwrap(), before);
    assert!(!fs::read_dir(f._temp.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".avw-thumbnails-")
    }));
}

#[test]
fn decoded_frame_with_wrong_pts_is_never_published() {
    let f = fixture();
    let fake = f._temp.path().join("fake-ffmpeg");
    fs::write(&fake, "#!/bin/sh\necho '[Parsed_showinfo_0 @ x] config in time_base: 1/30, frame_rate: 30/1' >&2\necho '[Parsed_showinfo_0 @ x] n: 0 pts: 1 pts_time:0.033333' >&2\nexit 0\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let output = f._temp.path().join("output");
    let backend = FfmpegBackend::new(fake, PathBuf::from("/no/such/ffprobe"));
    let error = thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend).unwrap_err();
    assert!(error.to_string().contains("timestamp differs"));
    assert!(!output.exists());
    assert!(!fs::read_dir(f._temp.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".avw-thumbnails-")
    }));
}

#[test]
fn synchronous_renders_without_job_bound_sequence_are_refused() {
    let f = fixture();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.manifest).unwrap()).unwrap();
    manifest["artifactId"] = json!(f.job);
    fs::write(&f.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let db = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    let result = json!({"artifactId":f.job,"revision":0,"path":f.video,"manifest":f.manifest});
    db.execute(
        "UPDATE jobs SET input=NULL,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), f.job],
    )
    .unwrap();
    let output = f._temp.path().join("output");
    let error =
        thumbnails::create(&f.root, &f.job, &output, &request(&[0]), &backend()).unwrap_err();
    assert!(error.to_string().contains("queued render"));
    assert!(!output.exists());
}
