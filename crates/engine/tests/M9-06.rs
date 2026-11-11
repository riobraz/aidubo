//! M9-06 — usage limits load without surfacing a 503 as an error.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn loads_usage_limits_or_reports_them_unavailable() {
    if support::skip_e2e() {
        eprintln!("skipping usage limits e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping usage limits e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("limits");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.load_usage_limits(1));
    let EngineEvent::UsageLimitsLoaded { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::UsageLimitsLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(request_id, 1);
    let limits = result.expect("a 503 must be Ok(None), not an error");
    if let Some(limits) = limits {
        for budget in &limits.budgets {
            let _ = budget.share();
        }
    }

    support::shutdown(started.engine, started.port);
}
