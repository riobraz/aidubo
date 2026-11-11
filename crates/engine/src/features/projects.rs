//! Project registry against `GET /api/project`.

use harness_protocol::Project;
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Every non-archived project the server knows about.
    ///
    /// This endpoint returns a bare array, unlike the `/api/session` envelope.
    pub async fn projects(&self) -> Result<Vec<Project>, String> {
        let response = self
            .request(Method::GET, "/api/project")
            .query(&[("archived", "false")])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        response.json().await.map_err(decode_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    #[tokio::test]
    async fn projects_accepts_a_bare_array() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
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
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = r#"[{"id":"p1","canonical":"/tmp/one"}]"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).expect("write");
            String::from_utf8_lossy(&buffer).to_string()
        });

        let api =
            SessionApi::new(port, "secret".into(), std::path::Path::new("/tmp")).expect("api");
        let projects = api.projects().await.expect("projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id.as_str(), "p1");

        let request = handle.join().expect("server thread");
        let request_line = request.lines().next().unwrap_or_default();
        assert!(request_line.starts_with("GET /api/project?"));
        assert!(request_line.contains("archived=false"));
    }
}
