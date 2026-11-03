use agent_video_workbench::{
    delivery_batch::{self, BatchBrief},
    jobs::RenderInput,
    media,
    store::Store,
};
use agentcut_core::{OperationBatch, Project};
use agentcut_render::FfmpegBackend;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    store: Store,
    batch: String,
    children: Vec<String>,
    snapshot: Project,
}

fn input(sequence: &str, revision: u64) -> RenderInput {
    RenderInput {
        sequence: sequence.into(),
        priority: 0,
        expected_revision: revision,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: None,
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Batch delivery").unwrap();
    let project = store.project().unwrap();
    let operation: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
      "projectId":project.project_id,"baseRevision":0,"idempotencyKey":"create-batch-sequences", "operations":[
        {"id":"one","op":"sequence.add","params":{"id":"square","name":"Square","width":360,"height":360,"frameRate":{"numerator":30,"denominator":1}}},
        {"id":"two","op":"sequence.add","params":{"id":"landscape","name":"Landscape","width":640,"height":360,"frameRate":{"numerator":30,"denominator":1}}}
      ]})).unwrap();
    store.apply(&operation, false).unwrap();
    let mut snapshot = store.project().unwrap();
    snapshot.name = "SECRET_PROJECT_NAME_8b1d".into();
    snapshot.require_sequence_mut("seq_main").unwrap().name = "SECRET_SELECTED_TITLE_287a".into();
    snapshot.require_sequence_mut("landscape").unwrap().name = "SECRET_OTHER_SEQUENCE_7cab".into();
    snapshot.extensions.insert(
        "private.extension".into(),
        json!({"note":"SECRET_EXTENSION_6da2"}),
    );
    snapshot.extensions.insert("avw.transcripts.v1".into(),json!([{"assetId":"unrelated","language":"en",
        "provider":"fixture","cues":[{"id":"private","startMs":0,"endMs":1000,"text":"SECRET_TRANSCRIPT_97e1"}]}]));
    snapshot.extensions.insert(
        "avw.reviews".into(),
        json!([{"id":"old-private-review","text":"SECRET_REVIEW_391d"}]),
    );
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE revisions SET snapshot=?1 WHERE id=1",
        [serde_json::to_string(&snapshot).unwrap()],
    )
    .unwrap();
    let batch = store
        .enqueue_batch(
            "batch-key",
            &[
                input("seq_main", 1),
                input("square", 1),
                input("landscape", 1),
            ],
        )
        .unwrap();
    let children = batch.result.unwrap()["jobIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    Fixture {
        _temp: temp,
        root,
        store,
        batch: batch.id,
        children,
        snapshot,
    }
}

fn mark_succeeded(f: &Fixture, index: usize) -> PathBuf {
    let id = &f.children[index];
    let job = f.store.job(id).unwrap();
    let sequence = &job.input.unwrap().sequence;
    let artifact = format!("{id}-1");
    let dir = f.root.join("renders").join(&artifact);
    fs::create_dir_all(&dir).unwrap();
    let master = dir.join("video.mp4");
    fs::write(&master, format!("output for {sequence}")).unwrap();
    let sheet = dir.join("sheet.png");
    fs::write(&sheet, b"generated contact sheet fixture").unwrap();
    let extra = dir.join("private-note.txt");
    fs::write(&extra, b"SECRET_EXTRA_FILE_d3e6").unwrap();
    let manifest_path = dir.join("manifest.json");
    let manifest = json!({"artifactId":artifact,"projectId":f.snapshot.project_id,
        "revision":f.snapshot.revision,"sequenceId":sequence,"output":"video.mp4","snapshot":f.snapshot,
        "verification":{"sha256":media::hash_file(&master).unwrap(),"decoded":true,"expectedFrames":30,
            "probe":{"streams":[{"codec_type":"video","avg_frame_rate":"30/1","nb_read_frames":"30",
                "start_pts":0,"duration_ts":15360,"time_base":"1/15360"}]}},
        "contactSheet":{"sha256":media::hash_file(&sheet).unwrap()},
        "delivery":{"files":[{"path":"private-note.txt","sha256":media::hash_file(&extra).unwrap()}]},
        "editPreflight":{"editPlan":{"brief":{"objective":"SECRET_EDIT_OBJECTIVE_0d2f"},
            "decisions":[{"reason":"SECRET_CUT_NOTE_81ef"}]}}});
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let result = json!({"artifactId":artifact,"revision":f.snapshot.revision,"manifest":manifest_path,"path":master});
    let conn = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    conn.execute(
        "UPDATE jobs SET state='succeeded', attempt=1, result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), id],
    )
    .unwrap();
    master
}

