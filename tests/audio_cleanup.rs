use agent_video_workbench::{
    audio_cleanup::{self, CleanupEdit, KEY, Settings},
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
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let mut store = Store::create(&root, "Audio cleanup").unwrap();
    fs::write(root.join("originals/audio.wav"), b"fixture").unwrap();
    let project = store.project().unwrap();
    let rate = json!({"numerator":30,"denominator":1});
    let t = |value| json!({"value":value,"rate":rate});
    let batch:OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0",
        "projectId":project.project_id,"baseRevision":0,"idempotencyKey":"audio-cleanup-fixture",
        "operations":[
            {"id":"asset","op":"asset.add","params":{"id":"audio","uri":"originals/audio.wav","kind":"audio",
                "metadata":{"status":"available","duration":t(120),"audio":{"sampleRate":48000,"channels":1,"channelLayout":"mono","sampleFormat":"s16"}}}},
            {"id":"one","op":"clip.add","params":{"id":"one","asset":"audio","track":"track_a1","at":t(0),"sourceIn":t(0),"duration":t(60)}},
            {"id":"two","op":"clip.add","params":{"id":"two","asset":"audio","track":"track_a1","at":t(60),"sourceIn":t(60),"duration":t(60)}}
        ]})).unwrap();
    store.apply(&batch, false).unwrap();
    Fixture { _dir: dir, root }
}

fn setting(item: &str, reduction: u8, floor: i8) -> CleanupEdit {
    CleanupEdit::Set {
        sequence: "seq_main".into(),
        item_id: item.into(),
        settings: Settings {
            noise_reduction_db: reduction,
            noise_floor_db: floor,
        },
    }
}

fn apply(root: &Path, key: &str, operations: Vec<Value>) {
    let mut store = Store::open(root).unwrap();
    let p = store.project().unwrap();
    let batch: OperationBatch =
        serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,
        "baseRevision":p.revision,"idempotencyKey":key,"operations":operations}))
        .unwrap();
    store.apply(&batch, false).unwrap();
}

#[test]
fn settings_are_atomic_reversible_and_idempotent() {
    let f = fixture();
    let before = Store::open(&f.root).unwrap().project().unwrap();
    let requested = setting("one", 12, -35);
    let preview = audio_cleanup::edit(&f.root, &requested, 1, "cleanup-one", true).unwrap();
    assert_eq!(preview.revision, 2);
    assert_eq!(Store::open(&f.root).unwrap().project().unwrap(), before);
    let applied = audio_cleanup::edit(&f.root, &requested, 1, "cleanup-one", false).unwrap();
    assert_eq!(preview, applied);
    assert_eq!(
        audio_cleanup::edit(&f.root, &requested, 1, "cleanup-one", false).unwrap(),
        applied
    );
    let project = Store::open(&f.root).unwrap().project().unwrap();
    assert_eq!(
        project.require_item("one").unwrap().extensions[KEY],
        json!({"noiseReductionDb":12,"noiseFloorDb":-35})
    );
    assert!(
        !project
            .require_item("two")
            .unwrap()
            .extensions
            .contains_key(KEY)
    );
    let remove = CleanupEdit::Remove {
        sequence: "seq_main".into(),
        item_id: "one".into(),
    };
    assert_eq!(
        audio_cleanup::edit(&f.root, &remove, 2, "cleanup-remove", false)
            .unwrap()
            .revision,
        3
    );
    assert!(
        !Store::open(&f.root)
            .unwrap()
            .project()
            .unwrap()
            .require_item("one")
            .unwrap()
            .extensions
            .contains_key(KEY)
    );
    let mut store = Store::open(&f.root).unwrap();
    store.restore(2, 3, "restore-cleanup").unwrap();
    assert!(
        store
            .project()
            .unwrap()
            .require_item("one")
            .unwrap()
            .extensions
            .contains_key(KEY)
    );
    assert_eq!(
        fs::read(f.root.join("originals/audio.wav")).unwrap(),
        b"fixture"
    );
}

