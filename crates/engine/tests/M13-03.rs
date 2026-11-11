//! M13-03 — saved permission rules for a project.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn lists_saved_permissions_for_the_project() {
    if support::skip_e2e() {
        eprintln!("skipping saved permissions e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping saved permissions e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("saved-perms");
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
    let project_id = session.project_id.expect("session project id");

    assert!(started.engine.load_saved_permissions(1, project_id));
    let EngineEvent::SavedPermissionsLoaded { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::SavedPermissionsLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(request_id, 1);
    let rules = result.expect("saved permissions failed");
    for rule in &rules {
        assert!(!rule.action.is_empty());
    }

    support::shutdown(started.engine, started.port);
}
