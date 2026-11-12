//! PTY sessions.
//!
//! The sidecar owns the pseudo-terminal. These types describe the management
//! endpoints only; the byte stream itself is a WebSocket, see
//! `docs/spikes/pty-transport.md` and ADR 0003.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A PTY process as returned by `GET /api/pty`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PtyProcess {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub status: PtyStatus,
    #[serde(default)]
    pub pid: u32,
    #[serde(rename = "exitCode", default)]
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyStatus {
    Running,
    Exited,
    /// A status this client does not know yet.
    Unknown(String),
}

impl Default for PtyStatus {
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl PtyStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Running => "Running",
            Self::Exited => "Exited",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for PtyStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PtyStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "running" => Self::Running,
            "exited" => Self::Exited,
            _ => Self::Unknown(value),
        })
    }
}

/// Body of `POST /api/pty`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatePtyRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub title: String,
    pub env: BTreeMap<String, String>,
}

/// Answer of `POST /api/pty/{id}/connect-token`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PtyTicket {
    pub ticket: String,
    #[serde(default)]
    pub expires_in: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_parses_exited_without_code() {
        let pty: PtyProcess = serde_json::from_str(
            r#"{"id":"pty_1","title":"t","command":"/bin/sh","args":[],
                "cwd":"/tmp","status":"exited","pid":42}"#,
        )
        .expect("pty");
        assert_eq!(pty.status, PtyStatus::Exited);
        assert_eq!(pty.exit_code, None);
        assert_eq!(pty.pid, 42);
    }

    #[test]
    fn pty_status_unknown_is_kept() {
        let pty: PtyProcess =
            serde_json::from_str(r#"{"id":"pty_1","status":"warming"}"#).expect("pty");
        assert_eq!(pty.status, PtyStatus::Unknown("warming".into()));
        assert_eq!(pty.status.label(), "warming");
    }

    #[test]
    fn create_body_carries_the_contract_fields() {
        let body = serde_json::to_value(CreatePtyRequest {
            command: "/bin/sh".into(),
            args: Vec::new(),
            cwd: "/tmp".into(),
            title: String::new(),
            env: BTreeMap::new(),
        })
        .expect("body");
        assert_eq!(
            body,
            serde_json::json!({
                "command": "/bin/sh",
                "args": [],
                "cwd": "/tmp",
                "title": "",
                "env": {}
            })
        );
    }
}
