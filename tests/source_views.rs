use agent_video_workbench::{
    analysis::{self, Input, Task},
    media,
    service::{Request, Service, envelope},
    source_views::{BudgetPreview, Storyboard},
    store::Store,
};
use agentcut_core::{Asset, Project, RationalRate};
use serde_json::{Value, json};
fn fixture() -> Project {
    let mut project = Project::new("views", 360, 640, RationalRate::frames(30).unwrap());
    let asset:Asset=serde_json::from_value(json!({"id":"source","kind":"video","uri":"originals/fake","label":"source","fingerprint":{"strategy":"sha256","sizeBytes":1,"sha256":"fake"},
    "metadata":{"status":"available","duration":{"value":120000,"rate":{"numerator":1000,"denominator":1}},"video":{"size":{"width":640,"height":360},"frameRate":{"numerator":30,"denominator":1},"pixelFormat":"yuv420p","sampleAspectRatio":{"numerator":1,"denominator":1},"rotationDegrees":0}},
    "extensions":{"avw.ingest.v1":{"format":{"start_time":"5.000000"},"streams":[{"index":0,"codec_type":"video","time_base":"1/15360","start_time":"5.000000"}]}}})).unwrap();
    project.assets.push(asset);
    project
}
fn storyboard() -> Value {
    json!({"kind":"storyboard","assetId":"source"})
}
fn preview() -> Value {
    json!({"kind":"budget-preview","assetId":"source","startMs":100,"endMs":10100,"maxDurationMs":10000,"maxBytes":1000000,"width":640,"height":360})
}
#[test]
fn view_requests_are_strict_and_validate_all_budgets_before_work() {
    let project = fixture();
    for value in [storyboard(), preview()] {
        let task: Task = serde_json::from_value(value).unwrap();
        analysis::validate(
            &Input {
                task,
                provider: None,
            },
            &project,
        )
        .unwrap();
    }
    for (field, bad) in [
        ("limit", json!(0)),
        ("limit", json!(25)),
        ("sampleEveryMs", json!(999)),
        ("tileWidth", json!(130)),
        ("maxBytes", json!(2097153)),
        ("offset", json!(12)),
        ("endMs", json!(120001)),
    ] {
        let mut value = storyboard();
        value[field] = bad;
        let task = serde_json::from_value(value).unwrap();
        assert!(
            analysis::validate(
                &Input {
                    task,
                    provider: None
                },
                &project
            )
            .is_err(),
            "{field}"
        );
    }
    for (field, bad) in [
        ("maxDurationMs", json!(9999)),
        ("maxDurationMs", json!(60001)),
        ("maxBytes", json!(16383)),
        ("width", json!(641)),
        ("height", json!(1282)),
        ("startMs", json!(-1)),
        ("endMs", json!(120001)),
    ] {
        let mut value = preview();
        value[field] = bad;
        let task = serde_json::from_value(value).unwrap();
        assert!(
            analysis::validate(
                &Input {
                    task,
                    provider: None
                },
                &project
            )
            .is_err(),
            "{field}"
        );
    }
    let mut unknown = storyboard();
    unknown["semanticUnderstanding"] = json!(true);
    assert!(serde_json::from_value::<Task>(unknown).is_err());
}
#[test]
fn absent_source_clock_is_refused_and_request_aliases_parse() {
    let mut project = fixture();
    project.assets[0].extensions.clear();
    let q: Storyboard = serde_json::from_value(json!({"assetId":"source"})).unwrap();
    assert!(agent_video_workbench::source_views::validate_storyboard(&q, &project).is_err());
    let q: BudgetPreview = serde_json::from_value({
        let mut v = preview();
        v.as_object_mut().unwrap().remove("kind");
        v
    })
    .unwrap();
    assert!(agent_video_workbench::source_views::validate_preview(&q, &project).is_err());
    let req = json!({"command":"source-storyboard","project":"p","request":{"assetId":"source"},"expectedRevision":1,"key":"story-01","noLaunch":true});
    assert!(serde_json::from_value::<Request>(req).is_ok());
}
#[test]
fn invalid_view_enqueue_does_not_leave_a_job_or_edit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    Store::create(&root, "empty").unwrap();
    let service = Service {
        root: None,
        backend: media::backend("unused".into(), "unused".into()),
        executable: "unused".into(),
        asr: None,
        provider: None,
        downloads: Default::default(),
    };
    let response=envelope(service.execute(serde_json::from_value(json!({"command":"source-storyboard","project":root,"request":{"assetId":"missing"},"expectedRevision":0,"key":"bad-view-01","noLaunch":true})).unwrap()));
    assert_eq!(response["ok"], false);
    let store = Store::open(&root).unwrap();
    assert!(store.jobs().unwrap().is_empty());
    assert_eq!(store.project().unwrap().revision, 0);
}
