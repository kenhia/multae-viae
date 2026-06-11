//! Test fixture: a minimal stdio MCP server (sprint 008, T027).
//!
//! Hand-rolled JSON-RPC-over-stdio implementing the subset of MCP that
//! rmcp's client handshake requires: `initialize`, the `notifications/
//! initialized` notification (ignored), `tools/list`, `tools/call`, and
//! `ping`. Messages are newline-delimited JSON, matching rmcp's child-process
//! transport.
//!
//! It exposes four tools chosen to exercise mv-core's MCP merge rules
//! (`mv_core::mcp::registry`):
//! - `file_list`  — collides with a built-in name → must be skipped;
//! - `read_file`  — in `SEMANTIC_OVERLAPS` → must be skipped;
//! - `echo_tool`  — a normal tool, echoes its `text` argument;
//! - `big_tool`   — returns > 10k chars to exercise output truncation.
//!
//! This binary exists only for the integration tests in
//! `tests/cli_mcp_fake.rs` (spawned via `CARGO_BIN_EXE_fake_mcp_server`);
//! it is not part of the user-facing CLI.

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        // Notifications (no id) get no response.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let reply = match method {
            "initialize" => result_response(id, initialize_result(&msg)),
            "tools/list" => result_response(id, tools_list_result()),
            "tools/call" => result_response(id, tools_call_result(&msg)),
            "ping" => result_response(id, serde_json::json!({})),
            other => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method not found: {other}")},
            }),
        };

        let mut out = stdout.lock();
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

fn result_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn initialize_result(msg: &serde_json::Value) -> serde_json::Value {
    // Echo whatever protocol version the client asked for; rmcp's
    // ProtocolVersion deserializer accepts known versions and is lenient
    // about the rest.
    let requested = msg
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("2025-06-18");
    serde_json::json!({
        "protocolVersion": requested,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "fake-mcp-server", "version": "0.1.0"},
    })
}

fn path_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"path": {"type": "string", "description": "a path"}},
    })
}

fn tools_list_result() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {
                "name": "file_list",
                "description": "fake tool colliding with the built-in file_list",
                "inputSchema": path_schema(),
            },
            {
                "name": "read_file",
                "description": "fake tool semantically overlapping the built-in file_read",
                "inputSchema": path_schema(),
            },
            {
                "name": "echo_tool",
                "description": "echo the text argument back",
                "inputSchema": {
                    "type": "object",
                    "properties": {"text": {"type": "string", "description": "text to echo"}},
                    "required": ["text"],
                },
            },
            {
                "name": "big_tool",
                "description": "return more than 10k characters",
                "inputSchema": {"type": "object", "properties": {}},
            },
        ],
    })
}

fn tools_call_result(msg: &serde_json::Value) -> serde_json::Value {
    let params = msg.get("params");
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    let text = match name {
        "echo_tool" => {
            let input = params
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            format!("echo: {input}")
        }
        "big_tool" => "B".repeat(12_000),
        // The colliding/overlapping tools answer with sentinels so a test
        // can prove they were never routed to (the built-ins win).
        "file_list" => "FAKE-MCP-FILE-LIST".to_string(),
        "read_file" => "FAKE-MCP-READ-FILE".to_string(),
        other => format!("unknown tool: {other}"),
    };
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "isError": false,
    })
}
