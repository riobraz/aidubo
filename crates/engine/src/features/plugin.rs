//! Installed plugins.

use harness_protocol::{Envelope, PluginInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn plugins(&self) -> Result<Vec<PluginInfo>, String> {
        let response = self
            .request(Method::GET, "/api/plugin")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<PluginInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
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
