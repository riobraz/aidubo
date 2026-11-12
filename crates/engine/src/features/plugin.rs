//! Installed plugins.

use harness_protocol::{Envelope, PluginInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn plugins(&self, directory: &str) -> Result<Vec<PluginInfo>, String> {
        let response = self
            .request(Method::GET, "/api/plugin")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<PluginInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn update_plugins(
        &self,
        directory: &str,
        targets: Vec<String>,
    ) -> Result<(), String> {
        let response = self
            .request(Method::POST, "/api/plugin/update")
            .query(&[("location[directory]", directory)])
            .json(&serde_json::json!({ "targets": targets }))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn check_plugins(&self) -> Result<(), String> {
        let response = self
            .request(Method::POST, "/api/plugin/check")
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
