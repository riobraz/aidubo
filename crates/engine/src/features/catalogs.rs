//! Server-backed catalogs used by the composer pickers.

use harness_protocol::{AgentInfo, CommandInfo, Envelope, ModelInfo, SkillInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Slash commands available in `directory`.
    pub async fn commands(&self, directory: &str) -> Result<Vec<CommandInfo>, String> {
        self.catalog("/api/command", directory).await
    }

    /// Skills available in `directory`.
    pub async fn skills(&self, directory: &str) -> Result<Vec<SkillInfo>, String> {
        self.catalog("/api/skill", directory).await
    }

    /// Agents available in `directory`.
    pub async fn agents(&self, directory: &str) -> Result<Vec<AgentInfo>, String> {
        self.catalog("/api/agent", directory).await
    }

    /// Models available in `directory`.
    pub async fn models(&self, directory: &str) -> Result<Vec<ModelInfo>, String> {
        self.catalog("/api/model", directory).await
    }

    async fn catalog<T>(&self, path: &str, directory: &str) -> Result<Vec<T>, String>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .request(Method::GET, path)
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: Envelope<Vec<T>> = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }
}
