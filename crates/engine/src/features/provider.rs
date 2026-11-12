//! Provider discovery and integration auth flows.
//!
//! The API key exists only as a [`Secret`], passed by reference into this
//! module, serialized into one request body, and wiped when dropped. It is
//! never stored in a command field beyond the send, never returned in an
//! event, and never logged.

use harness_protocol::{
    AttemptStatus, CommandAttempt, CommandStartRequest, Envelope, ExternalStartRequest,
    IntegrationInfo, OAuthAttempt, OAuthCompleteRequest, OAuthStartRequest, ProviderInfo,
    SendKeyRequest,
};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

/// A credential held only long enough to send it.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Overwrite the backing allocation before releasing it. `replace_range`
        // keeps the byte length valid UTF-8 (NUL) while clearing the secret.
        let zeros = "\0".repeat(self.0.len());
        self.0.replace_range(.., &zeros);
        self.0.clear();
    }
}

fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

impl SessionApi {
    fn integration_path(&self, parts: &[&str]) -> String {
        let mut path = String::from("/api/integration");
        for part in parts {
            path.push('/');
            path.push_str(&encode_segment(part));
        }
        path
    }

    fn located(
        &self,
        request: reqwest::RequestBuilder,
        directory: &str,
    ) -> reqwest::RequestBuilder {
        request.query(&[("location[directory]", directory)])
    }

    pub async fn providers(&self, directory: &str) -> Result<Vec<ProviderInfo>, String> {
        let response = self
            .request(Method::GET, "/api/provider")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<ProviderInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn integrations(&self, directory: &str) -> Result<Vec<IntegrationInfo>, String> {
        let response = self
            .request(Method::GET, "/api/integration")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<IntegrationInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn start_oauth(
        &self,
        directory: &str,
        integration_id: &str,
        method_id: &str,
    ) -> Result<OAuthAttempt, String> {
        let path = self.integration_path(&[integration_id, "connect", "oauth"]);
        let response = self
            .located(self.request(Method::POST, &path), directory)
            .json(&OAuthStartRequest {
                method_id: method_id.to_string(),
                label: None,
            })
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<OAuthAttempt> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn oauth_status(
        &self,
        directory: &str,
        integration_id: &str,
        attempt_id: &str,
    ) -> Result<AttemptStatus, String> {
        let path = self.integration_path(&[integration_id, "connect", "oauth", attempt_id]);
        let response = self
            .located(self.request(Method::GET, &path), directory)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<AttemptStatus> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn cancel_oauth(
        &self,
        directory: &str,
        integration_id: &str,
        attempt_id: &str,
    ) -> Result<(), String> {
        let path = self.integration_path(&[integration_id, "connect", "oauth", attempt_id]);
        let response = self
            .located(self.request(Method::DELETE, &path), directory)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn complete_oauth(
        &self,
        directory: &str,
        integration_id: &str,
        attempt_id: &str,
        code: Option<String>,
    ) -> Result<(), String> {
        let path =
            self.integration_path(&[integration_id, "connect", "oauth", attempt_id, "complete"]);
        let response = self
            .located(self.request(Method::POST, &path), directory)
            .json(&OAuthCompleteRequest { code })
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn start_command(
        &self,
        directory: &str,
        integration_id: &str,
        method_id: &str,
    ) -> Result<CommandAttempt, String> {
        let path = self.integration_path(&[integration_id, "connect", "command"]);
        let response = self
            .located(self.request(Method::POST, &path), directory)
            .json(&CommandStartRequest {
                method_id: method_id.to_string(),
                label: None,
            })
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<CommandAttempt> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn command_status(
        &self,
        directory: &str,
        integration_id: &str,
        attempt_id: &str,
    ) -> Result<AttemptStatus, String> {
        let path = self.integration_path(&[integration_id, "connect", "command", attempt_id]);
        let response = self
            .located(self.request(Method::GET, &path), directory)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<AttemptStatus> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn cancel_command(
        &self,
        directory: &str,
        integration_id: &str,
        attempt_id: &str,
    ) -> Result<(), String> {
        let path = self.integration_path(&[integration_id, "connect", "command", attempt_id]);
        let response = self
            .located(self.request(Method::DELETE, &path), directory)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn send_key(
        &self,
        directory: &str,
        integration_id: &str,
        key: &Secret,
    ) -> Result<(), String> {
        let path = self.integration_path(&[integration_id, "connect", "key"]);
        let response = self
            .located(self.request(Method::POST, &path), directory)
            .json(&SendKeyRequest {
                key: key.expose().to_string(),
                label: None,
            })
            .send()
            .await
            .map_err(network_error)?;
        // The server could echo request material in an error body. Scrub the
        // key before the message leaves this function; the UI redacts the rest.
        ensure_success(response)
            .await
            .map(|_| ())
            .map_err(|message| message.replace(key.expose(), "[redacted]"))
    }

    pub async fn connect_external(
        &self,
        directory: &str,
        integration_id: &str,
        method_id: &str,
    ) -> Result<(), String> {
        let path = self.integration_path(&[integration_id, "connect", "external"]);
        let response = self
            .located(self.request(Method::POST, &path), directory)
            .json(&ExternalStartRequest {
                method_id: method_id.to_string(),
                label: None,
            })
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await.map(|_| ())
    }
}
