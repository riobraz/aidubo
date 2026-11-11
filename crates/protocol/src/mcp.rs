//! MCP server status.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct McpServer {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: serde_json::Value,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "integrationID", default)]
    pub integration_id: Option<String>,
}

impl McpServer {
    pub fn status_label(&self) -> &str {
        match self
            .status
            .get("status")
            .and_then(serde_json::Value::as_str)
        {
            Some("connected") => "connected",
            Some("pending") => "pending",
            Some("disabled") => "disabled",
            Some("needs-auth") => "needs auth",
            Some("failed") => "failed",
            _ => "unknown",
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.status.get("error").and_then(serde_json::Value::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_status_parses_every_variant() {
        for (json, label) in [
            (r#"{"status":"connected"}"#, "connected"),
            (r#"{"status":"pending"}"#, "pending"),
            (r#"{"status":"disabled"}"#, "disabled"),
            (r#"{"status":"needs-auth"}"#, "needs auth"),
            (r#"{"status":"failed","error":"boom"}"#, "failed"),
        ] {
            let server: McpServer =
                serde_json::from_str(&format!(r#"{{"name":"s","status":{json}}}"#))
                    .expect("server");
            assert_eq!(server.status_label(), label);
        }
    }

    #[test]
    fn mcp_unknown_status_is_kept() {
        let server: McpServer =
            serde_json::from_str(r#"{"name":"s","status":{"status":"warming"}}"#).expect("server");
        assert_eq!(server.status_label(), "unknown");
        assert_eq!(server.name, "s");
    }
}
