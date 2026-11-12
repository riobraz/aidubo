//! M12-01 — a goal loops through an auditor session until the objective is met.
//!
//! Gated on `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`, matching the other model
//! tests. The mock is started by the test, so the variable only signals that
//! this environment can run a model-backed e2e.

mod support;

use std::time::{Duration, Instant};

use async_channel::Receiver;
use harness_core::{GoalLimits, Stopped};
use harness_engine::{Engine, EngineConfig, EngineEvent, ModelRef};
use harness_protocol::{AssistantPart, SessionMessage};
use support::mock::{MockProvider, Script, Turn};

fn turn(text: &str) -> Turn {
    Turn {
        chunks: vec![text.into()],
        ..Turn::default()
    }
}

fn transcript(
    engine: &Engine,
    events: &Receiver<EngineEvent>,
    directory: &str,
    session_id: &str,
    deadline: Instant,
) -> Vec<SessionMessage> {
    assert!(engine.load_transcript(directory, session_id));
    let EngineEvent::TranscriptLoaded { messages, .. } =
        support::wait_for(events, deadline, |event| {
            matches!(
                event,
                EngineEvent::TranscriptLoaded { session_id: loaded, .. } if loaded == session_id
            )
        })
    else {
        unreachable!("predicate only accepts TranscriptLoaded")
    };
    messages
}

fn assistant_texts(messages: &[SessionMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| {
            let SessionMessage::Assistant { content, .. } = message else {
                return None;
            };
            let text = content
                .iter()
                .filter_map(|part| match part {
                    AssistantPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            Some(text)
        })
        .collect()
}

fn count_users(messages: &[SessionMessage]) -> usize {
    messages
        .iter()
        .filter(|message| matches!(message, SessionMessage::User { .. }))
        .count()
}

#[test]
fn goal_audits_in_a_separate_session_and_stops_when_done() {
    if support::skip_e2e() {
        eprintln!("skipping goals e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping goals e2e test: opencode was not found");
        return;
    }
    if std::env::var_os("RUST_HARNESS_MOCK_SCRIPTED_PROVIDER").is_none() {
        eprintln!("skipping goals e2e test: RUST_HARNESS_MOCK_SCRIPTED_PROVIDER is not set");
        return;
    }

    let mock = MockProvider::start(Script {
        turns: vec![
            turn("not done"),
            turn(
                r#"{"decision":"continue","reason":"the test still fails","next":"fix the failing test"}"#,
            ),
            turn("done"),
            turn(r#"{"decision":"finish","reason":"tests pass"}"#),
        ],
    });
    let project = support::project("goals");
    project.use_mock_provider(&mock.base_url());

    let config = EngineConfig::new(project.path()).with_goal_auditor_model(ModelRef {
        id: "mock-model".into(),
        provider_id: Some("mock".into()),
        variant: None,
    });
    let started = support::start_engine_with(config);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(120);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session(&started.directory));
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!("predicate only accepts SessionCreated")
    };
    let main_id = session.id;

    assert!(started.engine.start_goal(
        &started.directory,
        main_id.clone(),
        "make the tests pass",
        GoalLimits::default(),
    ));

    for expected in 0..2 {
        let EngineEvent::GoalAudited { decision, .. } =
            support::wait_for(&events, deadline, |event| {
                matches!(event, EngineEvent::GoalAudited { .. })
            })
        else {
            unreachable!("predicate only accepts GoalAudited")
        };
        match expected {
            0 => assert_eq!(
                decision,
                harness_core::AuditDecision::Continue {
                    next: "fix the failing test".into()
                }
            ),
            _ => assert_eq!(
                decision,
                harness_core::AuditDecision::Finish {
                    reason: "tests pass".into()
                }
            ),
        }
    }

    let EngineEvent::GoalStopped { goal, reason } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::GoalStopped { .. })
    }) else {
        unreachable!("predicate only accepts GoalStopped")
    };
    assert_eq!(goal.session_id, main_id);
    assert_eq!(
        reason,
        Stopped::Done {
            reason: "tests pass".into()
        }
    );
    assert_eq!(goal.continuations, 1);
    let auditor_id = goal
        .auditor_session_id
        .clone()
        .expect("the goal has an auditor session");
    assert_ne!(auditor_id, main_id);

    // Durable effects: both sessions exist, the main session ran two turns,
    // and the auditor ran once per audit with no tool output at all.
    let main_messages = transcript(
        &started.engine,
        &events,
        &started.directory,
        &main_id,
        deadline,
    );
    assert_eq!(count_users(&main_messages), 2, "main session turns");
    assert_eq!(
        assistant_texts(&main_messages),
        vec!["not done".to_string(), "done".to_string()]
    );

    let auditor_messages = transcript(
        &started.engine,
        &events,
        &started.directory,
        &auditor_id,
        deadline,
    );
    assert_eq!(count_users(&auditor_messages), 2, "audit requests");
    assert_eq!(
        assistant_texts(&auditor_messages),
        vec![
            r#"{"decision":"continue","reason":"the test still fails","next":"fix the failing test"}"#
                .to_string(),
            r#"{"decision":"finish","reason":"tests pass"}"#.to_string()
        ]
    );
    for message in &auditor_messages {
        if let SessionMessage::Assistant { content, .. } = message {
            assert!(
                content
                    .iter()
                    .all(|part| matches!(part, AssistantPart::Text { .. })),
                "the auditor session must not receive tool output"
            );
        }
    }

    assert!(started.engine.refresh_sessions(&started.directory));
    let EngineEvent::SessionsLoaded { sessions } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    }) else {
        unreachable!("predicate only accepts SessionsLoaded")
    };
    assert!(sessions.iter().any(|session| session.id == main_id));
    assert!(sessions.iter().any(|session| session.id == auditor_id));

    support::shutdown(started.engine, started.port);
}
