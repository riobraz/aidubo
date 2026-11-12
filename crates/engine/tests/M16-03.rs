//! M16-03 — export, import, and statistics.
//!
//! Runs one real turn through a mock provider, exports the session twice
//! (sanitized and not), imports it into a second project with a fresh id, and
//! proves the messages survived on the server. A same-id import must be
//! refused with the server's conflict message and no new session.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use harness_protocol::{PromptRequest, SessionTransfer, StatsRange};
use support::Started;
use support::mock::{MockProvider, Script};

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// Wait until the server stores the turn's user and assistant messages.
fn wait_for_messages(
    started: &Started,
    session_id: &str,
    expected: usize,
    deadline: Instant,
) -> usize {
    loop {
        assert!(
            started
                .engine
                .load_transcript(&started.directory, session_id)
        );
        let EngineEvent::TranscriptLoaded { messages, .. } =
            support::wait_for(&started.events, deadline, |event| {
                matches!(event, EngineEvent::TranscriptLoaded { .. })
            })
        else {
            unreachable!()
        };
        if messages.len() >= expected {
            return messages.len();
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {expected} messages");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn export(
    started: &Started,
    session_id: &str,
    sanitize: bool,
    path: &std::path::Path,
    deadline: Instant,
) -> SessionTransfer {
    assert!(
        started
            .engine
            .export_session(&started.directory, session_id, sanitize, path)
    );
    let EngineEvent::SessionExported { bytes, .. } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::SessionExported { .. })
        })
    else {
        unreachable!()
    };
    assert!(bytes > 0, "the export was empty");
    let raw = std::fs::read(path).expect("read export");
    SessionTransfer::parse(&raw).expect("parse export")
}

#[test]
fn exports_imports_and_reports_stats() {
    if support::skip_e2e() {
        eprintln!("skipping session transfer e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping session transfer e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project_a = support::project("transfer-a");
    project_a.use_mock_provider(&mock.base_url());

    let started_a = support::start_engine(&project_a);
    let events_a = started_a.events.clone();
    let deadline = Instant::now() + Duration::from_secs(120);
    support::wait_for_stream(&events_a, deadline);

    assert!(started_a.engine.create_session(&started_a.directory,));
    let EngineEvent::SessionCreated { session } = support::wait_for(&events_a, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let original_id = session.id.clone();
    assert!(started_a.engine.rename_session(
        &started_a.directory,
        original_id.clone(),
        "Transfer round trip"
    ));
    support::wait_for(&events_a, deadline, |event| {
        matches!(event, EngineEvent::SessionRenamed { .. })
    });

    assert!(started_a.engine.send_prompt(
        &started_a.directory,
        original_id.clone(),
        PromptRequest {
            text: "say ok".into(),
            ..PromptRequest::default()
        },
    ));
    support::wait_for(&events_a, deadline, |event| {
        matches!(
            event,
            EngineEvent::Stream(harness_engine::StreamEvent::ExecutionFinished { .. })
        )
    });
    let stored = wait_for_messages(&started_a, &original_id, 2, deadline);
    assert!(
        stored >= 2,
        "one turn should store at least the user and assistant messages"
    );

    let sanitized_path = project_a.join("sanitized.json");
    let sanitized = export(&started_a, &original_id, true, &sanitized_path, deadline);
    let sanitized_raw = std::fs::read_to_string(&sanitized_path).expect("sanitized export");
    for secret in ["sk-", "Bearer ", "Authorization"] {
        assert!(
            !sanitized_raw.contains(secret),
            "sanitized export contains {secret:?}"
        );
    }
    assert!(
        !sanitized_raw.contains("Transfer round trip"),
        "sanitize left the session title in the file"
    );
    assert!(
        sanitized_raw.contains("[redacted:session-title:"),
        "sanitize did not mark the redacted title"
    );
    assert_eq!(sanitized.session_id(), original_id);

    let plain_path = project_a.join("plain.json");
    let plain = export(&started_a, &original_id, false, &plain_path, deadline);
    assert_eq!(plain.title(), Some("Transfer round trip"));
    let message_count = plain.message_count();
    assert!(message_count >= 2, "the plain export lost messages");
    assert_eq!(sanitized.message_count(), message_count);

    // Storage is global and message ids are global too: importing the export
    // beside its source works only as a copy with fresh ids.
    let project_b = support::project("transfer-b");
    project_b.use_mock_provider(&mock.base_url());
    let started_b = support::start_engine(&project_b);
    let events_b = started_b.events.clone();
    support::wait_for_stream(&events_b, deadline);

    assert!(
        started_b
            .engine
            .import_session(&started_b.directory, &plain_path, true)
    );
    let EngineEvent::SessionImported { session } =
        support::wait_for(&events_b, deadline, |event| {
            matches!(event, EngineEvent::SessionImported { .. })
        })
    else {
        unreachable!()
    };
    let imported_id = session.id.clone();
    assert_ne!(imported_id, original_id);
    assert_eq!(session.title.as_deref(), Some("Transfer round trip"));

    let imported_count = wait_for_messages(&started_b, &imported_id, message_count, deadline);
    assert_eq!(
        imported_count, message_count,
        "the imported session lost messages"
    );

    // Installing the same export without a copy is refused: the id exists and
    // the server never overwrites a session.
    assert!(
        started_b
            .engine
            .import_session(&started_b.directory, &plain_path, false)
    );
    let EngineEvent::SessionImportError { message } =
        support::wait_for(&events_b, deadline, |event| {
            matches!(event, EngineEvent::SessionImportError { .. })
        })
    else {
        unreachable!()
    };
    assert!(
        message.contains("already exists"),
        "unexpected import error: {message}"
    );

    assert!(started_b.engine.refresh_sessions(&started_b.directory,));
    let EngineEvent::SessionsLoaded { sessions } =
        support::wait_for(&events_b, deadline, |event| {
            matches!(event, EngineEvent::SessionsLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert!(
        sessions.iter().any(|session| session.id == imported_id),
        "the imported session is missing from the project list"
    );
    assert!(
        sessions.iter().all(|session| session.id != original_id),
        "the conflicting import created a session"
    );
    assert_eq!(
        sessions.len(),
        1,
        "the conflict import left a side effect: {sessions:?}"
    );

    // Statistics are global, so a wide range must include this turn.
    assert!(started_b.engine.load_stats(
        &started_b.directory,
        1,
        StatsRange {
            from: 0,
            to: now_millis() as i64,
        }
    ));
    let EngineEvent::StatsLoaded { result, .. } = support::wait_for(&events_b, deadline, |event| {
        matches!(event, EngineEvent::StatsLoaded { .. })
    }) else {
        unreachable!()
    };
    let stats = result.expect("stats");
    assert!(stats.sessions >= 1);
    assert!(stats.prompts >= 1);

    // Delete both sessions so the test does not accumulate storage.
    assert!(
        started_b
            .engine
            .delete_session(&started_b.directory, imported_id.clone())
    );
    support::wait_for(
        &events_b,
        deadline,
        |event| matches!(event, EngineEvent::SessionDeleted { id } if id == &imported_id),
    );
    assert!(
        started_a
            .engine
            .delete_session(&started_a.directory, original_id.clone())
    );
    support::wait_for(
        &events_a,
        deadline,
        |event| matches!(event, EngineEvent::SessionDeleted { id } if id == &original_id),
    );

    support::shutdown(started_b.engine, started_b.port);
    support::shutdown(started_a.engine, started_a.port);
}