fn mark_failed(f: &Fixture, index: usize) {
    let conn = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    conn.execute(
        "UPDATE jobs SET state='failed', error='fixture failure' WHERE id=?1",
        [&f.children[index]],
    )
    .unwrap();
}

fn request() -> BatchBrief {
    serde_json::from_value(json!({"schemaVersion":1,"outputs":[
      {"sequenceId":"seq_main","brief":{"schemaVersion":1,"maxBytes":100},
       "metadata":{"title":"Public title","description":"A caller supplied description\nSecond line","tags":["demo","video"]}},
      {"sequenceId":"square","brief":{"schemaVersion":1,"maxBytes":2}},
      {"sequenceId":"landscape","brief":{"schemaVersion":1,"maxBytes":100}}
    ]})).unwrap()
}

fn backend() -> FfmpegBackend {
    FfmpegBackend::new("/missing-ffmpeg", "/missing-ffprobe")
}

fn assert_tree_excludes(path: &Path, secrets: &[&str]) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_tree_excludes(&path, secrets);
        } else {
            let bytes = fs::read(&path).unwrap();
            for secret in secrets {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "secret {secret} leaked in {}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn validates_all_briefs_before_reading_the_project() {
    for value in [
        json!({"schemaVersion":2,"outputs":[]}),
        json!({"schemaVersion":1,"outputs":[]}),
        json!({"schemaVersion":1,"outputs":[{"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":0}}]}),
        json!({"schemaVersion":1,"outputs":[{"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":10}},
          {"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":10}}]}),
        json!({"schemaVersion":1,"outputs":[{"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":10},
          "metadata":{"title":"line\nbreak"}}]}),
        json!({"schemaVersion":1,"outputs":[{"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":10},
          "metadata":{"tags":[""]}}]}),
    ] {
        let input: BatchBrief = serde_json::from_value(value).unwrap();
        let error =
            delivery_batch::check(Path::new("/no/such/project"), "missing", &input, &backend())
                .unwrap_err();
        assert_eq!(error.code(), "E_INVALID_REQUEST", "{error}");
    }
    let too_long = "x".repeat(1001);
    let oversized: BatchBrief = serde_json::from_value(json!({"schemaVersion":1,"outputs":[
      {"sequenceId":"x","brief":{"schemaVersion":1,"maxBytes":10},"metadata":{"title":too_long}}
    ]}))
    .unwrap();
    assert!(oversized.validate().is_err());
    assert!(
        serde_json::from_value::<BatchBrief>(json!({"schemaVersion":1,"outputs":[],"extra":true}))
            .is_err()
    );
}

#[test]
fn per_output_checks_keep_successes_and_caller_metadata_when_siblings_fail() {
    let f = fixture();
    let first = mark_succeeded(&f, 0);
    mark_succeeded(&f, 1);
    mark_failed(&f, 2);
    let before = f.store.project().unwrap();
    let history = f.store.history().unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["status"], "fail");
    assert_eq!(report["frozenRevision"], 1);
    assert_eq!(report["items"][0]["sequenceId"], "seq_main");
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][0]["metadata"]["title"], "Public title");
    assert_eq!(report["items"][0]["metadataSource"], "caller-supplied");
    assert_eq!(
        report["items"][0]["report"]["artifact"]["sha256"],
        media::hash_file(&first).unwrap()
    );
    assert_eq!(report["items"][1]["status"], "fail");
    assert_eq!(
        report["items"][1]["report"]["checks"][0]["constraint"],
        "maxBytes"
    );
    assert_eq!(report["items"][2]["status"], "fail");
    assert_eq!(report["items"][2]["jobError"], "fixture failure");
    assert!(report["unmatchedBriefs"].as_array().unwrap().is_empty());
    assert_eq!(f.store.project().unwrap(), before);
    assert_eq!(f.store.history().unwrap(), history);
}

