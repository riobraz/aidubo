//! Session CRUD against the project-scoped API.

use harness_protocol::{
    CreateSessionRequest, Envelope, Session, SessionList, SessionLocation, UpdateSessionRequest,
};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Root sessions in `directory`, newest first.
    pub async fn list(&self, directory: &str) -> Result<Vec<Session>, String> {
        let response = self
            .request(Method::GET, "/api/session")
            .query(&[
                ("directory", directory),
                ("parentID", "null"),
                ("limit", "100"),
                ("order", "desc"),
            ])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: SessionList = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }

    /// Create a root session pinned to `directory`.
    pub async fn create(&self, directory: &str) -> Result<Session, String> {
        let request = CreateSessionRequest {
            title: None,
            location: Some(SessionLocation {
                directory: directory.to_owned(),
            }),
        };
        let response = self
            .request(Method::POST, "/api/session")
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let envelope: Envelope<Session> = response.json().await.map_err(decode_error)?;
        Ok(envelope.data)
    }

    pub async fn rename(&self, id: &str, title: String) -> Result<(), String> {
        let request = UpdateSessionRequest { title: Some(title) };
        let response = self
            .request(Method::PATCH, &format!("/api/session/{id}"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/session/{id}"))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}

impl SessionApi {
    /// Fork a session before the given message (or the whole session).
    pub async fn fork(&self, id: &str, before: Option<String>) -> Result<Session, String> {
        let request = harness_protocol::ForkRequest { before };
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/fork"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let envelope: Envelope<Session> = response.json().await.map_err(decode_error)?;
        Ok(envelope.data)
    }
}
