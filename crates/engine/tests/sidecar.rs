//! End-to-end sidecar lifecycle and session command test.
//!
//! Requires `opencode` on `PATH` (or `RUST_HARNESS_OPENCODE_BIN`). Set
//! `RUST_HARNESS_SKIP_E2E=1` to skip in environments without it.

use std::time::{Duration, Instant};

use async_channel::{Receiver, TryRecvError};
use harness_engine::{
    Engine, EngineConfig, EngineEvent, ExecutionOutcome, PermissionReply, StreamEvent,
    locate_binary,
};
use harness_protocol::{AssistantPart, SessionMessage};

#[test]
fn boots_reaches_ready_and_round_trips_sessions() {
    if std::env::var_os("RUST_HARNESS_SKIP_E2E").is_some() {
        eprintln!("skipping sidecar e2e test");
        return;
    }
    if locate_binary(None).is_err() {
        eprintln!("skipping sidecar e2e test: opencode was not found");
        return;
    }

    let project = std::env::temp_dir().join(format!(
        "rust-harness-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();

    let mut engine = Engine::spawn(EngineConfig::new(&project));
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(30);

    let ready = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    });
    let EngineEvent::Ready { version, port } = ready else {
        unreachable!("predicate only accepts Ready");
    };
    eprintln!("sidecar ready: {version} on 127.0.0.1:{port}");

    // The supervisor loads the project's sessions immediately after Ready.
    let loaded = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    });
    let EngineEvent::SessionsLoaded { sessions } = loaded else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    assert!(
        sessions.is_empty(),
        "new project should have no sessions, got {}",
        sessions.len()
    );

    assert!(engine.create_session(), "engine stopped before create");
    let created = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    });
    let EngineEvent::SessionCreated { session } = created else {
        unreachable!("predicate only accepts SessionCreated");
    };
    let id = session.id.clone();
    assert!(session.title.is_none());
    let canonical_project = std::fs::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        session
            .location
            .as_ref()
            .map(|location| location.directory.as_str()),
        Some(canonical_project.as_str()),
        "the created session should be pinned to the project directory"
    );

    assert!(
        engine.rename_session(&id, "M1 e2e session"),
        "engine stopped before rename"
    );
    let renamed = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionRenamed { .. })
    });
    let EngineEvent::SessionRenamed {
        id: renamed_id,
        title,
    } = renamed
    else {
        unreachable!("predicate only accepts SessionRenamed");
    };
    assert_eq!(renamed_id, id);
    assert_eq!(title, "M1 e2e session");

    // A fresh session has an empty timeline.
    assert!(
        engine.load_transcript(&id),
        "engine stopped before transcript load"
    );
    let transcript = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::TranscriptLoaded { .. })
    });
    let EngineEvent::TranscriptLoaded {
        session_id,
        messages,
    } = transcript
    else {
        unreachable!("predicate only accepts TranscriptLoaded");
    };
    assert_eq!(session_id, id);
    assert!(messages.is_empty(), "new session transcript is not empty");

    // Interrupting an idle session is a no-op, not an error.
    assert!(engine.interrupt(&id), "engine stopped before interrupt");
    let interrupted = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Interrupted { .. })
    });
    let EngineEvent::Interrupted {
        session_id,
        interrupted,
    } = interrupted
    else {
        unreachable!("predicate only accepts Interrupted");
    };
    assert_eq!(session_id, id);
    assert!(!interrupted);

    assert!(engine.delete_session(&id), "engine stopped before delete");
    let deleted = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionDeleted { .. })
    });
    let EngineEvent::SessionDeleted { id: deleted_id } = deleted else {
        unreachable!("predicate only accepts SessionDeleted");
    };
    assert_eq!(deleted_id, id);

    // Confirm the delete reached the server, not just our local mirror.
    assert!(engine.refresh_sessions(), "engine stopped before refresh");
    let reloaded = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    });
    let EngineEvent::SessionsLoaded { sessions } = reloaded else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    assert!(sessions.is_empty(), "deleted session is still listed");

    engine.shutdown();
    std::thread::sleep(Duration::from_millis(250));

    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "sidecar is still listening on 127.0.0.1:{port} after shutdown"
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Streams one assistant turn from a local OpenAI-compatible mock.
///
/// Gated on `RUST_HARNESS_MOCK_PROVIDER` (an OpenAI-style base URL such as
/// `http://127.0.0.1:47899/v1`) because it needs that server running.
#[test]
fn streams_assistant_text_from_a_mock_provider() {
    let Some(base_url) = std::env::var("RUST_HARNESS_MOCK_PROVIDER").ok() else {
        eprintln!("skipping streaming e2e test: RUST_HARNESS_MOCK_PROVIDER is not set");
        return;
    };
    if locate_binary(None).is_err() {
        eprintln!("skipping streaming e2e test: opencode was not found");
        return;
    }

    let project = std::env::temp_dir().join(format!(
        "rust-harness-stream-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("opencode.json"),
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
        ),
    )
    .unwrap();

    let mut engine = Engine::spawn(EngineConfig::new(&project));
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(60);

    wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    });
    // Prompting before the SSE stream is open would race the deltas.
    wait_for_startup(&events, deadline);

    assert!(engine.create_session(), "engine stopped before create");
    let created = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    });
    let EngineEvent::SessionCreated { session } = created else {
        unreachable!("predicate only accepts SessionCreated");
    };
    let id = session.id.clone();

    assert!(
        engine.send_prompt(&id, "hi"),
        "engine stopped before prompt"
    );
    let accepted = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::PromptAccepted { .. })
    });
    let EngineEvent::PromptAccepted {
        session_id,
        message,
    } = accepted
    else {
        unreachable!("predicate only accepts PromptAccepted");
    };
    assert_eq!(session_id, id);
    assert_eq!(message.payload.text, "hi");

    let delta = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Stream(StreamEvent::TextDelta { .. }))
    });
    let EngineEvent::Stream(StreamEvent::TextDelta { delta, .. }) = delta else {
        unreachable!("predicate only accepts TextDelta");
    };
    assert_eq!(delta, "Hello from the mock");

    let finished = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::ExecutionFinished { outcome, .. }) = finished else {
        unreachable!("predicate only accepts ExecutionFinished");
    };
    assert_eq!(outcome, ExecutionOutcome::Succeeded);

    // The turn is durable: the same text comes back from the timeline.
    assert!(
        engine.load_transcript(&id),
        "engine stopped before transcript load"
    );
    let transcript = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::TranscriptLoaded { .. })
    });
    let EngineEvent::TranscriptLoaded { messages, .. } = transcript else {
        unreachable!("predicate only accepts TranscriptLoaded");
    };
    let assistant = messages
        .iter()
        .find_map(|message| match message {
            SessionMessage::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .expect("the turn should have produced an assistant message");
    assert!(matches!(
        &assistant[0],
        AssistantPart::Text { text } if text == "Hello from the mock"
    ));

    engine.shutdown();
    let _ = std::fs::remove_dir_all(&project);
}

/// Streams reasoning and a tool call from a local OpenAI-compatible mock.
///
/// Gated on `RUST_HARNESS_MOCK_TOOL_PROVIDER` (an OpenAI-style base URL such as
/// `http://127.0.0.1:47899/v1`) because it needs that server running. The mock
/// must stream a reasoning delta, call a tool, and then answer; the prompt is
/// `read <project>/hello.txt`, so the tool should be able to read that file.
#[test]
fn streams_reasoning_and_tool_calls_from_a_mock_provider() {
    let Some(base_url) = std::env::var("RUST_HARNESS_MOCK_TOOL_PROVIDER").ok() else {
        eprintln!("skipping tool e2e test: RUST_HARNESS_MOCK_TOOL_PROVIDER is not set");
        return;
    };
    if locate_binary(None).is_err() {
        eprintln!("skipping tool e2e test: opencode was not found");
        return;
    }

    let project = std::env::temp_dir().join(format!(
        "rust-harness-tools-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();
    let target = project.join("hello.txt");
    std::fs::write(&target, "hello from the mock project").unwrap();
    std::fs::write(
        project.join("opencode.json"),
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
        ),
    )
    .unwrap();

    let mut engine = Engine::spawn(EngineConfig::new(&project));
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(90);

    wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    });
    wait_for_startup(&events, deadline);

    assert!(engine.create_session(), "engine stopped before create");
    let created = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    });
    let EngineEvent::SessionCreated { session } = created else {
        unreachable!("predicate only accepts SessionCreated");
    };
    let id = session.id.clone();

    assert!(
        engine.send_prompt(&id, format!("read {}", target.display())),
        "engine stopped before prompt"
    );

    // Reasoning streams before the tool call.
    let reasoning = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ReasoningDelta { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::ReasoningDelta { delta, .. }) = reasoning else {
        unreachable!("predicate only accepts ReasoningDelta");
    };
    assert!(!delta.is_empty(), "reasoning delta should carry text");

    let started = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ToolInputStarted { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::ToolInputStarted { tool_id, name, .. }) = started else {
        unreachable!("predicate only accepts ToolInputStarted");
    };
    assert!(!name.is_empty(), "a tool call should have a name");

    let called = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Stream(StreamEvent::ToolCalled { .. }))
    });
    let EngineEvent::Stream(StreamEvent::ToolCalled {
        tool_id: called_id,
        input,
        ..
    }) = called
    else {
        unreachable!("predicate only accepts ToolCalled");
    };
    assert_eq!(called_id, tool_id);
    assert!(!input.is_empty(), "the tool call should carry its input");

    let succeeded = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ToolSucceeded { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::ToolSucceeded {
        tool_id: succeeded_id,
        content,
        ..
    }) = succeeded
    else {
        unreachable!("predicate only accepts ToolSucceeded");
    };
    assert_eq!(succeeded_id, tool_id);
    assert!(!content.is_empty(), "a successful tool should have content");

    // The answer after the tool result still streams as text.
    let text = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Stream(StreamEvent::TextDelta { .. }))
    });
    let EngineEvent::Stream(StreamEvent::TextDelta { delta, .. }) = text else {
        unreachable!("predicate only accepts TextDelta");
    };
    assert!(!delta.is_empty());

    let finished = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::ExecutionFinished { outcome, .. }) = finished else {
        unreachable!("predicate only accepts ExecutionFinished");
    };
    assert_eq!(outcome, ExecutionOutcome::Succeeded);

    // The snapshot preserves the reasoning and the finished tool state.
    assert!(
        engine.load_transcript(&id),
        "engine stopped before transcript load"
    );
    let transcript = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::TranscriptLoaded { .. })
    });
    let EngineEvent::TranscriptLoaded { messages, .. } = transcript else {
        unreachable!("predicate only accepts TranscriptLoaded");
    };
    let parts: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            SessionMessage::Assistant { content, .. } => Some(content.as_slice()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        parts.iter().any(|part| matches!(
            part,
            AssistantPart::Reasoning { text } if !text.is_empty()
        )),
        "the transcript should keep the reasoning part"
    );
    assert!(
        parts.iter().any(|part| matches!(
            part,
            AssistantPart::Tool {
                name: part_name,
                state: harness_protocol::ToolState::Completed { .. },
                ..
            } if part_name == &name
        )),
        "the transcript should keep the completed tool call"
    );

    engine.shutdown();
    let _ = std::fs::remove_dir_all(&project);
}

