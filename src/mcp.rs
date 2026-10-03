//! MCP stdio adapter. Tool execution delegates to Service; no separate mutation path.
use crate::{
    Error, Result,
    service::{Request, Service, envelope},
};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
const PROTOCOL: &str = "2025-11-25";
fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub fn serve(service: &Service, reader: &mut impl BufRead, writer: &mut impl Write) -> Result<()> {
    let mut initialized = false;
    loop {
        let mut line = Vec::new();
        let count = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count > 8 * 1024 * 1024 {
            return Err(Error::Invalid("MCP message exceeds 8 MiB".into()));
        }
        let message: Value = match crate::json::parse(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(writer, "{}", error(&Value::Null, -32700, &e.to_string()))?;
                writer.flush()?;
                continue;
            }
        };
        let id = message.get("id").cloned();
        let null = Value::Null;
        let response = if message["jsonrpc"] != "2.0" || !message["method"].is_string() {
            Some(error(
                id.as_ref().unwrap_or(&null),
                -32600,
                "Invalid JSON-RPC request",
            ))
        } else {
            match message["method"].as_str().unwrap_or("") {
                "initialize" if id.is_some() => {
                    initialized = true;
                    let requested = message["params"]["protocolVersion"]
                        .as_str()
                        .unwrap_or(PROTOCOL);
                    let negotiated = if matches!(
                        requested,
                        "2025-11-25" | "2025-06-18" | "2025-03-26" | "2024-11-05"
                    ) {
                        requested
                    } else {
                        PROTOCOL
                    };
                    Some(reply(
                        id.as_ref().unwrap_or(&null),
                        json!({"protocolVersion":negotiated,"capabilities":{"tools":{"listChanged":false},"resources":{"listChanged":false}},"serverInfo":{"name":"agent-video-workbench","version":env!("CARGO_PKG_VERSION")},"instructions":"Use avw command=agent-guide first. Long renders return durable job handles; use render-start and job-status. All paths are restricted to the configured workspace."}),
                    ))
                }
                "notifications/initialized" | "notifications/cancelled" => None,
                _ if id.is_none() => None,
                _ if !initialized => Some(error(
                    id.as_ref().unwrap_or(&null),
                    -32002,
                    "Initialize before requesting tools",
                )),
                "ping" => Some(reply(id.as_ref().unwrap_or(&null), json!({}))),
                "tools/list" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"tools":[{"name":"avw","description":"Persistent video editing, source inspection, transcripts, revision history, background render jobs and verified artifacts. Call agent-guide for the complete workflow.","inputSchema":tool_schema()}]}),
                )),
                "tools/call" => {
                    let result = if message["params"]["name"] != "avw" {
                        Err(Error::Invalid("unknown tool".into()))
                    } else {
                        crate::json::parse::<Request>(&serde_json::to_vec(
                            &message["params"]["arguments"],
                        )?)
                        .and_then(|r| service.execute(r))
                    };
                    let envelope = envelope(result);
                    let failed = envelope["ok"] == false;
                    Some(reply(
                        id.as_ref().unwrap_or(&null),
                        json!({"content":[{"type":"text","text":serde_json::to_string(&envelope)?}],"structuredContent":envelope,"isError":failed}),
                    ))
                }
                "resources/list" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"resources":[{"uri":"avw://guide","name":"Agent workflow","mimeType":"text/markdown"}]}),
                )),
                "resources/read" if message["params"]["uri"] == "avw://guide" => Some(reply(
                    id.as_ref().unwrap_or(&null),
                    json!({"contents":[{"uri":"avw://guide","mimeType":"text/markdown","text":include_str!("../AGENT_GUIDE.md")}]}),
                )),
                _ => Some(error(
                    id.as_ref().unwrap_or(&null),
                    -32601,
                    "Method not found",
                )),
            }
        };
        if let Some(response) = response {
            writeln!(writer, "{response}")?;
            writer.flush()?;
        }
    }
}

fn tool_schema() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_else(|_| json!({}));
    schema["type"] = json!("object");
    schema
}
