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
