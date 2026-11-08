//! Session commands against the OpenCode `/api` surface.

use std::path::Path;
use std::time::Duration;

use harness_protocol::{
    AcceptedPrompt, CreateSessionRequest, Envelope, InterruptResponse, MessagePage,
    PermissionReply, PermissionReplyRequest, PermissionRequest, PromptRequest, Session,
    SessionList, SessionLocation, SessionMessage, UpdateSessionRequest,
};
use reqwest::{Client, Method, Response};

/// Authenticated client for one sidecar instance.
///
/// `directory` is canonicalized up front: OpenCode stores and filters location
/// paths in canonical form (on macOS `/tmp` becomes `/private/tmp`), and a raw
/// path would produce an empty session list.
/// Cloning is cheap: the underlying HTTP client and strings are shared.
#[derive(Clone)]
pub(crate) struct SessionApi {
    client: Client,
    base_url: String,
    password: String,
    directory: String,
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

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base_url))
            .timeout(Duration::from_secs(15))
            .basic_auth("opencode", Some(&self.password))
    }

    /// Root sessions in this project, newest first.
    pub async fn list(&self) -> Result<Vec<Session>, String> {
        let response = self
            .request(Method::GET, "/api/session")
            .query(&[
                ("directory", self.directory.as_str()),
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

    /// Create a root session pinned to this project.
    pub async fn create(&self) -> Result<Session, String> {
        let request = CreateSessionRequest {
            title: None,
            location: Some(SessionLocation {
                directory: self.directory.clone(),
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

    /// Queue a prompt for the session. The turn itself runs asynchronously and
    /// is observed through the event stream.
    pub async fn prompt(&self, id: &str, text: String) -> Result<AcceptedPrompt, String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/prompt"))
            .json(&PromptRequest { text })
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let accepted: Envelope<AcceptedPrompt> = response.json().await.map_err(decode_error)?;
        Ok(accepted.data)
    }

    /// Interrupt the running turn. Returns whether anything was interrupted.
    pub async fn interrupt(&self, id: &str) -> Result<bool, String> {
        let response = self
            .request(Method::POST, &format!("/api/session/{id}/interrupt"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let answer: InterruptResponse = response.json().await.map_err(decode_error)?;
        Ok(answer.interrupted)
    }

    /// Pending permission requests for a session, oldest first.
    pub async fn permissions(&self, id: &str) -> Result<Vec<PermissionRequest>, String> {
        let response = self
            .request(Method::GET, &format!("/api/session/{id}/permission"))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: Envelope<Vec<PermissionRequest>> = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }

    /// Answer a permission request, unblocking the waiting turn. Replies are
    /// `204 No Content`.
    pub async fn reply_permission(
        &self,
        session_id: &str,
        request_id: &str,
        reply: PermissionReply,
    ) -> Result<(), String> {
        let request = PermissionReplyRequest {
            decision: reply,
            message: None,
        };
        let response = self
            .request(
                Method::POST,
                &format!("/api/session/{session_id}/permission/{request_id}/reply"),
            )
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
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

async fn ensure_success(response: Response) -> Result<Response, String> {
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

fn network_error(error: reqwest::Error) -> String {
    format!("request to OpenCode failed: {error}")
}

fn decode_error(error: reqwest::Error) -> String {
    format!("OpenCode returned an unreadable response: {error}")
}