/// Asks for permission, allows once, and watches the turn complete.
///
/// Gated on `RUST_HARNESS_MOCK_PERMISSION_PROVIDER` (an OpenAI-style base URL
/// such as `http://127.0.0.1:47942/v1`). The mock must call the `bash` tool.
#[test]
fn asks_for_permission_and_unblocks_after_reply() {
    let Some(base_url) = std::env::var("RUST_HARNESS_MOCK_PERMISSION_PROVIDER").ok() else {
        eprintln!("skipping permission e2e test: RUST_HARNESS_MOCK_PERMISSION_PROVIDER is not set");
        return;
    };
    if locate_binary(None).is_err() {
        eprintln!("skipping permission e2e test: opencode was not found");
        return;
    }

    let project = std::env::temp_dir().join(format!(
        "rust-harness-permission-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("opencode.json"), permission_config(&base_url)).unwrap();

    let mut engine = Engine::spawn(EngineConfig::new(&project));
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(90);

    wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    });
    wait_for_startup(&events, deadline);

    assert!(engine.create_session(), "engine stopped before create");
    let created = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    });
    let EngineEvent::SessionCreated { session } = created else {
        unreachable!("predicate only accepts SessionCreated");
    };
    let id = session.id.clone();

    assert!(
        engine.send_prompt(&id, "please run the command"),
        "engine stopped before prompt"
    );

    let asked = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::PermissionAsked { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::PermissionAsked { request }) = asked else {
        unreachable!("predicate only accepts PermissionAsked");
    };
    assert_eq!(request.session_id, id);
    assert!(!request.id.is_empty());
    assert!(
        request.action == "bash" || request.action == "shell",
        "unexpected permission action: {}",
        request.action
    );
    assert!(!request.resources.is_empty());

    // The pending list agrees with the live event.
    assert!(
        engine.load_permissions(&id),
        "engine stopped before permission load"
    );
    let listed = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::PermissionsLoaded { .. })
    });
    let EngineEvent::PermissionsLoaded {
        session_id,
        requests,
    } = listed
    else {
        unreachable!("predicate only accepts PermissionsLoaded");
    };
    assert_eq!(session_id, id);
    assert!(
        requests.iter().any(|pending| pending.id == request.id),
        "the pending permission should be listed"
    );

    assert!(
        engine.reply_permission(&id, &request.id, PermissionReply::Once),
        "engine stopped before permission reply"
    );

    // One loop, because wait_for discards events it does not match.
    let mut saw_replied = false;
    let mut saw_success = false;
    let outcome = loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for the permission turn to finish");
        }
        match events.try_recv() {
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(event) => match event {
                EngineEvent::Stream(StreamEvent::PermissionReplied { .. })
                | EngineEvent::PermissionReplied { .. } => saw_replied = true,
                EngineEvent::Stream(StreamEvent::ToolSucceeded { .. }) => saw_success = true,
                EngineEvent::Stream(StreamEvent::ExecutionFinished { outcome, .. }) => {
                    break outcome;
                }
                _ => {}
            },
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(25)),
            Err(TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    };
    assert!(saw_replied, "the reply should be confirmed by the server");
    assert!(saw_success, "the allowed tool should run and succeed");
    assert_eq!(outcome, ExecutionOutcome::Succeeded);

    // Nothing is left waiting once the turn ends.
    assert!(
        engine.load_permissions(&id),
        "engine stopped before permission reload"
    );
    let reloaded = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::PermissionsLoaded { .. })
    });
    let EngineEvent::PermissionsLoaded { requests, .. } = reloaded else {
        unreachable!("predicate only accepts PermissionsLoaded");
    };
    assert!(
        requests.is_empty(),
        "the answered permission should no longer be pending"
    );

    engine.shutdown();
    let _ = std::fs::remove_dir_all(&project);
}

