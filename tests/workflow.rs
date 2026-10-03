use agent_video_workbench::workflow::wrap_caption;

#[test]
fn captions_wrap_without_losing_words_and_refuse_overflow() {
    let measure = |s: &str| s.chars().count() as f64;
    let text = "Literal 'quote' \\ slash; 100%";
    let wrapped = wrap_caption(text, 16.0, measure).unwrap();
    assert_eq!(
        wrapped.split_whitespace().collect::<Vec<_>>(),
        text.split_whitespace().collect::<Vec<_>>()
    );
    assert!(wrapped.lines().all(|line| measure(line) <= 16.0));
    assert!(wrap_caption("oversizedword", 4.0, measure).is_err());
    assert!(wrap_caption("one two six ten end", 3.0, measure).is_err());
}

#[test]
fn multiline_caption_render_planes_preserve_timing_and_safe_positions() {
    use agentcut_core::{OperationBatch, Project, RationalRate};
    use serde_json::json;
    let p = Project::new("caption", 360, 640, RationalRate::frames(30).unwrap());
    let time = json!({"value":0,"rate":{"numerator":30,"denominator":1}});
    let batch: OperationBatch=serde_json::from_value(json!({"schemaVersion":"1.0.0","projectId":p.project_id,"baseRevision":0,"idempotencyKey":"caption-lines","operations":[
        {"id":"track","op":"track.add","params":{"id":"captions","sequence":"seq_main","type":"caption"}},
        {"id":"caption","op":"caption.add","params":{"id":"cue","track":"captions","at":time,"duration":{"value":30,"rate":{"numerator":30,"denominator":1}},"text":"First line\nSecond line"}}
    ]})).unwrap();
    let project = agentcut_core::apply_batch(&p, &batch).unwrap().project;
    let mut normalized =
        agentcut_core::normalize::normalize_sequence(&project, "seq_main").unwrap();
    agent_video_workbench::media::layout_caption_lines(&mut normalized).unwrap();
    let lines: Vec<_> = normalized
        .video_layers
        .iter()
        .flat_map(|l| &l.items)
        .filter(|i| i.caption.is_some())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].timeline_range, lines[1].timeline_range);
    assert_eq!(lines[0].text.as_ref().unwrap().text, "First line");
    assert_eq!(lines[1].text.as_ref().unwrap().text, "Second line");
    assert!(lines[0].placement.unwrap().y < lines[1].placement.unwrap().y);
    assert!(lines[1].placement.unwrap().y + lines[1].placement.unwrap().scaled_height <= 608.0);
    assert_eq!(project.require_item("cue").unwrap().kind(), "caption");
}
