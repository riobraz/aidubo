//! Provider budget limits.

use harness_protocol::{Envelope, UsageLimits};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Limits are optional: a 503 means "not available for this workspace".
    pub async fn usage_limits(&self, directory: &str) -> Result<Option<UsageLimits>, String> {
        let response = self
            .request(Method::GET, "/api/experimental/usage/limits")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Ok(None);
        }
        let response = ensure_success(response).await?;
        let page: Envelope<Option<UsageLimits>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}
