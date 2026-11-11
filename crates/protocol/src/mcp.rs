//! MCP server status.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct McpServer {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: McpStatus,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "integrationID", default)]
    pub integration_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpStatus {
    Connected,
    Pending,
    Disabled,
    NeedsAuth,
    Failed { error: String },
    Unknown { raw: String },
}

impl Default for McpStatus {
    fn default() -> Self {
        Self::Unknown { raw: String::new() }
    }
}

impl McpStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Connected => "connected",
            Self::Pending => "pending",
            Self::Disabled => "disabled",
            Self::NeedsAuth => "needs auth",
            Self::Failed { .. } => "failed",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error } => Some(error),
            _ => None,
        }
    }

    /// A status object from a newer server must not fail the whole list, so
    /// anything unrecognized keeps its raw tag instead of returning an error.
    fn from_value(value: &serde_json::Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::Unknown {
                raw: value.to_string(),
            };
        };
        let raw = object
            .get("status")
            .and_then(|tag| tag.as_str())
            .unwrap_or("");
        match raw {
            "connected" => Self::Connected,
            "pending" => Self::Pending,
            "disabled" => Self::Disabled,
            "needs_auth" => Self::NeedsAuth,
            "failed" => Self::Failed {
                error: object
                    .get("error")
                    .and_then(|error| error.as_str())
                    .unwrap_or_default()
                    .to_string(),
            },
            _ => Self::Unknown {
                raw: raw.to_string(),
            },
        }
    }
}

impl<'de> Deserialize<'de> for McpStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value =
            serde_json::Value::deserialize(deserializer).map_err(serde::de::Error::custom)?;
        Ok(Self::from_value(&value))
    }
}

impl McpServer {
    pub fn status_label(&self) -> &str {
        self.status.label()
    }

    pub fn error(&self) -> Option<&str> {
        self.status.error()
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
            (r#"{"status":"needs_auth","error":"sign in"}"#, "needs auth"),
            (r#"{"status":"failed","error":"boom"}"#, "failed"),
        ] {
            let server: McpServer =
                serde_json::from_str(&format!(r#"{{"name":"s","status":{json}}}"#))
                    .expect("server");
            assert_eq!(server.status_label(), label);
        }
    }

    #[test]
    fn mcp_failed_status_keeps_the_error() {
        let server: McpServer =
            serde_json::from_str(r#"{"name":"s","status":{"status":"failed","error":"boom"}}"#)
                .expect("server");
        assert_eq!(
            server.status,
            McpStatus::Failed {
                error: "boom".into()
            }
        );
        assert_eq!(server.error(), Some("boom"));
    }

    #[test]
    fn mcp_unknown_status_is_kept() {
        let server: McpServer =
            serde_json::from_str(r#"{"name":"s","status":{"status":"warming"}}"#).expect("server");
        assert_eq!(
            server.status,
            McpStatus::Unknown {
                raw: "warming".into()
            }
        );
        assert_eq!(server.status_label(), "unknown");
        assert_eq!(server.name, "s");
    }

    #[test]
    fn mcp_missing_status_and_fields_are_tolerated() {
        let server: McpServer = serde_json::from_str(r#"{"name":"s"}"#).expect("server");
        assert_eq!(server.status, McpStatus::Unknown { raw: String::new() });
        let bare: McpServer = serde_json::from_str(r#"{}"#).expect("bare server");
        assert_eq!(bare.name, "");
        assert_eq!(bare.title, None);
        assert_eq!(bare.integration_id, None);
    }
}
