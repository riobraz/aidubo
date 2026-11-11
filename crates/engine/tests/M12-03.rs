//! M12-03 — a finished turn becomes exactly one notification, even when the
//! stream replays the frame after a reconnect.
//!
//! OS delivery is a manual check; this test exercises the live event payload,
//! the core policy, and the JSON sink contract.

mod support;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use harness_core::{Notification, NotifyContext, NotifyPolicy};
use harness_engine::{EngineEvent, StreamEvent};
use harness_protocol::PromptRequest;
use support::mock::{MockProvider, Script};

fn append_line(path: &PathBuf, notification: &Notification) {
    let line = serde_json::to_string(notification).expect("encode notification");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open notification sink");
    writeln!(file, "{line}").expect("write notification sink");
}

#[test]
fn turn_done_notifies_once_across_replay() {
    if support::skip_e2e() {
        eprintln!("skipping notification e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping notification e2e test: opencode was not found");
        return;
    }
    let Some(sink) = std::env::var_os("RUST_HARNESS_NOTIFY_SINK").map(PathBuf::from) else {
        eprintln!("skipping notification e2e test: RUST_HARNESS_NOTIFY_SINK is not set");
        return;
    };
    std::fs::write(&sink, "").expect("truncate notification sink");

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("notify");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session());
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let session_id = session.id.clone();

    assert!(
        started
            .engine
            .rename_session(session_id.clone(), "Notification test")
    );
    let EngineEvent::SessionRenamed { title, .. } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionRenamed { .. })
    }) else {
        unreachable!()
    };

    assert!(started.engine.send_prompt(
        session_id.clone(),
        PromptRequest {
            text: "say ok".into(),
            ..PromptRequest::default()
        },
    ));
    let EngineEvent::Stream(finished @ StreamEvent::ExecutionFinished { .. }) =
        support::wait_for(&events, deadline, |event| {
            matches!(
                event,
                EngineEvent::Stream(StreamEvent::ExecutionFinished { .. })
            )
        })
    else {
        unreachable!()
    };

    let context = NotifyContext {
        session_id: &session_id,
        session_title: &title,
        focused: false,
        active_session: false,
    };
    let mut policy = NotifyPolicy::default();
    let now = Instant::now();
    let notification = policy
        .on_stream_event(&finished, context, now)
        .expect("the finished turn notifies once");
    append_line(&sink, &notification);

    // A reconnect replays the same server frame.
    let replay = policy.on_stream_event(&finished, context, now + Duration::from_secs(3));
    assert!(replay.is_none(), "a replayed frame must not notify again");

    let raw = std::fs::read_to_string(&sink).expect("read notification sink");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one notification was written");
    let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("json line");
    assert_eq!(parsed["kind"], "turn_done");
    assert_eq!(parsed["session_title"], "Notification test");
    assert_eq!(parsed["body"], "Turn finished");
    assert!(
        !parsed["body"]
            .as_str()
            .is_some_and(|body| body.contains("say ok")),
        "the prompt must not leak into the default notification body"
    );

    support::shutdown(started.engine, started.port);
}