#[test]
fn rejects_bad_settings_disabled_muted_solo_and_locked_targets_without_reserving_key() {
    let f = fixture();
    for (reduction, floor) in [(0, -35), (19, -35), (12, -81), (12, -24)] {
        assert!(
            audio_cleanup::edit(
                &f.root,
                &setting("one", reduction, floor),
                1,
                "same-valid-key",
                false
            )
            .is_err()
        );
    }
    assert!(
        audio_cleanup::edit(
            &f.root,
            &setting("missing", 12, -35),
            1,
            "same-valid-key",
            false
        )
        .is_err()
    );
    let before = Store::open(&f.root).unwrap().project().unwrap();
    assert_eq!(before.revision, 1);
    assert_eq!(
        audio_cleanup::edit(
            &f.root,
            &setting("one", 12, -35),
            1,
            "same-valid-key",
            false
        )
        .unwrap()
        .revision,
        2
    );

    for (field, value) in [("enabled", false), ("muted", true), ("locked", true)] {
        let g = fixture();
        apply(
            &g.root,
            "disable-target",
            vec![
                json!({"id":"change","op":"track.set","target":"track_a1","params":{field:value}}),
            ],
        );
        assert!(
            audio_cleanup::edit(
                &g.root,
                &setting("one", 12, -35),
                2,
                "reject-disabled",
                false
            )
            .is_err(),
            "{field}"
        );
        assert_eq!(Store::open(&g.root).unwrap().project().unwrap().revision, 2);
    }
    let g = fixture();
    apply(
        &g.root,
        "other-track-solo",
        vec![
            json!({"id":"add","op":"track.add","params":{"id":"other","sequence":"seq_main","type":"audio"}}),
            json!({"id":"solo","op":"track.set","target":"other","params":{"solo":true}}),
        ],
    );
    assert!(
        audio_cleanup::edit(&g.root, &setting("one", 12, -35), 2, "reject-solo", false).is_err()
    );
    let g = fixture();
    apply(
        &g.root,
        "disabled-solo",
        vec![
            json!({"id":"add","op":"track.add","params":{"id":"other","sequence":"seq_main","type":"audio"}}),
            json!({"id":"solo","op":"track.set","target":"other","params":{"solo":true,"enabled":false}}),
        ],
    );
    assert!(
        audio_cleanup::edit(
            &g.root,
            &setting("one", 12, -35),
            2,
            "ignore-disabled-solo",
            false
        )
        .is_ok()
    );
}

#[test]
fn refuses_unprobed_audio_and_source_preserving_output() {
    let f = fixture();
    let mut project = Store::open(&f.root).unwrap().project().unwrap();
    project
        .assets
        .iter_mut()
        .find(|asset| asset.id == "audio")
        .unwrap()
        .metadata
        .audio = None;
    let db = rusqlite::Connection::open(f.root.join("project.sqlite")).unwrap();
    db.execute(
        "UPDATE revisions SET snapshot=?1 WHERE id=1",
        [serde_json::to_string(&project).unwrap()],
    )
    .unwrap();
    assert!(
        audio_cleanup::edit(
            &f.root,
            &setting("one", 12, -35),
            1,
            "unprobed-audio",
            false
        )
        .unwrap_err()
        .to_string()
        .contains("probed source audio")
    );
    project
        .assets
        .iter_mut()
        .find(|asset| asset.id == "audio")
        .unwrap()
        .metadata
        .audio = Some(
        serde_json::from_value(
            json!({"sampleRate":48000,"channels":1,"channelLayout":"mono","sampleFormat":"s16"}),
        )
        .unwrap(),
    );
    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .extensions
        .insert(
            agent_video_workbench::assembly::POLICY.into(),
            json!({"color":"preserve","quality":"high","explicitFrameRate":false,
               "allowTightCuts":false,"allowReorder":false}),
        );
    db.execute(
        "UPDATE revisions SET snapshot=?1 WHERE id=1",
        [serde_json::to_string(&project).unwrap()],
    )
    .unwrap();
    assert!(
        audio_cleanup::edit(
            &f.root,
            &setting("one", 12, -35),
            1,
            "source-preserving",
            false
        )
        .unwrap_err()
        .to_string()
        .contains("source-preserving")
    );
    assert_eq!(Store::open(&f.root).unwrap().project().unwrap().revision, 1);
}

