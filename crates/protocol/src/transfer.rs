//! Session export, import, and usage statistics.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ModelRef, TokenUsage};

const INVALID_EXPORT: &str = "This file is not an OpenCode session export.";

/// One exported session. `info` and `messages` stay as raw JSON because an
/// import must send the server's payload back unchanged: the typed transcript
/// types drop fields they do not model, which would lose data on a round trip.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SessionTransfer {
    #[serde(default)]
    pub info: Value,
    #[serde(default)]
    pub messages: Vec<Value>,
}

impl SessionTransfer {
    /// Parse an export file. Both a bare `{info, messages}` document and the
    /// endpoint's `{data: {info, messages}}` envelope are accepted, so a file
    /// saved straight from the API can be imported too.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| INVALID_EXPORT.to_string())?;
        Self::from_value(value)
    }

    pub fn from_value(mut value: Value) -> Result<Self, String> {
        if value.get("info").is_none()
            && let Some(data) = value.get_mut("data")
        {
            value = data.take();
        }
        let transfer: Self =
            serde_json::from_value(value).map_err(|_| INVALID_EXPORT.to_string())?;
        if !transfer.info.is_object() {
            return Err(INVALID_EXPORT.to_string());
        }
        Ok(transfer)
    }

    pub fn session_id(&self) -> &str {
        self.info
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn title(&self) -> Option<&str> {
        self.info.get("title").and_then(Value::as_str)
    }

    pub fn project_directory(&self) -> Option<&str> {
        self.info
            .get("location")
            .and_then(|location| location.get("directory"))
            .and_then(Value::as_str)
    }

    pub fn project_id(&self) -> Option<&str> {
        self.info.get("projectID").and_then(Value::as_str)
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// The closed range the stats endpoint summed, in epoch milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub struct StatsRange {
    #[serde(default)]
    pub from: i64,
    #[serde(default)]
    pub to: i64,
}

// The stats endpoint requires epoch milliseconds. ISO date strings are
// rejected with a 500, so the untyped query is built from these integers.

/// Totals for one stats range.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SessionStats {
    #[serde(default)]
    pub range: StatsRange,
    #[serde(default)]
    pub sessions: u64,
    #[serde(default)]
    pub subagents: u64,
    #[serde(default)]
    pub prompts: u64,
    #[serde(default)]
    pub steps: u64,
    #[serde(default)]
    pub tokens: TokenUsage,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(default, rename = "activeDays")]
    pub active_days: u64,
    #[serde(default)]
    pub streak: u64,
    #[serde(default)]
    pub activity: Vec<StatsActivity>,
    #[serde(default)]
    pub models: Vec<StatsModel>,
}

impl SessionStats {
    pub fn is_empty(&self) -> bool {
        self.sessions == 0 && self.prompts == 0 && self.steps == 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct StatsActivity {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub steps: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct StatsModel {
    #[serde(default)]
    pub model: ModelRef,
    #[serde(default)]
    pub steps: u64,
    #[serde(default)]
    pub tokens: TokenUsage,
    #[serde(default)]
    pub cost: f64,
}

impl StatsModel {
    pub fn label(&self) -> String {
        let provider = self.model.provider_id.as_deref().unwrap_or_default();
        let name = if provider.is_empty() {
            self.model.id.clone()
        } else {
            format!("{provider}/{}", self.model.id)
        };
        match self.model.variant.as_deref() {
            Some(variant) if !variant.is_empty() && variant != "default" => {
                format!("{name} ({variant})")
            }
            _ => name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_parses_info_and_messages() {
        let raw = r#"{
            "info": {
                "id": "ses_1",
                "title": "Export test",
                "projectID": "prj_1",
                "location": { "directory": "/tmp/project" }
            },
            "messages": [
                { "type": "user", "id": "msg_1", "text": "hi" },
                { "type": "assistant", "id": "msg_2", "content": [] },
                { "type": "future-kind", "id": "msg_3", "extra": true }
            ]
        }"#;
        let transfer = SessionTransfer::parse(raw.as_bytes()).expect("transfer");
        assert_eq!(transfer.session_id(), "ses_1");
        assert_eq!(transfer.title(), Some("Export test"));
        assert_eq!(transfer.project_id(), Some("prj_1"));
        assert_eq!(transfer.project_directory(), Some("/tmp/project"));
        assert_eq!(transfer.message_count(), 3);
        // Unknown message shapes survive untouched for a lossless import.
        assert_eq!(transfer.messages[2]["type"], "future-kind");
        assert_eq!(transfer.messages[2]["extra"], true);

        let enveloped = format!(r#"{{"data": {raw}}}"#);
        let wrapped = SessionTransfer::parse(enveloped.as_bytes()).expect("enveloped transfer");
        assert_eq!(wrapped, transfer);

        assert!(SessionTransfer::parse(b"not json").is_err());
        assert!(SessionTransfer::parse(br#"{"messages": []}"#).is_err());
    }

    #[test]
    fn stats_without_tools_parses() {
        let raw = r#"{
            "range": { "from": 1700000000000, "to": 1800000000000 },
            "sessions": 12,
            "subagents": 3,
            "prompts": 40,
            "steps": 210,
            "tokens": {
                "input": 1000, "output": 200, "reasoning": 50,
                "cache": { "read": 500, "write": 10 }
            },
            "cost": 0.75,
            "activeDays": 6,
            "streak": 2,
            "activity": [{ "date": "2026-11-10", "steps": 20 }],
            "models": [{
                "model": { "id": "m", "providerID": "p", "variant": "default" },
                "steps": 190,
                "tokens": { "input": 900, "output": 180, "reasoning": 40,
                            "cache": { "read": 100, "write": 0 } },
                "cost": 0.5
            }]
        }"#;
        let stats: SessionStats = serde_json::from_str(raw).expect("stats without tools");
        assert_eq!(stats.range.from, 1700000000000);
        assert_eq!(stats.sessions, 12);
        assert_eq!(stats.subagents, 3);
        assert_eq!(stats.prompts, 40);
        assert_eq!(stats.steps, 210);
        assert_eq!(stats.tokens.input, 1000.0);
        assert_eq!(stats.tokens.cache.read, 500.0);
        assert_eq!(stats.cost, 0.75);
        assert_eq!(stats.tools, None);
        assert_eq!(stats.active_days, 6);
        assert_eq!(stats.streak, 2);
        assert_eq!(stats.activity[0].steps, 20);
        assert_eq!(stats.models[0].label(), "p/m");
        assert!(!stats.is_empty());

        let empty: SessionStats =
            serde_json::from_str(r#"{"range":{"from":1,"to":2}}"#).expect("empty range");
        assert!(empty.is_empty());
        assert_eq!(empty.models.len(), 0);
    }
}
