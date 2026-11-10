//! Inbox wire types: prompts waiting for or steering a running turn.

use serde::{Deserialize, Serialize};

/// How a prompt enters the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InboxDelivery {
    Steer,
    Queue,
}

/// Creation time of an inbox item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct InboxTime {
    #[serde(default)]
    pub created: i64,
}

/// One item as `GET /api/session/{id}/inbox` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InboxInfo {
    pub id: String,
    #[serde(rename = "sessionID", default)]
    pub session_id: String,
    #[serde(default)]
    pub time: InboxTime,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub delivery: Option<InboxDelivery>,
}

impl InboxInfo {
    /// The prompt text, when the item carries one.
    pub fn text(&self) -> &str {
        self.payload
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    }
}

/// Body for `PATCH /api/session/{id}/inbox/{item}`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct InboxDeliveryRequest {
    pub delivery: InboxDelivery,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_item_parses_user_and_compaction() {
        let user: InboxInfo = serde_json::from_str(
            r#"{"id":"in_1","sessionID":"s1","type":"user","payload":{"text":"next"},"delivery":"queue","time":{"created":7}}"#,
        )
        .expect("user");
        assert_eq!(user.text(), "next");
        assert_eq!(user.delivery, Some(InboxDelivery::Queue));

        let compaction: InboxInfo = serde_json::from_str(
            r#"{"id":"in_2","sessionID":"s1","type":"compaction","payload":{"id":"m1"},"delivery":"steer","time":{"created":8}}"#,
        )
        .expect("compaction");
        assert_eq!(compaction.text(), "");
        assert_eq!(compaction.delivery, Some(InboxDelivery::Steer));
    }

    #[test]
    fn inbox_unknown_type_is_kept() {
        let item: InboxInfo =
            serde_json::from_str(r#"{"id":"in_3","type":"future.thing","payload":{}}"#)
                .expect("unknown");
        assert_eq!(item.kind, "future.thing");
        assert_eq!(item.session_id, "");
        assert_eq!(item.delivery, None);
    }

    #[test]
    fn inbox_delivery_request_serializes_lowercase() {
        let json = serde_json::to_string(&InboxDeliveryRequest {
            delivery: InboxDelivery::Steer,
        })
        .expect("json");
        assert_eq!(json, r#"{"delivery":"steer"}"#);
    }
}
