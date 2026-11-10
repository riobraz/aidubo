//! Session timeline reads.

use harness_protocol::{MessagePage, SessionMessage};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// The session timeline, oldest first.
    pub async fn messages(&self, id: &str) -> Result<Vec<SessionMessage>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{id}/message"))
            .query(&[("order", "asc")])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: MessagePage = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}
