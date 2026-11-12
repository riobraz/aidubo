//! M14-01 — the PTY WebSocket transport.
//!
//! Creates a real PTY, reads its output, resizes it, and proves that deleting
//! it terminates the OS process and removes it from the server list.

mod support;

use std::collections::BTreeMap;
use std::process::Command;
use std::time::{Duration, Instant};

use harness_engine::{CreatePtyRequest, EngineEvent};
use support::Started;

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_for_output(started: &Started, deadline: Instant, id: &str, needle: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        if Instant::now() >= deadline {
            panic!("timed out waiting for {needle:?} in the terminal output");
        }
        match started.events.try_recv() {
            Ok(EngineEvent::PtyOutput {
                id: event_id,
                bytes,
            }) if event_id == id => {
                output.extend_from_slice(&bytes);
                if output.windows(needle.len()).any(|window| window == needle) {
                    return output;
                }
            }
            Ok(EngineEvent::PtyError {
                id: event_id,
                message,
            }) if event_id == id => {
                panic!("terminal failed: {message}");
            }
            Ok(EngineEvent::Failed { message }) => panic!("engine failed: {message}"),
            Ok(_) => {}
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(async_channel::TryRecvError::Closed) => panic!("engine event stream closed"),
        }
    }
}

#[test]
fn pty_streams_output_resizes_and_terminates() {
    if support::skip_e2e() {
        eprintln!("skipping PTY e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping PTY e2e test: opencode was not found");
        return;
    }

    let project = support::project("pty");
    let cwd = std::fs::canonicalize(project.path()).expect("canonical cwd");
    let started = support::start_engine(&project);
    let mut deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&started.events, deadline);

    assert!(started.engine.create_pty(
        &started.directory,
        CreatePtyRequest {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf hello; sleep 30".into()],
            cwd: cwd.to_string_lossy().into_owned(),
            title: "M14-01 e2e".into(),
            env: BTreeMap::new(),
        }
    ));
    let EngineEvent::PtyCreated { pty } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::PtyCreated { .. })
    }) else {
        unreachable!()
    };
    assert!(pty.status.is_running());
    assert!(pty.pid > 0);

    assert!(started.engine.connect_pty(&started.directory, &pty.id));
    deadline = Instant::now() + Duration::from_secs(30);
    let output = wait_for_output(&started, deadline, &pty.id, b"hello");
    assert!(
        output.starts_with(b"hello"),
        "first terminal bytes were {:?}",
        String::from_utf8_lossy(&output)
    );

    assert!(
        started
            .engine
            .resize_pty(&started.directory, &pty.id, 100, 30)
    );
    let EngineEvent::PtyCreated { pty: resized } = support::wait_for(
        &started.events,
        deadline,
        |event| matches!(event, EngineEvent::PtyCreated { pty: refreshed } if refreshed.id == pty.id),
    ) else {
        unreachable!()
    };
    assert_eq!(resized.id, pty.id);

    // The process must be alive before delete, otherwise "gone" after delete
    // would not prove that DELETE terminated it.
    assert!(alive(pty.pid), "PTY pid {} exited too early", pty.pid);

    assert!(started.engine.delete_pty(&started.directory, &pty.id));
    let EngineEvent::PtyClosed { id } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::PtyClosed { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(id, pty.id);

    let gone_deadline = Instant::now() + Duration::from_secs(5);
    while alive(pty.pid) {
        if Instant::now() >= gone_deadline {
            panic!("PTY process {} is still alive after delete", pty.pid);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(started.engine.list_ptys(&started.directory,));
    let EngineEvent::PtysLoaded { result } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::PtysLoaded { .. })
        })
    else {
        unreachable!()
    };
    let ptys = result.expect("PTY list failed");
    assert!(
        !ptys.iter().any(|candidate| candidate.id == pty.id),
        "deleted PTY is still listed"
    );

    support::shutdown(started.engine, started.port);
}
