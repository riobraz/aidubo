//! M9-04 — compact a session and keep using it.

mod support;

use std::time::{Duration, Instant};

use harness_engine::{EngineEvent, StreamEvent};
use harness_protocol::PromptRequest;
use support::mock::{MockProvider, Script, Turn};

fn scripted() -> Script {
    Script {
        turns: vec![
            Turn {
                chunks: vec!["ok".into()],
                ..Turn::default()
            };
            32
        ],
    }
}

#[test]
fn compacts_and_the_session_stays_usable() {
    if support::skip_e2e() {
        eprintln!("skipping compaction e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping compaction e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(scripted());
    let project = support::project("compact");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(120);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session(&started.directory,));
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let id = session.id;

    assert!(started.engine.send_prompt(
        &started.directory,
        &id,
        PromptRequest {
            text: "first".into(),
            ..Default::default()
        }
    ));
    support::wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
        )
    });

    assert!(started.engine.compact(&started.directory, &id));
    support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::CompactionRequested { .. })
    });

    support::wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::CompactionEnded { .. })
                | EngineEvent::Stream(StreamEvent::CompactionFailed { .. })
        )
    });

    assert!(started.engine.send_prompt(
        &started.directory,
        &id,
        PromptRequest {
            text: "after".into(),
            ..Default::default()
        }
    ));
    support::wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
        )
    });

    support::shutdown(started.engine, started.port);
}
