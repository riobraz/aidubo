//! M9-03 — stage a revert, commit it, and the transcript rolls back.

mod support;

use std::time::{Duration, Instant};

use harness_engine::{EngineEvent, StreamEvent};
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

fn users(messages: &[SessionMessage]) -> Vec<&str> {
    messages
        .iter()
        .filter_map(|message| match message {
            SessionMessage::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn reverts_a_session_to_a_message() {
    if support::skip_e2e() {
        eprintln!("skipping revert e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping revert e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(scripted());
    let project = support::project("revert");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(90);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session(&started.directory,));
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let id = session.id;

    for text in ["first", "second"] {
        assert!(started.engine.send_prompt(
            &started.directory,
            &id,
            PromptRequest {
                text: text.into(),
                ..Default::default()
            }
        ));
        support::wait_for(&events, deadline, |event| {
            matches!(
                event,
                EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
            )
        });
    }

    let mut messages = Vec::new();
    for _ in 0..20 {
        assert!(started.engine.load_transcript(&started.directory, &id));
        let EngineEvent::TranscriptLoaded {
            messages: loaded, ..
        } = support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::TranscriptLoaded { .. })
        })
        else {
            unreachable!()
        };
        messages = loaded;
        if users(&messages).len() >= 2 {
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

    assert!(
        started
            .engine
            .stage_revert(&started.directory, &id, second, Some(false))
    );
    let EngineEvent::RevertStaged { revert, .. } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::RevertStaged { .. })
    }) else {
        unreachable!()
    };
    assert!(revert.files.is_empty(), "files:false staged files");
    assert!(!revert.message_id.is_empty());

    assert!(started.engine.commit_revert(&started.directory, &id));
    support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::RevertCommitted { .. })
    });

    let mut after = Vec::new();
    for _ in 0..20 {
        assert!(started.engine.load_transcript(&started.directory, &id));
        let EngineEvent::TranscriptLoaded {
            messages: loaded, ..
        } = support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::TranscriptLoaded { .. })
        })
        else {
            unreachable!()
        };
        after = users(&loaded).into_iter().map(str::to_string).collect();
        if after == ["first"] {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(after, ["first"], "revert did not roll back the transcript");

    support::shutdown(started.engine, started.port);
}