#[test]
fn all_pass_and_later_project_edits_do_not_change_the_frozen_batch_report() {
    let mut f = fixture();
    for index in 0..3 {
        mark_succeeded(&f, index);
    }
    let request: BatchBrief = serde_json::from_value(json!({"schemaVersion":1,"outputs":[
      {"sequenceId":"seq_main","brief":{"schemaVersion":1,"maxBytes":100}},
      {"sequenceId":"square","brief":{"schemaVersion":1,"maxBytes":100}},
      {"sequenceId":"landscape","brief":{"schemaVersion":1,"maxBytes":100}}
    ]}))
    .unwrap();
    let first = delivery_batch::check(&f.root, &f.batch, &request, &backend()).unwrap();
    assert_eq!(first["status"], "pass");
    assert!(
        first["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["status"] == "pass")
    );
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":f.snapshot.project_id,"baseRevision":1,"idempotencyKey":"rename-after-batch",
        "operations":[{"id":"rename","op":"project.rename","params":{"name":"Later project"}}]
    }))
    .unwrap();
    f.store.apply(&batch, false).unwrap();
    assert_eq!(f.store.project().unwrap().revision, 2);
    let after = delivery_batch::check(&f.root, &f.batch, &request, &backend()).unwrap();
    assert_eq!(after, first);
}

#[test]
fn tampered_missing_and_unfinished_children_do_not_hide_verified_siblings() {
    let f = fixture();
    mark_succeeded(&f, 0);
    let tampered = mark_succeeded(&f, 1);
    fs::write(tampered, b"altered").unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["status"], "unknown");
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert!(
        report["items"][1]["verificationError"]["message"]
            .as_str()
            .unwrap()
            .contains("bytes changed")
    );
    assert_eq!(report["items"][2]["status"], "unknown");

    let conn = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    conn.execute("DELETE FROM jobs WHERE id=?1", [&f.children[2]])
        .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][2]["status"], "unknown");
    assert_eq!(report["items"][2]["reason"], "child job is unavailable");
    assert_eq!(report["unmatchedBriefs"][0]["sequenceId"], "landscape");
}

#[test]
fn duplicate_sequence_revision_or_nonrender_inputs_never_pass() {
    let f = fixture();
    for i in 0..3 {
        mark_succeeded(&f, i);
    }
    let conn = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    let original = f.store.job(&f.children[1]).unwrap().input.unwrap();
    let mut duplicate = original.clone();
    duplicate.sequence = "seq_main".into();
    conn.execute(
        "UPDATE jobs SET input=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&duplicate).unwrap(), f.children[1]],
    )
    .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["items"][0]["status"], "unknown");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert_eq!(report["items"][2]["status"], "pass");
    assert_eq!(report["status"], "unknown");

    let mut stale = original.clone();
    stale.expected_revision = 0;
    conn.execute(
        "UPDATE jobs SET input=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&stale).unwrap(), f.children[1]],
    )
    .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert_eq!(report["items"][2]["status"], "pass");

    let mut nonrender = original.clone();
    nonrender.analysis = Some(
        serde_json::from_value(json!({"task":{"kind":"proxy",
        "assetId":"source","startMs":0,"endMs":1000,"width":320,"height":180}}))
        .unwrap(),
    );
    conn.execute(
        "UPDATE jobs SET input=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&nonrender).unwrap(), f.children[1]],
    )
    .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert_eq!(report["items"][2]["status"], "pass");

    conn.execute(
        "UPDATE jobs SET input=?1, revision=0 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&original).unwrap(), f.children[1]],
    )
    .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert_eq!(report["items"][2]["status"], "pass");
}

#[test]
fn duplicate_child_ids_are_never_reported_as_verified_outputs() {
    let f = fixture();
    for index in 0..3 {
        mark_succeeded(&f, index);
    }
    let conn = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    let altered =
        json!({"kind":"render-batch","jobIds":[f.children[0],f.children[0],f.children[2]]});
    conn.execute(
        "UPDATE jobs SET result=?1 WHERE id=?2",
        rusqlite::params![altered.to_string(), f.batch],
    )
    .unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request(), &backend()).unwrap();
    assert_eq!(report["status"], "unknown");
    assert_eq!(report["items"][0]["status"], "unknown");
    assert_eq!(report["items"][1]["status"], "unknown");
    assert_eq!(report["items"][2]["status"], "pass");
}

