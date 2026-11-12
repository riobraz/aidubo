//! Session revert: stage, commit, clear.

use harness_protocol::{Envelope, RevertStageRequest, SessionRevert};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn stage_revert(
        &self,
        session_id: &str,
        message_id: String,
        files: Option<bool>,
    ) -> Result<SessionRevert, String> {
        let body = RevertStageRequest { message_id, files };
        let response = self
            .request(
                Method::POST,
                &format!("/api/session/{session_id}/revert/stage"),
            )
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<SessionRevert> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn commit_revert(&self, session_id: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/session/{session_id}/revert/commit"),
            )
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn clear_revert(&self, session_id: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/session/{session_id}/revert"))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
