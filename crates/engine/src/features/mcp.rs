//! MCP servers.

use harness_protocol::{Envelope, McpServer};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

/// The experimental endpoint requires the server config wrapped in a
/// `config` object; anything but a JSON object is rejected before the request.
pub(crate) fn mcp_config_body(config: serde_json::Value) -> Result<serde_json::Value, String> {
    if !config.is_object() {
        return Err("MCP config must be a JSON object.".into());
    }
    Ok(serde_json::json!({ "config": config }))
}

impl SessionApi {
    pub async fn mcp_servers(&self, directory: &str) -> Result<Vec<McpServer>, String> {
        let response = self
            .request(Method::GET, "/api/mcp")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<McpServer>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn add_mcp(
        &self,
        directory: &str,
        name: &str,
        config: serde_json::Value,
    ) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("MCP server needs a name.".into());
        }
        let body = mcp_config_body(config)?;
        let response = self
            .request(Method::PUT, &format!("/api/experimental/mcp/{name}"))
            .query(&[("location[directory]", directory)])
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn remove_mcp(&self, directory: &str, name: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/experimental/mcp/{name}"))
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn connect_mcp(&self, directory: &str, name: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/experimental/mcp/{name}/connect"),
            )
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn disconnect_mcp(&self, directory: &str, name: &str) -> Result<(), String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/experimental/mcp/{name}/disconnect"),
            )
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::mcp_config_body;

    #[test]
    fn mcp_add_body_is_valid_json() {
        let body = mcp_config_body(json!({
            "type": "local",
            "command": ["mcp-echo"]
        }))
        .expect("valid config");
        assert_eq!(
            body,
            json!({ "config": { "type": "local", "command": ["mcp-echo"] } })
        );

        for invalid in [json!([]), json!("local"), json!(null), json!(7)] {
            assert!(
                mcp_config_body(invalid.clone()).is_err(),
                "{invalid} must be rejected"
            );
        }
    }
}
