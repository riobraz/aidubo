//! VCS and session diff reads.

use harness_protocol::{Envelope, FileDiff};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Working-tree, branch, or committed diff for the project.
    pub async fn vcs_diff(
        &self,
        mode: &str,
        base: Option<&str>,
        context: Option<&str>,
        untracked: &str,
    ) -> Result<Vec<FileDiff>, String> {
        let mut query: Vec<(&str, &str)> = vec![
            ("location[directory]", self.directory.as_str()),
            ("mode", mode),
            ("untracked", untracked),
        ];
        if let Some(base) = base {
            query.push(("base", base));
        }
        if let Some(context) = context {
            query.push(("context", context));
        }
        let response = self
            .request(Method::GET, "/api/vcs/diff")
            .query(&query)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<FileDiff>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    /// Files a session changed between two messages.
    pub async fn session_diff(
        &self,
        session_id: &str,
        from: Option<&str>,
        to: Option<&str>,
        context: Option<&str>,
    ) -> Result<Vec<FileDiff>, String> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(from) = from {
            query.push(("from", from));
        }
        if let Some(to) = to {
            query.push(("to", to));
        }
        if let Some(context) = context {
            query.push(("context", context));
        }
        let response = self
            .request(Method::GET, &format!("/api/session/{session_id}/diff"))
            .query(&query)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<FileDiff>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }
}
