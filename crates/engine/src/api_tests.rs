use std::io::{Read as _, Write as _};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use harness_protocol::{
    AgentAttachment, FileSource, MentionRange, ModelRef, PromptFile, PromptRequest, SkillAttachment,
};

use crate::api::SessionApi;

/// Answer exactly one request with a fixed response and return it verbatim.
fn serve_once(status: &str, body: &str) -> (u16, Receiver<String>) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind stub server");
    let port = listener.local_addr().expect("stub address").port();
    let status = status.to_owned();
    let body = body.to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept request");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let read = socket.read(&mut chunk).expect("read request");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(head_end) = find(&buffer, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buffer[..head_end]).to_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= head_end + 4 + length {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .expect("write response");
        let _ = sender.send(String::from_utf8_lossy(&buffer).to_string());
    });
    (port, receiver)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn api(port: u16) -> SessionApi {
    SessionApi::new(port, "secret".into(), &std::env::temp_dir()).expect("api")
}

fn body_of(request: &str) -> serde_json::Value {
    let body = request.split("\r\n\r\n").nth(1).expect("request body");
    serde_json::from_str(body).expect("json body")
}

fn receive(receiver: Receiver<String>) -> String {
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("stub received the request")
}

#[tokio::test]
async fn prompt_posts_files_agents_and_skills() {
    let (port, receiver) = serve_once("200 OK", r#"{"data":{"id":"user_1","sessionID":"s1"}}"#);
    let request = PromptRequest {
        text: "read @src/main.rs".into(),
        files: vec![PromptFile {
            uri: "file:///tmp/p/src/main.rs".into(),
            source: Some(FileSource::Uri {
                uri: "file:///tmp/p/src/main.rs".into(),
            }),
            name: Some("main.rs".into()),
            mention: Some(MentionRange {
                start: 5,
                end: 17,
                text: "src/main.rs".into(),
            }),
        }],
        agents: vec![AgentAttachment {
            name: "build".into(),
            mention: None,
        }],
        skills: vec![SkillAttachment {
            id: "opencode".into(),
            mention: None,
        }],
    };

    let accepted = api(port).prompt("s1", request).await.expect("prompt");
    assert_eq!(accepted.id, "user_1");
    assert_eq!(accepted.session_id, "s1");

    let raw = receive(receiver);
    assert!(raw.starts_with("POST /api/session/s1/prompt HTTP/1.1"));
    let body = body_of(&raw);
    assert_eq!(body["text"], "read @src/main.rs");
    assert_eq!(body["files"][0]["uri"], "file:///tmp/p/src/main.rs");
    assert_eq!(body["files"][0]["name"], "main.rs");
    assert_eq!(body["files"][0]["mention"]["start"], 5);
    assert_eq!(body["files"][0]["mention"]["end"], 17);
    assert_eq!(body["agents"][0]["name"], "build");
    assert_eq!(body["skills"][0]["id"], "opencode");
}

#[tokio::test]
async fn find_files_sends_the_location_query_and_decodes_entries() {
    let (port, receiver) = serve_once(
        "200 OK",
        r#"{"location":"/tmp","data":[{"path":"src/main.rs","type":"file"},{"path":"src/lib.rs","type":"file"}]}"#,
    );

    let files = api(port).find_files("src/", 24).await.expect("files");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "src/main.rs");
    assert_eq!(files[0].kind, "file");

    let raw = receive(receiver);
    assert!(raw.starts_with("GET /api/fs/find?"));
    assert!(raw.contains("location%5Bdirectory%5D="));
    assert!(raw.contains("query=src%2F"));
    assert!(raw.contains("type=file"));
    assert!(raw.contains("limit=24"));
}

#[tokio::test]
async fn catalogs_ignore_unknown_fields_and_reject_malformed_payloads() {
    let (port, receiver) = serve_once(
        "200 OK",
        r#"{"data":[{"id":"c1","modelID":"claude-x","providerID":"anthropic","name":"Claude X","enabled":true,"extra":{"nested":1}}]}"#,
    );
    let models = api(port).models().await.expect("models");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].wire_id(), "claude-x");
    assert_eq!(
        models[0].model_ref(),
        ModelRef {
            id: "claude-x".into(),
            provider_id: Some("anthropic".into()),
            variant: None,
        }
    );
    let raw = receive(receiver);
    assert!(raw.starts_with("GET /api/model?location%5Bdirectory%5D="));

    let (port, receiver) = serve_once("200 OK", "not json");
    let error = api(port).skills().await.expect_err("malformed catalog");
    assert!(error.contains("unreadable response"), "{error}");
    let _ = receive(receiver);
}

#[tokio::test]
async fn model_and_agent_changes_post_the_expected_bodies() {
    let (port, receiver) = serve_once("204 No Content", "");
    api(port)
        .set_model(
            "s1",
            ModelRef {
                id: "claude-x".into(),
                provider_id: Some("anthropic".into()),
                variant: None,
            },
        )
        .await
        .expect("set model");
    let raw = receive(receiver);
    assert!(raw.starts_with("POST /api/session/s1/model HTTP/1.1"));
    assert_eq!(
        body_of(&raw),
        serde_json::json!({"model": {"id": "claude-x", "providerID": "anthropic"}})
    );

    let (port, receiver) = serve_once("204 No Content", "");
    api(port)
        .set_agent("s1", "plan".into())
        .await
        .expect("set agent");
    let raw = receive(receiver);
    assert!(raw.starts_with("POST /api/session/s1/agent HTTP/1.1"));
    assert_eq!(body_of(&raw), serde_json::json!({"agent": "plan"}));
}

#[tokio::test]
async fn command_posts_arguments_and_attachments() {
    let (port, receiver) = serve_once("204 No Content", "");
    api(port)
        .command(
            "s1",
            harness_protocol::CommandRequest {
                name: "review".into(),
                text: "HEAD~1".into(),
                ..Default::default()
            },
        )
        .await
        .expect("command");
    let raw = receive(receiver);
    assert!(raw.starts_with("POST /api/session/s1/command HTTP/1.1"));
    assert_eq!(
        body_of(&raw),
        serde_json::json!({"name": "review", "text": "HEAD~1"})
    );
}
