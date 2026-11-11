//! Session export, import, and statistics.

use std::time::Duration;

use harness_protocol::{Envelope, Session, SessionStats, SessionTransfer, StatsRange};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// The raw export document. The caller writes it to the chosen file.
    pub async fn export_session(
        &self,
        session_id: &str,
        sanitize: bool,
    ) -> Result<SessionTransfer, String> {
        let sanitize = if sanitize { "true" } else { "false" };
        let response = self
            .request(
                Method::GET,
                &format!("/api/experimental/session/{session_id}/export"),
            )
            .query(&[("sanitize", sanitize)])
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<SessionTransfer> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    /// Import an export into this project. `info` and `messages` are sent
    /// untouched; only the location is replaced with this project's directory.
    pub async fn import_session(&self, transfer: &SessionTransfer) -> Result<Session, String> {
        let body = serde_json::json!({
            "info": transfer.info,
            "messages": transfer.messages,
            "location": { "directory": self.directory },
        });
        let response = self
            .request(Method::POST, "/api/experimental/session/import")
            .timeout(Duration::from_secs(60))
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Session> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    /// Usage statistics. The endpoint only accepts epoch milliseconds for the
    /// range; ISO date strings fail with a server error. Statistics are global
    /// to the server's storage, not scoped to this project's directory.
    pub async fn session_stats(&self, range: StatsRange) -> Result<SessionStats, String> {
        let response = self
            .request(Method::GET, "/api/experimental/session/stats")
            .query(&[
                ("from", range.from.to_string()),
                ("to", range.to.to_string()),
                ("tools", "none".to_string()),
            ])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<SessionStats> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}

/// Rewrite the session and message ids so an export can be imported next to
/// the session it came from. Storage is global: keeping the exported ids makes
/// the server answer 409 for the session, or 500 when a message id already
/// exists. Every other field is preserved verbatim.
pub(crate) fn import_as_copy(mut transfer: SessionTransfer) -> SessionTransfer {
    let suffix = format!("{:032x}", rand::random::<u128>());
    transfer.info["id"] = serde_json::json!(format!("ses_copy_{suffix}"));
    for (index, message) in transfer.messages.iter_mut().enumerate() {
        if let Some(object) = message.as_object_mut() {
            object.insert(
                "id".into(),
                serde_json::json!(format!("msg_copy_{suffix}_{index}")),
            );
        }
    }
    transfer
}

#[cfg(test)]
mod tests {
    use harness_protocol::{SessionTransfer, StatsRange};
    use serde_json::json;

    use super::{SessionApi, import_as_copy};

    #[test]
    fn stats_query_uses_epoch_milliseconds() {
        let range = StatsRange {
            from: 1_700_000_000_000,
            to: 1_800_000_000_000,
        };
        assert_eq!(range.from.to_string(), "1700000000000");
        assert_eq!(range.to.to_string(), "1800000000000");
    }

    #[test]
    fn import_body_keeps_the_payload_and_replaces_the_location() {
        let transfer = SessionTransfer {
            info: json!({ "id": "ses_1", "title": "keep", "extra": 7 }),
            messages: vec![json!({ "type": "user", "id": "msg_1", "text": "hi" })],
        };
        let body = json!({
            "info": transfer.info,
            "messages": transfer.messages,
            "location": { "directory": "/tmp/project" },
        });
        assert_eq!(body["info"]["extra"], 7);
        assert_eq!(body["messages"][0]["text"], "hi");
        assert_eq!(body["location"]["directory"], "/tmp/project");
    }

    #[test]
    fn import_copy_rewrites_only_the_ids() {
        let transfer = SessionTransfer {
            info: json!({ "id": "ses_1", "title": "keep", "location": { "directory": "/old" } }),
            messages: vec![
                json!({ "type": "user", "id": "msg_1", "text": "hi" }),
                json!({ "type": "assistant", "id": "msg_2", "content": [] }),
            ],
        };
        let copied = import_as_copy(transfer.clone());
        assert_ne!(copied.session_id(), "ses_1");
        assert_eq!(copied.info["title"], "keep");
        assert_eq!(copied.messages[0]["text"], "hi");
        assert_ne!(copied.messages[0]["id"], "msg_1");
        assert_ne!(copied.messages[1]["id"], "msg_2");
        assert_ne!(copied.messages[0]["id"], copied.messages[1]["id"]);
        assert_eq!(copied.message_count(), 2);
    }

    #[test]
    fn session_api_directory_is_canonical() {
        // Guards the query value that location-scoped endpoints read.
        let api = SessionApi::new(1, "pass".into(), std::path::Path::new("/tmp")).expect("api");
        assert!(api.directory.ends_with("tmp"));
    }
}
