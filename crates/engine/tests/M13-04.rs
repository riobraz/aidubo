//! M13-04 — list plugins and run an update check.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn lists_plugins_and_checks_for_updates() {
    if support::skip_e2e() {
        eprintln!("skipping plugin e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping plugin e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("plugins");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.load_plugins(&started.directory, 1));
    let EngineEvent::PluginsLoaded { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::PluginsLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(request_id, 1);
    let plugins = result.expect("plugin list failed");
    for plugin in &plugins {
        assert!(!plugin.source_label().is_empty());
        let _ = plugin.status_label();
    }

    assert!(started.engine.check_plugins(&started.directory, 2));
    let EngineEvent::PluginsChecked { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::PluginsChecked { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(request_id, 2);
    result.expect("plugin check failed");

    support::shutdown(started.engine, started.port);
}
