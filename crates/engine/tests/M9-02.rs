//! M9-02 — fork a session before a message.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use harness_protocol::{PromptRequest, SessionMessage};
use support::mock::{MockProvider, Script, Turn};

fn scripted() -> Script {
    Script {
        turns: vec![
            Turn {
                chunks: vec!["ok".into()],
                ..Turn::default()
            };
            16
        ],
    }
}

#[test]
fn forks_before_the_second_user_message() {
    if support::skip_e2e() {
        eprintln!("skipping fork e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping fork e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(scripted());
    let project = support::project("fork");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(90);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session());
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let original = session.id;

    for text in ["first", "second"] {
        assert!(started.engine.send_prompt(
            &original,
            PromptRequest {
                text: text.into(),
                ..Default::default()
            }
        ));
        support::wait_for(&events, deadline, |event| {
            matches!(
                event,
                EngineEvent::Stream(harness_engine::StreamEvent::ExecutionFinished { .. })
            )
        });
    }

    // Both user messages must be on the server before forking.
    let mut messages = Vec::new();
    for _ in 0..20 {
        assert!(started.engine.load_transcript(&original));
        let EngineEvent::TranscriptLoaded {
            messages: loaded, ..
        } = support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::TranscriptLoaded { .. })
        })
        else {
            unreachable!()
        };
        messages = loaded;
        let users = messages
            .iter()
            .filter(|message| matches!(message, SessionMessage::User { .. }))
            .count();
        if users >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    let second = messages
        .iter()
        .find_map(|message| match message {
            SessionMessage::User { id, text, .. } if text == "second" => Some(id.clone()),
            _ => None,
        })
        .expect("second user message");

    assert!(started.engine.fork_session(&original, Some(second)));
    let EngineEvent::SessionForked { session: forked } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::SessionForked { .. })
        })
    else {
        unreachable!()
    };
    assert_ne!(forked.id, original, "fork returned the original session");

    assert!(started.engine.load_transcript(&forked.id));
    let EngineEvent::TranscriptLoaded { messages, .. } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::TranscriptLoaded { .. })
        })
    else {
        unreachable!()
    };
    let users: Vec<&str> = messages
        .iter()
        .filter_map(|message| match message {
            SessionMessage::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(users, ["first"], "fork boundary is off: {messages:?}");

    support::shutdown(started.engine, started.port);
}
