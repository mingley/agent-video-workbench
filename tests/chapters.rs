use agent_video_workbench::{
    chapters::{self, ChapterSelection},
    jobs::RenderInput,
    media,
    store::Store,
};
use agentcut_core::OperationBatch;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Store,
    job: String,
    manifest: PathBuf,
    video: PathBuf,
}

fn selection(id: &str, title: &str) -> ChapterSelection {
    ChapterSelection {
        marker_id: id.to_owned(),
        title: title.to_owned(),
    }
}

fn chosen(ids: &[&str]) -> Vec<ChapterSelection> {
    ids.iter()
        .map(|id| selection(id, &format!("Public {id}")))
        .collect()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut store = Store::create(&root, "Chapter test").unwrap();
    let project = store.project().unwrap();
    // Deliberately give the sequence 20 seconds of content and an explicit
    // 35-second duration. Bounds must follow the verified render, not content.
    let rate = json!({"numerator":30,"denominator":1});
    let at = |millis| json!({"value":millis,"rate":{"numerator":1000,"denominator":1}});
    let mut operations = vec![
        json!({"id":"asset","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video"}}),
        json!({"id":"clip","op":"clip.add","params":{"id":"clip","asset":"source","track":"track_v1","at":at(0),"sourceIn":at(0),"duration":at(20000),"fit":"contain"}}),
        json!({"id":"length","op":"sequence.set","target":"seq_main","params":{"duration":at(35000)}}),
    ];
    for (id, ms) in [
        ("intro", 0),
        ("second", 10500),
        ("collision", 10900),
        ("third", 20000),
        ("last_ok", 25500),
        ("last_short", 26000),
        ("end", 35000),
    ] {
        operations.push(json!({"id":format!("add_{id}"),"op":"marker.add","params":{
            "id":id,"sequence":"seq_main","at":at(ms),"name":format!("private name {id}"),"comment":"private comment should never leak"}}));
    }
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":project.project_id,
        "baseRevision":project.revision,"idempotencyKey":"chapter-fixture","operations":operations})).unwrap();
    store.apply(&batch, false).unwrap();
    let snapshot = store.project().unwrap();
    assert_eq!(
        snapshot
            .require_sequence("seq_main")
            .unwrap()
            .content_duration()
            .unwrap()
            .value,
        600
    );
    assert_eq!(
        snapshot.require_sequence("seq_main").unwrap().frame_rate,
        serde_json::from_value(rate).unwrap()
    );
    let input = RenderInput {
        sequence: "seq_main".into(),
        priority: 0,
        expected_revision: snapshot.revision,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        asr: None,
        analysis: None,
    };
    let job = store.enqueue("chapter-render", &input).unwrap().id;
    let artifact = format!("{job}-1");
    let folder = root.join("renders").join(&artifact);
    fs::create_dir(&folder).unwrap();
    let video = folder.join("video.mp4");
    fs::write(&video, b"frozen test video").unwrap();
    let manifest = folder.join("manifest.json");
    let verification = json!({"decoded":true,"expectedFrames":1050,"sha256":media::hash_file(&video).unwrap(),
        "probe":{"streams":[{"codec_type":"video","avg_frame_rate":"30/1","nb_read_frames":"1050",
            "start_pts":0,"duration_ts":537600,"time_base":"1/15360"}]}});
    fs::write(&manifest, serde_json::to_vec(&json!({"artifactId":artifact,"projectId":snapshot.project_id,
        "revision":snapshot.revision,"sequenceId":"seq_main","output":"video.mp4","snapshot":snapshot,
        "verification":verification,"delivery":{"reviewVideo":"video.mp4","files":[]}})).unwrap()).unwrap();
    let result = json!({"artifactId":artifact,"revision":snapshot.revision,"path":video,"manifest":manifest});
    let db = rusqlite::Connection::open(root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE jobs SET state='succeeded',attempt=1,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), job],
    )
    .unwrap();
    Fixture {
        _dir: dir,
        root,
        store,
        job,
        manifest,
        video,
    }
}

fn modify_manifest(path: &Path, edit: impl FnOnce(&mut Value)) {
    let mut m: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    edit(&mut m);
    fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
}

fn issue(result: &Value, code: &str) -> bool {
    result["youtube"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["code"] == code)
}

#[test]
fn authored_chapters_are_frozen_ordered_and_omit_private_marker_notes() {
    let mut f = fixture();
    let before = f.store.project().unwrap();
    let history = f.store.history().unwrap();
    let chapters = vec![
        selection("intro", "Introduction"),
        selection("second", "Café and ☕"),
        selection("last_ok", "Closing"),
    ];
    let report = chapters::draft(&f.root, &f.job, &chapters).unwrap();
    assert_eq!(
        report["text"],
        "00:00 Introduction\n00:10 Café and ☕\n00:25 Closing"
    );
    assert_eq!(report["youtube"]["documentedTimestampChecks"], "pass");
    assert_eq!(report["youtube"]["accountAndVideoEligibility"], "unknown");
    assert_eq!(report["chapters"][1]["exactTime"]["value"], 10500);
    assert_eq!(report["chapters"][1]["roundedDown"], true);
    assert_eq!(report["duration"]["value"], 537600);
    assert!(
        report["timingEvidence"]
            .as_str()
            .unwrap()
            .contains("not independently authenticated")
    );
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains("private name"));
    assert!(!encoded.contains("private comment"));
    assert_eq!(f.store.project().unwrap(), before);
    assert_eq!(f.store.history().unwrap(), history);
    let batch: OperationBatch = serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":before.project_id,
        "baseRevision":before.revision,"idempotencyKey":"move-marker-after-render","operations":[
        {"id":"move","op":"marker.set","target":"intro","params":{"at":{"value":4,"rate":{"numerator":1,"denominator":1}},"comment":"new private comment"}}]})).unwrap();
    f.store.apply(&batch, false).unwrap();
    assert_eq!(chapters::draft(&f.root, &f.job, &chapters).unwrap(), report);
}

