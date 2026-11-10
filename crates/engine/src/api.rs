//! Session commands against the OpenCode `/api` surface.

use std::path::Path;
use std::time::Duration;

use reqwest::{Client, Method, Response};

/// Authenticated client for one sidecar instance.
///
/// `directory` is canonicalized up front: OpenCode stores and filters location
/// paths in canonical form (on macOS `/tmp` becomes `/private/tmp`), and a raw
/// path would produce an empty session list.
/// Cloning is cheap: the underlying HTTP client and strings are shared.
#[derive(Clone)]
pub(crate) struct SessionApi {
    pub(crate) client: Client,
    pub(crate) base_url: String,
    pub(crate) password: String,
    pub(crate) directory: String,
}

impl SessionApi {
    pub fn new(port: u16, password: String, project_dir: &Path) -> Result<Self, String> {
        let directory = std::fs::canonicalize(project_dir)
            .unwrap_or_else(|_| project_dir.to_path_buf())
            .to_string_lossy()
            .into_owned();
        // No global timeout: the event stream is a long-lived response. Normal
        // requests set their own timeout below.
        let client = Client::builder()
            .build()
            .map_err(|err| format!("could not build the HTTP client: {err}"))?;
        Ok(Self {
            client,
            base_url: format!("http://127.0.0.1:{port}"),
            password,
            directory,
        })
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base_url))
            .timeout(Duration::from_secs(15))
            .basic_auth("opencode", Some(&self.password))
    }

    /// Open the server-sent event stream. The response is `text/event-stream`
    /// and stays open until the connection drops.
    pub async fn subscribe(&self) -> Result<Response, String> {
        let response = self
            .client
            .get(format!("{}/api/event", self.base_url))
            .basic_auth("opencode", Some(&self.password))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await
    }
}

pub(crate) async fn ensure_success(response: Response) -> Result<Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body = body.trim();
    if body.is_empty() {
        return Err(format!("OpenCode returned {status}"));
    }
    let body: String = body.chars().take(300).collect();
    Err(format!("OpenCode returned {status}: {body}"))
}

pub(crate) fn network_error(error: reqwest::Error) -> String {
    format!("request to OpenCode failed: {error}")
}

pub(crate) fn decode_error(error: reqwest::Error) -> String {
    format!("OpenCode returned an unreadable response: {error}")
}
