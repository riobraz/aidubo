//! Prompt, command, interrupt, and session settings writes.

use harness_protocol::{
    AcceptedPrompt, CommandRequest, Envelope, InterruptResponse, ModelRef, PromptRequest,
    SetAgentRequest, SetModelRequest,
};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Queue a prompt for the session. The turn itself runs asynchronously and
    /// is observed through the event stream.
    pub async fn prompt(&self, id: &str, request: PromptRequest) -> Result<AcceptedPrompt, String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/prompt"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let accepted: Envelope<AcceptedPrompt> = response.json().await.map_err(decode_error)?;
        Ok(accepted.data)
    }

    /// Run a slash command in the session. The server replies `204`.
    pub async fn command(&self, id: &str, request: CommandRequest) -> Result<(), String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/command"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// Switch the session's model. The server replies `204`; the new value is
    /// confirmed by the next session snapshot or selection event.
    pub async fn set_model(&self, id: &str, model: ModelRef) -> Result<(), String> {
        let request = SetModelRequest { model };
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/model"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// Switch the session's agent. The server replies `204`.
    pub async fn set_agent(&self, id: &str, agent: String) -> Result<(), String> {
        let request = SetAgentRequest { agent };
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/agent"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// Interrupt the running turn. Returns whether anything was interrupted.
    pub async fn interrupt(&self, id: &str) -> Result<bool, String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/interrupt"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let answer: InterruptResponse = response.json().await.map_err(decode_error)?;
        Ok(answer.interrupted)
    }
}