#[test]
fn incomplete_out_of_order_colliding_and_short_drafts_are_diagnostic() {
    let f = fixture();
    let empty = chapters::draft(&f.root, &f.job, &[]).unwrap();
    assert!(issue(&empty, "at-least-three") && issue(&empty, "first-at-zero"));
    let bad = chapters::draft(
        &f.root,
        &f.job,
        &chosen(&["third", "second", "collision", "last_short"]),
    )
    .unwrap();
    assert_eq!(bad["youtube"]["documentedTimestampChecks"], "fail");
    assert_eq!(bad["chapters"][0]["markerId"], "third");
    assert_eq!(bad["chapters"][1]["timestampSeconds"], 10);
    assert_eq!(bad["chapters"][2]["timestampSeconds"], 10);
    for code in [
        "first-at-zero",
        "not-strictly-increasing",
        "chapter-under-ten-seconds",
        "last-chapter-under-ten-seconds",
    ] {
        assert!(issue(&bad, code), "missing {code}");
    }
    let duplicate =
        chapters::draft(&f.root, &f.job, &chosen(&["intro", "intro", "third"])).unwrap();
    assert!(issue(&duplicate, "duplicate-marker"));
}

#[test]
fn refuses_unknown_or_out_of_video_markers_and_unsafe_titles() {
    let f = fixture();
    for id in ["unknown", "end"] {
        assert!(chapters::draft(&f.root, &f.job, &chosen(&[id])).is_err());
    }
    for title in [
        "",
        "  ",
        "New\n00:25 Injected",
        "tab\there",
        "line\u{2028}break",
        "rtl\u{202e}override",
    ] {
        assert!(
            chapters::draft(&f.root, &f.job, &[selection("intro", title)]).is_err(),
            "accepted {title:?}"
        );
    }
    assert!(chapters::draft(&f.root, &f.job, &[selection("intro", &"x".repeat(513))]).is_err());
}

#[test]
fn refuses_modified_video_and_uncertain_recorded_stream_duration() {
    let f = fixture();
    let original = fs::read(&f.manifest).unwrap();
    for (field, value) in [
        ("nb_read_frames", json!("1049")),
        ("avg_frame_rate", json!("25/1")),
        ("start_pts", json!(1)),
        ("duration_ts", json!(537599)),
        ("time_base", json!("broken")),
    ] {
        fs::write(&f.manifest, &original).unwrap();
        modify_manifest(&f.manifest, |m| {
            m["verification"]["probe"]["streams"][0][field] = value
        });
        assert!(
            chapters::draft(&f.root, &f.job, &chosen(&["intro"])).is_err(),
            "accepted {field}"
        );
    }
    fs::write(&f.manifest, &original).unwrap();
    modify_manifest(&f.manifest, |m| m["verification"]["decoded"] = json!(false));
    assert!(chapters::draft(&f.root, &f.job, &chosen(&["intro"])).is_err());
    fs::write(&f.manifest, &original).unwrap();
    fs::write(&f.video, b"modified").unwrap();
    assert!(chapters::draft(&f.root, &f.job, &chosen(&["intro"])).is_err());
}

#[test]
fn refuses_sync_jobs_and_old_or_ambiguous_stream_evidence() {
    let f = fixture();
    let original = fs::read(&f.manifest).unwrap();
    for change in 0..4 {
        fs::write(&f.manifest, &original).unwrap();
        modify_manifest(&f.manifest, |m| match change {
            0 => {
                m["verification"]
                    .as_object_mut()
                    .unwrap()
                    .remove("expectedFrames");
            }
            1 => {
                m["verification"]["probe"]["streams"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("duration_ts");
            }
            2 => m["verification"]["probe"]["streams"][0]["duration_ts"] = json!(0),
            3 => {
                let another = m["verification"]["probe"]["streams"][0].clone();
                m["verification"]["probe"]["streams"]
                    .as_array_mut()
                    .unwrap()
                    .push(another);
            }
            _ => unreachable!(),
        });
        assert!(
            chapters::draft(&f.root, &f.job, &chosen(&["intro"])).is_err(),
            "accepted change {change}"
        );
    }
    fs::write(&f.manifest, &original).unwrap();
    modify_manifest(&f.manifest, |m| m["artifactId"] = json!(f.job));
    let db = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    let old: String = db
        .query_row("SELECT result FROM jobs WHERE id=?1", [&f.job], |row| {
            row.get(0)
        })
        .unwrap();
    let mut result: Value = serde_json::from_str(&old).unwrap();
    result["artifactId"] = json!(f.job);
    db.execute(
        "UPDATE jobs SET input=NULL,result=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&result).unwrap(), f.job],
    )
    .unwrap();
    let message = chapters::draft(&f.root, &f.job, &chosen(&["intro"]))
        .unwrap_err()
        .to_string();
    assert!(message.contains("does not bind the sequence"));
}