#[test]
fn inserts_only_the_selected_audio_branch_and_reports_a_later_disabled_item() {
    let f = fixture();
    audio_cleanup::edit(&f.root, &setting("one", 12, -35), 1, "cleanup-one", false).unwrap();
    let mut project = Store::open(&f.root).unwrap().project().unwrap();
    fn plan(
        project: &agentcut_core::Project,
        root: &Path,
    ) -> (
        agentcut_render::compile::RenderPlan,
        agentcut_render::ir::RenderIr,
    ) {
        let normalized = agentcut_core::normalize::normalize_sequence(project, "seq_main").unwrap();
        let preset = agentcut_render::preset::require("h264-mp4").unwrap();
        let ir = agentcut_render::ir::build(
            project,
            &normalized,
            root,
            preset,
            &root.join("unused.mp4"),
            None,
            false,
        )
        .unwrap();
        let plan = agentcut_render::compile::compile(&ir, Path::new("ffmpeg"), "test").unwrap();
        (plan, ir)
    }
    let (mut compiled, ir) = plan(&project, &f.root);
    assert_eq!(
        ir.audio_elements
            .iter()
            .map(|e| e.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    agent_video_workbench::audio::adapt_fades(&mut compiled, &ir).unwrap();
    let report = audio_cleanup::adapt(&mut compiled, &ir, &project, "seq_main").unwrap();
    assert_eq!(report[0]["itemId"], "one");
    assert_eq!(report[0]["status"], "applied");
    assert!(
        report[0]["routing"]
            .as_str()
            .unwrap()
            .contains("after clip/track gain")
    );
    let graph = &compiled.args[compiled
        .args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .unwrap()
        + 1];
    assert_eq!(graph.matches("afftdn=").count(), 1);
    assert!(graph.contains("[avw_cleanup_0]afftdn=nr=12:nf=-35:nt=w:tn=0[a0]"));
    assert!(!graph.contains("[avw_cleanup_1]"));

    // Cleanup stays on the selected producer when the output is routed into
    // a bus and then participates in a sidechain; the sibling remains intact.
    apply(
        &f.root,
        "route-through-bus",
        vec![
            json!({"id":"bus","op":"bus.add","params":{"id":"mix","sequence":"seq_main","gainDb":-3.0}}),
            json!({"id":"route","op":"track.set","target":"track_a1","params":{"output":"mix"}}),
        ],
    );
    project = Store::open(&f.root).unwrap().project().unwrap();
    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .extensions
        .insert(
            "avw.ducking".into(),
            json!({"dialogueItems":["one"],"musicItems":["two"],"threshold":0.01,
            "ratio":5.0,"attackMs":20.0,"releaseMs":250.0}),
        );
    let (mut compiled, ir) = plan(&project, &f.root);
    assert!(
        ir.audio_elements
            .iter()
            .all(|element| element.bus.as_deref() == Some("mix"))
    );
    agent_video_workbench::audio::adapt_fades(&mut compiled, &ir).unwrap();
    audio_cleanup::adapt(&mut compiled, &ir, &project, "seq_main").unwrap();
    agent_video_workbench::audio::adapt(
        &mut compiled,
        &ir,
        project.require_sequence("seq_main").unwrap(),
    )
    .unwrap();
    let graph = &compiled.args[compiled
        .args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .unwrap()
        + 1];
    assert!(graph.contains("[avw_cleanup_0]afftdn=nr=12:nf=-35:nt=w:tn=0[avw_d0]"));
    assert!(graph.contains("sidechaincompress="));
    assert!(graph.contains("[bus0out]"));
    assert_eq!(graph.matches("afftdn=").count(), 1);

    let mut nested_project = project.clone();
    let rate = nested_project
        .require_sequence("seq_main")
        .unwrap()
        .frame_rate;
    nested_project.sequences.push(agentcut_core::Sequence::new(
        "child", "Child", 360, 640, rate, 48000,
    ));
    let mut nested = nested_project.require_item("two").unwrap().clone();
    nested.id = "nested".into();
    nested.payload = agentcut_core::ItemPayload::Sequence(agentcut_core::SequenceRefData {
        sequence_id: "child".into(),
        source_range: nested.timeline_range().unwrap(),
        transform: agentcut_core::Transform2d::identity(),
        opacity: 1.0,
        blend_mode: agentcut_core::BlendMode::Normal,
    });
    nested_project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("track_v1")
        .unwrap()
        .items
        .push(nested);
    assert!(
        audio_cleanup::validate_scope(&nested_project, "seq_main", false)
            .unwrap_err()
            .to_string()
            .contains("nested")
    );

    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("track_a1")
        .unwrap()
        .enabled = false;
    let (mut compiled, ir) = plan(&project, &f.root);
    let report = audio_cleanup::adapt(&mut compiled, &ir, &project, "seq_main").unwrap();
    assert_eq!(report[0]["status"], "skipped");
    assert!(!compiled.args.join(" ").contains("afftdn="));
    assert!(
        audio_cleanup::validate_scope(&project, "seq_main", true)
            .unwrap_err()
            .to_string()
            .contains("source-preserving")
    );
}
