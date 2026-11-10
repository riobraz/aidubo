//! Shared support for the sidecar end-to-end tests.
//!
//! Every e2e file declares `mod support;` and uses these helpers so a test only
//! describes its own scenario.
#![allow(dead_code)]

pub mod mock;

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_channel::Receiver;
use harness_engine::{Engine, EngineConfig, EngineEvent, locate_binary};

/// True when the environment opted out of every sidecar test.
pub fn skip_e2e() -> bool {
    std::env::var_os("RUST_HARNESS_SKIP_E2E").is_some()
}

/// True when an `opencode` binary can be located.
pub fn opencode_available() -> bool {
    locate_binary(None).is_ok()
}

/// A unique temporary project directory, removed on drop.
pub struct TestProject {
    path: PathBuf,
}

impl TestProject {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.path.join(name), contents).expect("write project file");
    }

    /// Register an OpenAI-compatible mock provider as the project's model.
    pub fn use_mock_provider(&self, base_url: &str) {
        self.write("opencode.json", &provider_config(base_url));
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl Deref for TestProject {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestProject {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Create a unique temp project. `tag` only makes failures readable.
pub fn project(tag: &str) -> TestProject {
    let path = std::env::temp_dir().join(format!(
        "rust-harness-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&path).expect("create temp project");
    TestProject { path }
}

/// The `opencode.json` that points a project at an OpenAI-compatible mock.
pub fn provider_config(base_url: &str) -> String {
    format!(
        r#"{{
            "$schema": "https://opencode.ai/config.json",
            "model": "mock/mock-model",
            "provider": {{
                "mock": {{
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "Mock",
                    "options": {{ "baseURL": "{base_url}", "apiKey": "test-key" }},
                    "models": {{ "mock-model": {{ "name": "Mock Model" }} }}
                }}
            }}
        }}"#
    )
}

/// A started engine plus its event channel and port.
pub struct Started {
    pub engine: Engine,
    pub events: Receiver<EngineEvent>,
    pub port: u16,
}

/// Spawn the engine and wait until it answers an authenticated request.
pub fn start_engine(project: &TestProject) -> Started {
    start_engine_with(EngineConfig::new(project.path()))
}

pub fn start_engine_with(config: EngineConfig) -> Started {
    let engine = Engine::spawn(config);
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(30);
    let EngineEvent::Ready { port, .. } = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    }) else {
        unreachable!("predicate only accepts Ready");
    };
    Started {
        engine,
        events,
        port,
    }
}

/// Wait until both the session snapshot and the live stream are up.
pub fn wait_for_stream(events: &Receiver<EngineEvent>, deadline: Instant) {
    let mut stream = false;
    let mut sessions = false;
    while !(stream && sessions) {
        if Instant::now() >= deadline {
            panic!("timed out waiting for the startup events");
        }
        match events.try_recv() {
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(EngineEvent::StreamConnected) => stream = true,
            Ok(EngineEvent::SessionsLoaded { .. }) => sessions = true,
            Ok(_) => {}
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(async_channel::TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    }
}

/// Wait for one event matching `predicate`, panicking on failure or timeout.
pub fn wait_for(
    events: &Receiver<EngineEvent>,
    deadline: Instant,
    mut predicate: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for an engine event");
        }
        match events.try_recv() {
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(event) => {
                if std::env::var_os("RUST_HARNESS_DEBUG_EVENTS").is_some() {
                    eprintln!("event: {event:?}");
                }
                if predicate(&event) {
                    return event;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(async_channel::TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    }
}

/// Shut the engine down and assert nothing is left listening on its port.
pub fn shutdown(mut engine: Engine, port: u16) {
    engine.shutdown();
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "sidecar is still listening on 127.0.0.1:{port} after shutdown"
    );
}
