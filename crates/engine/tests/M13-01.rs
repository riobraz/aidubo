//! M13-01 — list MCP servers.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn lists_mcp_servers() {
    if support::skip_e2e() {
        eprintln!("skipping mcp e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping mcp e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("mcp");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.load_mcp(1));
    let EngineEvent::McpLoaded { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::McpLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(request_id, 1);
    let servers = result.expect("mcp list failed");
    for server in &servers {
        assert!(!server.name.is_empty());
        let _ = server.status_label();
    }

    support::shutdown(started.engine, started.port);
}
