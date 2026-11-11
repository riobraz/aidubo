//! M16-02 — signed update check.
//!
//! Everything runs against a loopback HTTP server: serve a manifest signed
//! with a test key, assert it is offered; flip one byte and assert the
//! signature fails; stop the server and assert the check degrades to
//! `Unavailable`. The download path verifies size and SHA-256 before saving.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer as _, SigningKey};
use harness_protocol::{UpdateArtifact, UpdateManifest};
use rust_harness::update::{self, Trust, UpdateState};
use serde_json::json;

const KEY_SEED: [u8; 32] = [9u8; 32];

struct TestServer {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        listener.set_nonblocking(true).expect("nonblocking");
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));

        let thread_routes = Arc::clone(&routes);
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("update-test-server".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => respond(stream, &thread_routes),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("start test server");

        Self {
            port,
            routes,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn set(&self, path: &str, body: Vec<u8>) {
        self.routes
            .lock()
            .expect("routes")
            .insert(path.to_string(), body);
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("stop test server");
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn respond(mut stream: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) {
    // macOS may hand an accepted socket the listener's nonblocking flag; the
    // request read below must wait for bytes.
    stream.set_nonblocking(false).expect("blocking stream");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
            Err(_) => return,
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);

    let (status, body) = match routes.lock().expect("routes").get(path).cloned() {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", Vec::new()),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let written = stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .and_then(|_| stream.flush());
    if let Err(error) = written {
        eprintln!("test server response failed: {error}");
    }
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&KEY_SEED)
}

fn trust() -> Trust {
    Trust::from_bytes(signing_key().verifying_key().to_bytes()).expect("trust")
}

fn signed_manifest(version: &str, artifact_url: &str, artifact: &[u8]) -> Vec<u8> {
    let mut value = json!({
        "version": version,
        "channel": "stable",
        "artifacts": [{
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "url": artifact_url,
            "sha256": update::sha256_hex(artifact),
            "size": artifact.len(),
        }],
        "notes_url": "https://example.test/notes",
        "issued_at": 1_762_732_800,
    });
    let mut manifest: UpdateManifest =
        serde_json::from_value(value.clone()).expect("manifest struct");
    manifest.signature.clear();
    let payload = manifest.signature_payload().expect("payload");
    value["signature"] = json!(STANDARD.encode(signing_key().sign(&payload).to_bytes()));
    serde_json::to_vec(&value).expect("manifest json")
}

fn unique_dir() -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust-harness-update-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn signed_update_check_rejects_tampering_and_download_verifies() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    let mut server = TestServer::start();
    let artifact = b"release bytes".to_vec();
    server.set("/app.bin", artifact.clone());
    let artifact_url = server.url("/app.bin");
    let manifest_url = server.url("/manifest.json");
    let manifest = signed_manifest("0.2.0", &artifact_url, &artifact);
    server.set("/manifest.json", manifest.clone());

    let state = runtime.block_on(update::check(&manifest_url, "0.1.0", &trust()));
    let state_debug = format!("{state:?}");
    let UpdateState::Available {
        version,
        url,
        sha256,
        ..
    } = state
    else {
        panic!("a correctly signed newer manifest must be available: {state_debug}");
    };
    assert_eq!(version, "0.2.0");
    assert_eq!(url, artifact_url);
    assert_eq!(sha256, update::sha256_hex(&artifact));

    // The same manifest on an equal or older current version is up to date.
    let state = runtime.block_on(update::check(&manifest_url, "0.2.0", &trust()));
    let state_debug = format!("{state:?}");
    assert!(
        matches!(state, UpdateState::UpToDate { .. }),
        "{state_debug}"
    );

    // Download verifies size and digest, then writes the file.
    let dir = unique_dir();
    let descriptor = UpdateArtifact {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        url: artifact_url.clone(),
        sha256: update::sha256_hex(&artifact),
        size: artifact.len() as u64,
        extra: Default::default(),
    };
    let saved = runtime
        .block_on(update::download_verified(&descriptor, &dir))
        .expect("verified download");
    assert_eq!(std::fs::read(&saved).expect("read download"), artifact);

    // A different body at the same URL is refused and nothing is written.
    server.set("/app.bin", b"different bytes".to_vec());
    let error = runtime
        .block_on(update::download_verified(&descriptor, &dir))
        .expect_err("corrupt download must be refused");
    assert!(!error.is_empty());
    server.set("/app.bin", artifact.clone());

    // Flip one byte inside the signed version: the signature no longer holds.
    let mut tampered = manifest.clone();
    let position = tampered
        .windows(5)
        .position(|window| window == b"0.2.0")
        .expect("version in manifest");
    tampered[position] = b'3';
    server.set("/manifest.json", tampered);
    let state = runtime.block_on(update::check(&manifest_url, "0.1.0", &trust()));
    let state_debug = format!("{state:?}");
    let UpdateState::Unavailable { reason, .. } = state else {
        panic!("a tampered manifest must be unavailable: {state_debug}");
    };
    assert!(reason.contains("signature"), "{reason}");

    // A manifest signed by another key is refused too.
    let other_key = SigningKey::from_bytes(&[10u8; 32]);
    let other = Trust::from_bytes(other_key.verifying_key().to_bytes()).expect("other trust");
    server.set("/manifest.json", manifest.clone());
    let state = runtime.block_on(update::check(&manifest_url, "0.1.0", &other));
    assert!(
        matches!(state, UpdateState::Unavailable { .. }),
        "{state:?}"
    );

    // Offline is quiet: no listener means an Unavailable state, not a panic.
    server.stop();
    let state = runtime.block_on(update::check(&manifest_url, "0.1.0", &trust()));
    let state_debug = format!("{state:?}");
    let UpdateState::Unavailable { reason, checked_at } = state else {
        panic!("a stopped server must be unavailable: {state_debug}");
    };
    assert!(reason.contains("could not be reached"), "{reason}");
    assert!(checked_at > 0);

    std::fs::remove_dir_all(&dir).expect("clean up downloads");
}
