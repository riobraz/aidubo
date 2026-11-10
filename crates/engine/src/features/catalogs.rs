//! Server-backed catalogs used by the composer pickers.

use harness_protocol::{AgentInfo, CommandInfo, Envelope, ModelInfo, SkillInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Slash commands available in this project.
    pub async fn commands(&self) -> Result<Vec<CommandInfo>, String> {
        self.catalog("/api/command").await
    }

    /// Skills available in this project.
    pub async fn skills(&self) -> Result<Vec<SkillInfo>, String> {
        self.catalog("/api/skill").await
    }

    /// Agents available in this project.
    pub async fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        self.catalog("/api/agent").await
    }

    /// Models available in this project.
    pub async fn models(&self) -> Result<Vec<ModelInfo>, String> {
        self.catalog("/api/model").await
    }

    async fn catalog<T>(&self, path: &str) -> Result<Vec<T>, String>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .request(Method::GET, path)
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: Envelope<Vec<T>> = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }
}
