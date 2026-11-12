//! Context compaction and usage reads.

use harness_protocol::{Envelope, SessionMessage};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Ask the server to compact the session. The turn is asynchronous.
    pub async fn compact(&self, session_id: &str) -> Result<(), String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{session_id}/compact"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// The messages the model would see for the session.
    pub async fn context(&self, session_id: &str) -> Result<Vec<SessionMessage>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{session_id}/context"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<SessionMessage>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}
