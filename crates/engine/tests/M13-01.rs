//! M13-01 — MCP servers.
//!
//! Adds a stdio server that the test owns, waits for it to connect, toggles it
//! offline and online, then removes it. Every step is checked against the
//! server's own list, not against a local echo.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use harness_protocol::McpStatus;
use serde_json::json;
use support::Started;

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn load_status(
    started: &Started,
    name: &str,
    request_id: u64,
    deadline: Instant,
) -> Option<McpStatus> {
    assert!(started.engine.load_mcp(request_id));
    let EngineEvent::McpLoaded { result, .. } = support::wait_for(
        &started.events,
        deadline,
        |event| matches!(event, EngineEvent::McpLoaded { request_id: id, .. } if *id == request_id),
    ) else {
        unreachable!()
    };
    let servers = result.expect("mcp list failed");
    servers
        .into_iter()
        .find(|server| server.name == name)
        .map(|server| server.status)
}

fn wait_for_status(
    started: &Started,
    name: &str,
    deadline: Instant,
    accept: impl Fn(&McpStatus) -> bool,
) -> McpStatus {
    let mut request_id = 10;
    loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for MCP server {name} to reach the expected status");
        }
        if let Some(status) = load_status(started, name, request_id, deadline)
            && accept(&status)
        {
            return status;
        }
        request_id += 1;
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_change(started: &Started, name: &str, deadline: Instant) {
    let EngineEvent::McpChanged { name: changed } = support::wait_for(
        &started.events,
        deadline,
        |event| matches!(event, EngineEvent::McpChanged { name: changed } if changed == name),
    ) else {
        unreachable!()
    };
    assert_eq!(changed, name);
}

#[test]
fn adds_connects_and_removes_an_mcp_server() {
    if support::skip_e2e() {
        eprintln!("skipping mcp e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping mcp e2e test: opencode was not found");
        return;
    }

    let project = support::project("mcp");
    let config_path = project.join("opencode.json");
    let config_before = std::fs::read(&config_path).ok();

    let started = support::start_engine(&project);
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&started.events, deadline);

    let name = format!("m13-echo-{}-{}", std::process::id(), now_millis());
    assert!(
        load_status(&started, &name, 1, deadline).is_none(),
        "the test server already exists before it was added"
    );

    let command = env!("CARGO_BIN_EXE_mcp-echo");
    assert!(started.engine.add_mcp(
        name.clone(),
        json!({ "type": "local", "command": [command] })
    ));
    wait_for_change(&started, &name, deadline);

    let connected = wait_for_status(&started, &name, deadline, |status| {
        matches!(status, McpStatus::Connected)
    });
    assert_eq!(connected, McpStatus::Connected);

    assert!(started.engine.disconnect_mcp(name.clone()));
    wait_for_change(&started, &name, deadline);
    let disabled = wait_for_status(&started, &name, deadline, |status| {
        matches!(status, McpStatus::Disabled)
    });
    assert_eq!(disabled, McpStatus::Disabled);

    assert!(started.engine.connect_mcp(name.clone()));
    wait_for_change(&started, &name, deadline);
    wait_for_status(&started, &name, deadline, |status| {
        matches!(status, McpStatus::Connected)
    });

    assert!(started.engine.remove_mcp(name.clone()));
    wait_for_change(&started, &name, deadline);
    assert!(
        load_status(&started, &name, 2, deadline).is_none(),
        "the MCP server is still listed after it was removed"
    );

    // Removing a missing server must surface the server's error, not a
    // generic success.
    let missing = format!("{name}-missing");
    assert!(started.engine.remove_mcp(missing.clone()));
    let EngineEvent::McpError {
        name: failed,
        message,
    } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::McpError { .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(failed, missing);
    assert!(
        message.contains("not found"),
        "unexpected MCP error: {message}"
    );

    assert_eq!(
        std::fs::read(&config_path).ok(),
        config_before,
        "the test wrote the project's opencode.json"
    );

    support::shutdown(started.engine, started.port);
}