#[test]
fn missing_briefs_and_briefs_for_unmatched_sequences_are_explicit() {
    let f = fixture();
    for i in 0..3 {
        mark_succeeded(&f, i);
    }
    let request: BatchBrief = serde_json::from_value(json!({"schemaVersion":1,"outputs":[
      {"sequenceId":"seq_main","brief":{"schemaVersion":1,"maxBytes":100}},
      {"sequenceId":"other","brief":{"schemaVersion":1,"maxBytes":100},"metadata":{"title":"Not part of batch"}}
    ]})).unwrap();
    let report = delivery_batch::check(&f.root, &f.batch, &request, &backend()).unwrap();
    assert_eq!(report["status"], "unknown");
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(
        report["items"][1]["reason"],
        "no delivery brief was supplied for this output"
    );
    assert_eq!(report["items"][2]["status"], "unknown");
    assert_eq!(report["unmatchedBriefs"][0]["sequenceId"], "other");
}

#[test]
fn checked_package_retains_good_siblings_and_embeds_reports_and_metadata() {
    let f = fixture();
    let first = mark_succeeded(&f, 0);
    let second = mark_succeeded(&f, 1);
    mark_failed(&f, 2);
    let destination = f._temp.path().join("delivery");
    let mut brief = request();
    brief.outputs[0].metadata.as_mut().unwrap().title = Some("<script>no markup</script>".into());
    let response =
        delivery_batch::package(&f.root, &f.batch, &destination, &brief, &backend()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&destination).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode & 0o077,
            0,
            "published package grants group/other access"
        );
    }
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(response["manifest"], report);
    assert_eq!(report["package"]["state"], "partial");
    assert_eq!(report["package"]["packagedOutputs"], 2);
    assert_eq!(report["status"], "fail");
    assert_eq!(report["items"][0]["status"], "pass");
    assert_eq!(report["items"][1]["status"], "fail");
    assert_eq!(report["items"][2]["package"]["status"], "unavailable");
    for (index, source) in [(0, first), (1, second)] {
        let item = &report["items"][index];
        assert_eq!(item["package"]["status"], "packaged");
        let master = destination.join(item["package"]["masterPath"].as_str().unwrap());
        assert_eq!(
            media::hash_file(&master).unwrap(),
            media::hash_file(&source).unwrap()
        );
        assert_eq!(
            item["report"]["artifact"]["sha256"],
            media::hash_file(&master).unwrap()
        );
        assert!(item["report"]["artifact"].get("path").is_none());
        assert!(item["report"]["artifact"].get("manifest").is_none());
        assert!(
            destination
                .join(item["package"]["reviewPath"].as_str().unwrap())
                .is_file()
        );
    }
    assert_eq!(
        report["items"][0]["metadata"]["title"],
        "<script>no markup</script>"
    );
    assert_eq!(report["items"][0]["metadataSource"], "caller-supplied");
    let page = fs::read_to_string(destination.join("index.html")).unwrap();
    assert!(page.contains("&lt;script&gt;no markup&lt;/script&gt;"));
    assert!(!page.contains("<script>no markup</script>"));
    assert!(!destination.join("delivery.incomplete").exists());
}

