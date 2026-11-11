//! MCP servers.

use harness_protocol::{Envelope, McpServer};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn mcp_servers(&self) -> Result<Vec<McpServer>, String> {
        let response = self
            .request(Method::GET, "/api/mcp")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<McpServer>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn add_mcp(&self, name: &str, config: serde_json::Value) -> Result<(), String> {
        let response = self
            .request(Method::PUT, &format!("/api/experimental/mcp/{name}"))
            .json(&config)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn remove_mcp(&self, name: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/experimental/mcp/{name}"))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn connect_mcp(&self, name: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/experimental/mcp/{name}/connect"),
            )
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn disconnect_mcp(&self, name: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/experimental/mcp/{name}/disconnect"),
            )
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
