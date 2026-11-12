//! Saved permission rules.

use harness_protocol::{Envelope, SavedPermission};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn saved_permissions(
        &self,
        project_id: &str,
    ) -> Result<Vec<SavedPermission>, String> {
        let response = self
            .request(Method::GET, "/api/permission/saved")
            .query(&[("projectID", project_id)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<SavedPermission>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn delete_saved_permission(&self, id: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/permission/saved/{id}"))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}
