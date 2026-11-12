//! Agent forms: list, reply, dismiss.

use harness_protocol::{Envelope, FormInfo, FormReplyRequest};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn forms(&self, session_id: &str) -> Result<Vec<FormInfo>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{session_id}/form"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<FormInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn reply_form(
        &self,
        session_id: &str,
        form_id: &str,
        answer: serde_json::Value,
    ) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/session/{session_id}/form/{form_id}/reply"),
            )
            .json(&FormReplyRequest { answer })
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn create_form(
        &self,
        session_id: &str,
        title: String,
        fields: serde_json::Value,
    ) -> Result<FormInfo, String> {
        let body = serde_json::json!({ "title": title, "fields": fields });
        let response = self
            .request(Method::POST, &format!("/api/session/{session_id}/form"))
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<FormInfo> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn dismiss_form(&self, session_id: &str, form_id: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::DELETE,
                &format!("/api/session/{session_id}/form/{form_id}"),
            )
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