/// Declining a permission fails the tool and interrupts the turn.
///
/// Gated on `RUST_HARNESS_MOCK_PERMISSION_PROVIDER`.
#[test]
fn declining_permission_fails_the_tool_and_interrupts_the_turn() {
    let Some(base_url) = std::env::var("RUST_HARNESS_MOCK_PERMISSION_PROVIDER").ok() else {
        eprintln!("skipping permission reject e2e test: env var is not set");
        return;
    };
    if locate_binary(None).is_err() {
        eprintln!("skipping permission reject e2e test: opencode was not found");
        return;
    }

    let project = std::env::temp_dir().join(format!(
        "rust-harness-reject-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("opencode.json"), permission_config(&base_url)).unwrap();

    let mut engine = Engine::spawn(EngineConfig::new(&project));
    let events = engine.events();
    let deadline = Instant::now() + Duration::from_secs(90);

    wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::Ready { .. })
    });
    wait_for_startup(&events, deadline);

    assert!(engine.create_session(), "engine stopped before create");
    let created = wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    });
    let EngineEvent::SessionCreated { session } = created else {
        unreachable!("predicate only accepts SessionCreated");
    };
    let id = session.id.clone();

    assert!(
        engine.send_prompt(&id, "please run the command"),
        "engine stopped before prompt"
    );

    let asked = wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::PermissionAsked { .. })
        )
    });
    let EngineEvent::Stream(StreamEvent::PermissionAsked { request }) = asked else {
        unreachable!("predicate only accepts PermissionAsked");
    };

    assert!(
        engine.reply_permission(&id, &request.id, PermissionReply::Reject),
        "engine stopped before permission reply"
    );

    let mut saw_declined = false;
    let outcome = loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for the rejected turn to finish");
        }
        match events.try_recv() {
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(event) => match event {
                EngineEvent::Stream(StreamEvent::ToolFailed { error, .. }) => {
                    saw_declined = error.message.contains("declined");
                }
                EngineEvent::Stream(StreamEvent::ExecutionFinished { outcome, .. }) => {
                    break outcome;
                }
                _ => {}
            },
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(25)),
            Err(TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    };
    assert!(
        saw_declined,
        "the declined tool should fail with its message"
    );
    assert_eq!(outcome, ExecutionOutcome::Interrupted);

    engine.shutdown();
    let _ = std::fs::remove_dir_all(&project);
}

/// A project config that gates shell tools behind an explicit answer.
fn permission_config(base_url: &str) -> String {
    format!(
        r#"{{
            "$schema": "https://opencode.ai/config.json",
            "model": "mock/mock-model",
            "permission": {{ "bash": "ask", "shell": "ask" }},
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

/// Wait until the initial session snapshot has arrived and the event stream is
/// open. The two are emitted concurrently, so neither order can be assumed.
fn wait_for_startup(events: &Receiver<EngineEvent>, deadline: Instant) {
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
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(25)),
            Err(TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    }
}

fn wait_for(
    events: &Receiver<EngineEvent>,
    deadline: Instant,
    mut matches: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for an engine event");
        }
        match events.try_recv() {
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(event) => {
                if matches(&event) {
                    return event;
                }
            }
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(25)),
            Err(TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    }
}
