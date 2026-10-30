use agent_video_workbench::delivery;
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
