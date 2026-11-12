//! Worktree CRUD and session moves.

use harness_protocol::{
    CreateWorktreeRequest, MoveSessionRequest, RemoveWorktreeRequest, Worktree,
};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Every worktree the server lists for a project. The endpoint returns a
    /// bare array, not the usual `{data: ...}` envelope.
    pub async fn worktrees(&self, project_id: &str) -> Result<Vec<Worktree>, String> {
        let response = self
            .request(Method::GET, "/api/worktree")
            .query(&[("projectID", project_id)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        response.json().await.map_err(decode_error)
    }

    pub async fn create_worktree(
        &self,
        request: &CreateWorktreeRequest,
    ) -> Result<Worktree, String> {
        let response = self
            .request(Method::POST, "/api/worktree")
            .json(request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        response.json().await.map_err(decode_error)
    }

    pub async fn remove_worktree(&self, request: &RemoveWorktreeRequest) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, "/api/worktree")
            .json(request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// Ask the server to rediscover worktrees from the filesystem. A mutation
    /// that changes the list emits `worktree.*` events; this is the explicit
    /// resync for a stale picker.
    pub async fn refresh_worktrees(&self) -> Result<(), String> {
        let response = self
            .request(Method::POST, "/api/worktree/refresh")
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn move_session(&self, session_id: &str, directory: &str) -> Result<(), String> {
        let request = MoveSessionRequest {
            directory: directory.to_owned(),
        };
        let response = self
            .request(Method::POST, &format!("/api/session/{session_id}/move"))
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn serve_once(status: &str, body: &str) -> (u16, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let read = socket.read(&mut chunk).expect("read");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let Some(headers_end) = buffer
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|index| index + 4)
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&buffer[..headers_end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= headers_end + length {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).expect("write");
            String::from_utf8_lossy(&buffer).to_string()
        });
        (port, handle)
    }

    #[test]
    fn create_worktree_body_shape() {
        let (port, request) = serve_once("200 OK", r#"{"directory":"/tmp/project-wt"}"#);
        let api = SessionApi::new(port, "secret".into()).expect("api");
        let request_body = CreateWorktreeRequest {
            project_id: "p1".into(),
            from: "/tmp/project".into(),
            branch: Some("feature".into()),
            directory: None,
            name: "feature".into(),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let worktree = runtime
            .block_on(api.create_worktree(&request_body))
            .expect("create");
        assert_eq!(worktree.directory, "/tmp/project-wt");

        let raw = request.join().expect("request");
        assert!(raw.starts_with("POST /api/worktree "), "{raw}");
        assert!(raw.contains(r#""projectID":"p1""#), "{raw}");
        assert!(raw.contains(r#""branch":"feature""#), "{raw}");
        assert!(
            !raw.contains(r#""directory":"#),
            "the server chooses the path: {raw}"
        );
    }

    #[test]
    fn list_worktrees_sends_project_id_query() {
        let (port, request) = serve_once(
            "200 OK",
            r#"[{"directory":"/tmp/project-wt","strategy":"checkout"}]"#,
        );
        let api = SessionApi::new(port, "secret".into()).expect("api");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let worktrees = runtime.block_on(api.worktrees("p1")).expect("worktrees");
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].strategy.as_deref(), Some("checkout"));

        let raw = request.join().expect("request");
        assert!(raw.starts_with("GET /api/worktree?projectID=p1 "), "{raw}");
    }

    #[test]
    fn remove_worktree_sends_json_delete() {
        let (port, request) = serve_once("204 No Content", "");
        let api = SessionApi::new(port, "secret".into()).expect("api");
        let remove = RemoveWorktreeRequest {
            project_id: "p1".into(),
            directory: "/tmp/project-wt".into(),
            force: true,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime
            .block_on(api.remove_worktree(&remove))
            .expect("remove");

        let raw = request.join().expect("request");
        assert!(raw.starts_with("DELETE /api/worktree "), "{raw}");
        assert!(raw.contains(r#""force":true"#), "{raw}");
        assert!(raw.contains(r#""projectID":"p1""#), "{raw}");
    }
}
