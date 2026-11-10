//! An OpenAI-compatible mock provider driven by a JSON script.
//!
//! Written with the standard library only: one non-blocking listener thread,
//! one thread per connection. It answers `GET /v1/models` and
//! `POST /v1/chat/completions` with Server-Sent Events, consuming one script
//! turn per completion request in order. The last turn repeats.
//!
//! The script path comes from `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER_SCRIPT`.
//! Without it, the mock streams a single short turn so a test can boot the
//! engine and see text without any setup.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

/// One scripted completion.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Turn {
    #[serde(default)]
    pub chunks: Vec<String>,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub tool: Option<ToolCall>,
}

/// A tool call the mock asks for instead of text.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// The script: an ordered list of turns.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Script {
    #[serde(default)]
    pub turns: Vec<Turn>,
}

impl Script {
    pub fn single(text: &str) -> Self {
        Self {
            turns: vec![Turn {
                chunks: vec![text.to_string()],
                ..Turn::default()
            }],
        }
    }

    /// Load the script named by the environment, or a single default turn.
    pub fn from_env() -> Self {
        let Some(path) = std::env::var_os("RUST_HARNESS_MOCK_SCRIPTED_PROVIDER_SCRIPT") else {
            return Self::single("Scripted mock is running.");
        };
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read script {}: {error}", path.to_string_lossy()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("parse script {}: {error}", path.to_string_lossy()))
    }
}

/// A running mock provider. Dropping it stops the listener and releases the port.
pub struct MockProvider {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockProvider {
    pub fn start(script: Script) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock provider");
        let port = listener.local_addr().expect("mock address").port();
        listener
            .set_nonblocking(true)
            .expect("non-blocking mock listener");
        let stop = Arc::new(AtomicBool::new(false));
        let turns = Arc::new(script);
        let cursor = Arc::new(Mutex::new(0_usize));

        let stop_flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let turns = Arc::clone(&turns);
                        let cursor = Arc::clone(&cursor);
                        std::thread::spawn(move || handle_connection(stream, turns, cursor));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            stop,
            handle: Some(handle),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock accept() where the platform supports it; the loop also polls.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, script: Arc<Script>, cursor: Arc<Mutex<usize>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    if std::env::var_os("RUST_HARNESS_DEBUG_EVENTS").is_some() {
        eprintln!("mock request: {path}");
    }

    if path.ends_with("/models") {
        let body =
            r#"{"object":"list","data":[{"id":"mock-model","object":"model","owned_by":"mock"}]}"#;
        write_json(&mut stream, body);
        return;
    }

    if !path.ends_with("/chat/completions") {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        return;
    }

    let turn = {
        let mut index = cursor.lock().expect("mock cursor");
        let turn = script
            .turns
            .get(*index % script.turns.len().max(1))
            .cloned()
            .unwrap_or_default();
        *index += 1;
        turn
    };

    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }

    for chunk in &turn.chunks {
        let payload = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "delta": { "content": chunk },
                "finish_reason": null
            }]
        });
        if write_event(&mut stream, &payload).is_err() {
            return;
        }
        if turn.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(turn.delay_ms));
        }
    }

    if let Some(tool) = &turn.tool {
        let arguments = serde_json::to_string(&tool.input).unwrap_or_else(|_| "{}".into());
        let payload = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_mock",
                        "type": "function",
                        "function": { "name": tool.name, "arguments": arguments }
                    }]
                },
                "finish_reason": null
            }]
        });
        if write_event(&mut stream, &payload).is_err() {
            return;
        }
    }

    let finish = if turn.tool.is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    let payload = serde_json::json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "mock-model",
        "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }]
    });
    let _ = write_event(&mut stream, &payload);
    let _ = stream.write_all(b"data: [DONE]\n\n");
    let _ = stream.flush();
}

fn write_event(stream: &mut TcpStream, payload: &serde_json::Value) -> std::io::Result<()> {
    stream.write_all(b"data: ")?;
    stream.write_all(payload.to_string().as_bytes())?;
    stream.write_all(b"\n\n")?;
    stream.flush()
}

fn write_json(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let read = stream.read(&mut chunk).ok()?;
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
    String::from_utf8(buffer).ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Minimal client used by the mock's own tests and by node tests that need to
/// talk to the mock without pulling in an HTTP library.
pub fn http_post(port: u16, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to mock");
    let request = format!(
        "POST {path} HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("send request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read mock response");
    response
}

pub fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to mock");
    let request = format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).expect("send request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read mock response");
    response
}
