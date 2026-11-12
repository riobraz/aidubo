//! Prompts queued for or steering the running turn.

use harness_protocol::{Envelope, InboxDelivery, InboxDeliveryRequest, InboxInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn inbox(&self, session_id: &str) -> Result<Vec<InboxInfo>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{session_id}/inbox"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<InboxInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn cancel_inbox(&self, session_id: &str, inbox_id: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::DELETE,
                &format!("/api/session/{session_id}/inbox/{inbox_id}"),
            )
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn set_inbox_delivery(
        &self,
        session_id: &str,
        inbox_id: &str,
        delivery: InboxDelivery,
    ) -> Result<(), String> {
        let response = self
            .request(
                Method::PATCH,
                &format!("/api/session/{session_id}/inbox/{inbox_id}"),
            )
            .json(&InboxDeliveryRequest { delivery })
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
