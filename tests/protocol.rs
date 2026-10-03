use agent_video_workbench::{
    json, media,
    service::{Request, Service, envelope},
};
use serde_json::{Value, json};
#[test]
fn strict_parser_rejects_duplicate_keys_unknown_fields_and_wide_integers() {
    assert!(json::parse::<Value>(br#"{"a":{"x":1,"x":2}}"#).is_err());
    assert!(json::parse::<Value>(br#"{"revision":9007199254740992}"#).is_err());
    assert!(json::parse::<Request>(br#"{"command":"doctor","ignoreWarnings":true}"#).is_err());
    assert!(
        json::parse::<Request>(br#"{"command":"status","project":"p","project":"q"}"#).is_err()
    );
}
#[test]
fn scoped_service_rejects_escape_and_produces_equivalent_mcp_and_cli_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let service = Service {
        root: Some(root.clone()),
        backend: media::backend("ffmpeg".into(), "ffprobe".into()),
        executable: "avw".into(),
        asr: None,
    };
    assert!(
        !envelope(service.execute(Request::Create {
            project: "../escape".into(),
            name: "escape".into()
        }))["ok"]
            .as_bool()
            .unwrap()
    );
    let create = json!({"command":"create","project":"p","name":"Shared"});
    let result = envelope(service.execute(serde_json::from_value(create).unwrap()));
    assert_eq!(result["ok"], true);
    let invalid = json!({"command":"apply","project":"p","batch":{"schemaVersion":"1.0.0","projectId":result["result"]["projectId"],"baseRevision":10,"idempotencyKey":"stale","operations":[{"id":"rename","op":"project.rename","params":{"name":"Changed"}}]}});
    let expected = envelope(service.execute(serde_json::from_value(invalid.clone()).unwrap()));
    let messages = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"avw","arguments":invalid}})
    );
    let mut output = Vec::new();
    agent_video_workbench::mcp::serve(&service, &mut std::io::Cursor::new(messages), &mut output)
        .unwrap();
    let reply: Value =
        serde_json::from_str(String::from_utf8(output).unwrap().lines().nth(1).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"], expected);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(expected["error"]["code"], "E_REVISION_CONFLICT");
}
