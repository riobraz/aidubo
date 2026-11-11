//! Provider and integration discovery.

use harness_protocol::{Envelope, IntegrationInfo, ProviderInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn providers(&self) -> Result<Vec<ProviderInfo>, String> {
        let response = self
            .request(Method::GET, "/api/provider")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<ProviderInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn integrations(&self) -> Result<Vec<IntegrationInfo>, String> {
        let response = self
            .request(Method::GET, "/api/integration")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<IntegrationInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}
