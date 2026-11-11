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

    support::shutdown(started.engine, started.port);
}
