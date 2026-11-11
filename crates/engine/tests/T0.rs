//! T0 — shared e2e harness and scripted mock provider.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use harness_protocol::PromptRequest;
use support::mock::{MockProvider, Script, ToolCall, Turn, http_get, http_post};

#[test]
fn mock_serves_scripted_turns_in_order() {
    let script = Script {
        turns: vec![
            Turn {
                chunks: vec!["first".into()],
                ..Turn::default()
            },
            Turn {
                chunks: vec!["second".into()],
                ..Turn::default()
            },
        ],
    };
    let mock = MockProvider::start(script);

    let first = http_post(mock.port(), "/v1/chat/completions", "{}");
    assert!(first.contains("first"), "{first}");
    let second = http_post(mock.port(), "/v1/chat/completions", "{}");
    assert!(second.contains("second"), "{second}");
}

#[test]
fn mock_emits_tool_call_chunks() {
    let script = Script {
        turns: vec![Turn {
            chunks: vec![],
            tool: Some(ToolCall {
                name: "bash".into(),
                input: serde_json::json!({ "command": "true" }),
            }),
            ..Turn::default()
        }],
    };
    let mock = MockProvider::start(script);
    let body = http_post(mock.port(), "/v1/chat/completions", "{}");
    assert!(body.contains("tool_calls"), "{body}");
    assert!(body.contains("\"finish_reason\":\"tool_calls\""), "{body}");
}

#[test]
fn mock_lists_models() {
    let mock = MockProvider::start(Script::single("unused"));
    let body = http_get(mock.port(), "/v1/models");
    assert!(body.contains("mock-model"), "{body}");
}

#[test]
fn mock_stops_and_releases_port() {
    let mock = MockProvider::start(Script::single("unused"));
    let port = mock.port();
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    drop(mock);
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "mock still listening on {port}"
    );
}

#[test]
fn temp_project_is_unique_and_removed() {
    let first = support::project("t0");
    let second = support::project("t0");
    assert_ne!(first.path(), second.path());
    assert!(first.path().exists());
    let path = first.path().to_path_buf();
    drop(first);
    assert!(!path.exists(), "temp project was not removed");
}

/// Boots a real sidecar against the in-process mock provider and streams one
/// turn. Gated so a machine without `opencode` skips cleanly.
#[test]
fn engine_streams_from_the_scripted_provider() {
    if support::skip_e2e() {
        eprintln!("skipping scripted provider e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping scripted provider e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("hello from the scripted mock"));
    let project = support::project("scripted");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(
        started.engine.create_session(&started.directory,),
        "engine stopped before create"
    );
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!("predicate only accepts SessionCreated");
    };

    assert!(
        started.engine.send_prompt(
            &started.directory,
            &session.id,
            PromptRequest {
                text: "hi".into(),
                ..Default::default()
            }
        ),
        "engine stopped before prompt"
    );

    let EngineEvent::Stream(harness_engine::StreamEvent::TextEnded { text, .. }) =
        support::wait_for(&events, deadline, |event| {
            matches!(
                event,
                EngineEvent::Stream(harness_engine::StreamEvent::TextEnded { .. })
            )
        })
    else {
        unreachable!("predicate only accepts TextEnded");
    };
    assert!(text.contains("hello"), "unexpected text: {text:?}");

    support::shutdown(started.engine, started.port);
}