#[test]
fn shared_package_omits_private_project_data_and_keeps_feedback_importable() {
    let f = fixture();
    let master = mark_succeeded(&f, 0);
    mark_succeeded(&f, 1);
    mark_failed(&f, 2);
    let destination = f._temp.path().join("share");
    let report = delivery_batch::package(&f.root, &f.batch, &destination, &request(), &backend())
        .unwrap()["manifest"]
        .clone();
    let secrets = [
        "SECRET_PROJECT_NAME_8b1d",
        "SECRET_SELECTED_TITLE_287a",
        "SECRET_OTHER_SEQUENCE_7cab",
        "SECRET_EXTENSION_6da2",
        "SECRET_TRANSCRIPT_97e1",
        "SECRET_REVIEW_391d",
        "SECRET_EXTRA_FILE_d3e6",
        "SECRET_EDIT_OBJECTIVE_0d2f",
        "SECRET_CUT_NOTE_81ef",
    ];
    assert_tree_excludes(&destination, &secrets);
    let item = &report["items"][0];
    let nested = destination
        .join(item["package"]["reviewPath"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let html = fs::read_to_string(nested.join("index.html")).unwrap();
    assert!(html.contains("Public title"));
    assert!(!html.contains("data-edit-objective") && !html.contains("data-cut-reason"));
    assert!(html.contains(&format!("data-review-project='{}'", f.snapshot.project_id)));
    let sha = media::hash_file(&master).unwrap();
    assert!(html.contains(&format!("data-review-sha256='{sha}'")));
    let packaged = destination.join(item["package"]["masterPath"].as_str().unwrap());
    assert_eq!(media::hash_file(&packaged).unwrap(), sha);
    let shared: serde_json::Value = serde_json::from_slice(
        &fs::read(nested.join(&f.children[0]).join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(shared["format"], "avw-share-render");
    assert_eq!(shared["projectId"], f.snapshot.project_id.to_string());
    assert_eq!(shared["verification"]["sha256"], sha);
    assert!(shared.get("snapshot").is_none() && shared.get("editPreflight").is_none());
    let draft: agent_video_workbench::review_import::ReviewDraft = serde_json::from_value(json!({
        "schemaVersion":1,"draftId":uuid::Uuid::new_v4().to_string(),"projectId":f.snapshot.project_id,
        "jobId":f.children[0],"artifactRevision":1,"artifactSha256":sha,
        "items":[{"id":"note-1","atMs":100,"endMs":200,"actor":"Creator","text":"Public feedback"}]
    })).unwrap();
    let imported = agent_video_workbench::review_import::import(
        &f.root,
        &draft,
        1,
        "share-review-import",
        false,
    )
    .unwrap();
    assert_eq!(imported["outcome"]["revision"], 2);
    let project = f.store.project().unwrap();
    assert!(
        project.extensions["avw.reviews"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["text"] == "Public feedback")
    );
}

#[test]
fn tampered_sibling_is_omitted_and_existing_destination_is_never_replaced() {
    let f = fixture();
    mark_succeeded(&f, 0);
    let changed = mark_succeeded(&f, 1);
    mark_succeeded(&f, 2);
    fs::write(changed, b"tampered").unwrap();
    let destination = f._temp.path().join("delivery");
    let report = delivery_batch::package(&f.root, &f.batch, &destination, &request(), &backend())
        .unwrap()["manifest"]
        .clone();
    assert_eq!(report["status"], "unknown");
    assert_eq!(report["package"]["state"], "partial");
    assert_eq!(report["package"]["packagedOutputs"], 2);
    assert_eq!(report["items"][0]["package"]["status"], "packaged");
    assert_eq!(report["items"][1]["package"]["status"], "unavailable");
    assert_eq!(report["items"][2]["package"]["status"], "packaged");
    assert!(!destination.join("outputs").join(&f.children[1]).exists());
    let before = fs::read(destination.join("manifest.json")).unwrap();
    assert!(
        delivery_batch::package(&f.root, &f.batch, &destination, &request(), &backend()).is_err()
    );
    assert_eq!(fs::read(destination.join("manifest.json")).unwrap(), before);
    assert!(!fs::read_dir(f._temp.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".avw-batch-")
    }));
}

#[test]
fn complete_package_includes_every_checked_output() {
    let f = fixture();
    for index in 0..3 {
        mark_succeeded(&f, index);
    }
    let brief: BatchBrief = serde_json::from_value(json!({"schemaVersion":1,"outputs":[
      {"sequenceId":"seq_main","brief":{"schemaVersion":1,"maxBytes":100}},
      {"sequenceId":"square","brief":{"schemaVersion":1,"maxBytes":100}},
      {"sequenceId":"landscape","brief":{"schemaVersion":1,"maxBytes":100}}
    ]}))
    .unwrap();
    let destination = f._temp.path().join("complete");
    let response =
        delivery_batch::package(&f.root, &f.batch, &destination, &brief, &backend()).unwrap();
    let report = &response["manifest"];
    assert_eq!(report["status"], "pass");
    assert_eq!(report["package"]["state"], "complete");
    assert_eq!(report["package"]["packagedOutputs"], 3);
    for item in report["items"].as_array().unwrap() {
        assert_eq!(item["package"]["status"], "packaged");
        assert!(
            destination
                .join(item["package"]["masterPath"].as_str().unwrap())
                .is_file()
        );
    }
}
