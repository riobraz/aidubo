//! M13-02 — provider and integration discovery.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn lists_providers_and_integrations() {
    if support::skip_e2e() {
        eprintln!("skipping provider e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping provider e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("providers");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.load_providers(1));
    let EngineEvent::ProvidersLoaded { result, .. } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::ProvidersLoaded { .. })
        })
    else {
        unreachable!()
    };
    let providers = result.expect("provider list failed");
    assert!(providers.iter().any(|provider| provider.id == "mock"));

    assert!(started.engine.load_integrations(2));
    let EngineEvent::IntegrationsLoaded { result, .. } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::IntegrationsLoaded { .. })
        })
    else {
        unreachable!()
    };
    let integrations = result.expect("integration list failed");
    for integration in &integrations {
        let _ = integration.method_count();
    }

    // A credential sent to a missing integration must come back as a short
    // error that never echoes the key.
    let secret = "sk-e2e-abcdef1234567890abcdef1234567890";
    assert!(started.engine.send_key("definitely-missing", secret));
    let EngineEvent::AuthAttemptFailed {
        integration_id,
        message,
    } = support::wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::AuthAttemptFailed { integration_id, .. }
                if integration_id == "definitely-missing"
        )
    })
    else {
        unreachable!()
    };
    assert_eq!(integration_id, "definitely-missing");
    assert!(message.contains("404"), "{message}");
    assert!(!message.contains(secret), "{message}");
    assert!(!message.contains("sk-e2e"), "{message}");
    assert!(message.len() < 400, "{message}");

    // Cancelling an attempt the server never issued is a clean terminal event,
    // not a panic or a hang; the server currently answers it with success.
    assert!(
        started
            .engine
            .cancel_oauth("definitely-missing", "att_missing")
    );
    let cancelled = support::wait_for(&events, deadline, |event| {
        matches!(
            event,
            EngineEvent::AuthAttemptCancelled { .. } | EngineEvent::AuthAttemptFailed { .. }
        )
    });
    match cancelled {
        EngineEvent::AuthAttemptCancelled { integration_id } => {
            assert_eq!(integration_id, "definitely-missing");
        }
        EngineEvent::AuthAttemptFailed {
            integration_id,
            message,
        } => {
            assert_eq!(integration_id, "definitely-missing");
            assert!(!message.contains(secret), "{message}");
        }
        other => panic!("unexpected cancel event: {other:?}"),
    }

    support::shutdown(started.engine, started.port);
}
