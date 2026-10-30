use agent_video_workbench::{delivery, media, store::Store};
use agentcut_core::{OperationBatch, Project, RationalRate};
use serde_json::json;

fn project_with_captions() -> Project {
    let project = Project::new("Review", 360, 640, RationalRate::frames(30).unwrap());
    let batch: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0", "projectId":project.project_id, "baseRevision":0,
        "idempotencyKey":"caption-sidecar-fixture", "operations":[
            {"id":"track-a", "op":"track.add", "params":{"id":"captions_a","sequence":"seq_main","type":"caption"}},
            {"id":"track-b", "op":"track.add", "params":{"id":"captions_b","sequence":"seq_main","type":"caption"}},
            {"id":"cue-a", "op":"caption.add", "params":{"id":"cue_a","track":"captions_a","at":{"value":0,"rate":{"numerator":30,"denominator":1}},"duration":{"value":30,"rate":{"numerator":30,"denominator":1}},"text":"First caption"}},
            {"id":"cue-b", "op":"caption.add", "params":{"id":"cue_b","track":"captions_b","at":{"value":30,"rate":{"numerator":30,"denominator":1}},"duration":{"value":30,"rate":{"numerator":30,"denominator":1}},"text":"Second caption"}}
        ]
    }))
    .unwrap();
    agentcut_core::apply_batch(&project, &batch)
        .unwrap()
        .project
}

fn assert_sidecars_match_rendered(project: &Project, expected: &[&str]) {
    let sequence = project.require_sequence("seq_main").unwrap();
    let (srt, vtt, mapping) = delivery::sidecars(sequence).unwrap();
    let actual: Vec<_> = ["First caption", "Second caption"]
        .into_iter()
        .filter(|text| {
            assert_eq!(srt.contains(text), vtt.contains(text));
            srt.contains(text)
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(mapping["cues"].as_array().unwrap().len(), expected.len());

    // Rendering clears disabled solos in its private copy before calling the
    // pinned normalizer. Compare the sidecars with those visible captions.
    let mut prepared = project.clone();
    for track in &mut prepared.require_sequence_mut("seq_main").unwrap().tracks {
        if !track.enabled {
            track.solo = false;
        }
    }
    let normalized = agentcut_core::normalize::normalize_sequence(&prepared, "seq_main").unwrap();
    let mut rendered: Vec<_> = normalized
        .video_layers
        .iter()
        .flat_map(|layer| &layer.items)
        .filter_map(|item| item.caption.as_ref().map(|caption| caption.text.as_str()))
        .collect();
    rendered.sort_unstable();
    let mut expected_sorted = expected.to_vec();
    expected_sorted.sort_unstable();
    assert_eq!(rendered, expected_sorted);
}

#[test]
fn caption_sidecars_follow_visual_solos_and_ignore_audio_or_disabled_solos() {
    let mut project = project_with_captions();
    let seq = project.require_sequence_mut("seq_main").unwrap();
    seq.track_mut("track_a1").unwrap().solo = true;
    assert_sidecars_match_rendered(&project, &["First caption", "Second caption"]);

    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("track_v1")
        .unwrap()
        .solo = true;
    assert_sidecars_match_rendered(&project, &[]);

    project
        .require_sequence_mut("seq_main")
        .unwrap()
        .track_mut("captions_a")
        .unwrap()
        .solo = true;
    assert_sidecars_match_rendered(&project, &["First caption"]);

    let seq = project.require_sequence_mut("seq_main").unwrap();
    seq.track_mut("captions_a").unwrap().solo = false;
    seq.track_mut("track_v1").unwrap().enabled = false;
    assert_sidecars_match_rendered(&project, &["First caption", "Second caption"]);
}

#[test]
fn range_reference_uses_frozen_sequence_duration_without_rounding_past_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let mut store = Store::create(&root, "Range delivery").unwrap();
    let project = store.project().unwrap();
    let rate = json!({"numerator":30000,"denominator":1001});
    let batch: OperationBatch = serde_json::from_value(json!({
        "schemaVersion":"1.0.0","projectId":project.project_id,"baseRevision":0,
        "idempotencyKey":"range-duration-fixture","operations":[
            {"id":"source","op":"asset.add","params":{"id":"source","uri":"originals/test","kind":"video"}},
            {"id":"seq","op":"sequence.add","params":{"id":"ntsc","name":"Review <take>","width":360,"height":640,"frameRate":rate}},
            {"id":"track","op":"track.add","params":{"id":"ntsc_video","sequence":"ntsc","type":"video"}},
            {"id":"clip","op":"clip.add","params":{"id":"take","asset":"source","track":"ntsc_video",
                "at":{"value":0,"rate":rate},"sourceIn":{"value":0,"rate":rate},
                "duration":{"value":45,"rate":rate},"fit":"contain"}}
        ]
    })).unwrap();
    store.apply(&batch, false).unwrap();
    let artifact_dir = root.join("renders/fixture-job");
    std::fs::create_dir(&artifact_dir).unwrap();
    let video = artifact_dir.join("video.mp4");
    let sheet = artifact_dir.join("sheet.png");
    std::fs::write(&video, b"video fixture").unwrap();
    std::fs::write(&sheet, b"sheet fixture").unwrap();
    let manifest = artifact_dir.join("manifest.json");
    std::fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "snapshot":store.project().unwrap(),"sequenceId":"ntsc",
            "verification":{"sha256":media::hash_file(&video).unwrap()},
            "contactSheet":{"sha256":media::hash_file(&sheet).unwrap()},
            "delivery":{"files":[]},"editPreflight":{}
        }))
        .unwrap(),
    )
    .unwrap();
    store.start_job("fixture-job", 1).unwrap();
    store
        .finish_job("fixture-job", &json!({"manifest":manifest,"path":video}))
        .unwrap();
    let output = temp.path().join("review");
    delivery::package(&root, "fixture-job", &output).unwrap();
    let page = std::fs::read_to_string(output.join("index.html")).unwrap();
    // 45 × 1001 / 30000 seconds = 1501.5ms; review-add accepts integral
    // milliseconds, so the upper bound must never be rounded to 1502ms.
    assert!(page.contains("data-review-limit-ms='1501'"));
    assert!(page.contains("Review &lt;take&gt;"));
    assert!(!page.contains("Review <take>"));
    assert!(page.contains("data-feedback-range hidden"));
    assert!(page.contains("id='range-fixture-job'"));
}
