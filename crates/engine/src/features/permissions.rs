//! Permission requests and replies.

use harness_protocol::{Envelope, PermissionReply, PermissionReplyRequest, PermissionRequest};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Pending permission requests for a session, oldest first.
    pub async fn permissions(&self, id: &str) -> Result<Vec<PermissionRequest>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{id}/permission"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: Envelope<Vec<PermissionRequest>> = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }

    /// Answer a permission request, unblocking the waiting turn. Replies are
    /// `204 No Content`.
    pub async fn reply_permission(
        &self,
        session_id: &str,
        request_id: &str,
        reply: PermissionReply,
    ) -> Result<(), String> {
        let request = PermissionReplyRequest {
            decision: reply,
            message: None,
        };
        let response = self
            .request(
                Method::POST,
                &format!("/api/session/{session_id}/permission/{request_id}/reply"),
            )
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
