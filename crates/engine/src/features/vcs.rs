//! Working-tree status reads and repository initialization.

use harness_protocol::{Envelope, VcsBase, VcsFileStatus};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn vcs_status(&self) -> Result<Vec<VcsFileStatus>, String> {
        let response = self
            .request(Method::GET, "/api/vcs/status")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<VcsFileStatus>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn vcs_base(&self) -> Result<Option<VcsBase>, String> {
        let response = self
            .request(Method::GET, "/api/vcs/base")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Option<VcsBase>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn vcs_init(&self) -> Result<(), String> {
        let response = self
            .request(Method::POST, "/api/vcs/init")
            .query(&[("location[directory]", self.directory.as_str())])
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
