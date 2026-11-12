//! M9-01 — a prompt queued behind a running turn can be listed and cancelled.

mod support;

use std::time::{Duration, Instant};

use harness_engine::{EngineEvent, StreamEvent};
use harness_protocol::{InboxDelivery, PromptRequest};
use support::mock::{MockProvider, Script, Turn};

fn slow_script() -> Script {
    Script {
        turns: (0..16)
            .map(|_| Turn {
                chunks: vec!["a".into(), "b".into(), "c".into(), "d".into()],
                delay_ms: 120,
                tool: None,
            })
            .collect(),
    }
}

#[test]
fn lists_and_cancels_a_queued_prompt() {
    if support::skip_e2e() {
        eprintln!("skipping inbox e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping inbox e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(slow_script());
    let project = support::project("inbox");
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
            EngineEvent::Stream(StreamEvent::ExecutionStarted { .. })
        )
    });

    assert!(started.engine.send_prompt(
        &started.directory,
        &id,
        PromptRequest {
            text: "second".into(),
            delivery: Some(InboxDelivery::Queue),
            ..Default::default()
        }
    ));

    let mut queued = Vec::new();
    for _ in 0..30 {
        assert!(started.engine.load_inbox(&started.directory, &id));
        let EngineEvent::InboxLoaded { items, .. } =
            support::wait_for(&events, deadline, |event| {
                matches!(event, EngineEvent::InboxLoaded { .. })
            })
        else {
            unreachable!()
        };
        let found = items
            .iter()
            .find(|item| item.text() == "second")
            .map(|item| item.delivery);
        if let Some(delivery) = found {
            assert_eq!(delivery, Some(InboxDelivery::Queue));
            queued = items;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!queued.is_empty(), "the queued prompt never appeared");

    let inbox_id = queued
        .iter()
        .find(|item| item.text() == "second")
        .expect("queued item")
        .id
        .clone();
    assert!(
        started
            .engine
            .cancel_inbox(&started.directory, &id, inbox_id)
    );
    let EngineEvent::InboxLoaded { items, .. } = support::wait_for(
        &events,
        deadline,
        |event| matches!(event, EngineEvent::InboxLoaded { items, .. } if items.iter().all(|item| item.text() != "second")),
    ) else {
        unreachable!()
    };
    assert!(items.iter().all(|item| item.text() != "second"));

    support::shutdown(started.engine, started.port);
}
