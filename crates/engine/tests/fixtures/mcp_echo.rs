//! Minimal MCP stdio server for the M13-01 e2e test.
//!
//! It answers the handshake and the empty catalogs so the sidecar reports the
//! server as `connected`; it is not installed or shipped with the app.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Notifications have no id and need no answer.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let result = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                let version = message
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18");
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mcp-echo", "version": "0.1.0" }
                })
            }
            Some("tools/list") => json!({ "tools": [] }),
            Some("resources/list") => json!({ "resources": [], "templates": [] }),
            Some("prompts/list") => json!({ "prompts": [] }),
            _ => json!({}),
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }
    Ok(())
}
