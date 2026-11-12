//! M14-03 — the terminal dock's data path.
//!
//! Runs a real shell in a PTY, parses its bytes into a `harness_term::Grid`
//! the way the dock does, resizes it, pushes 10 MB through the batching
//! pipeline, and proves deleting it terminates the OS process.

mod support;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use harness_engine::{CreatePtyRequest, EngineEvent};
use harness_term::Grid;
use support::Started;

/// The dock's batch ceiling; every event must stay at or under it.
const OUTPUT_LIMIT: usize = 64 * 1024;
/// How much output the burst test pushes through the socket.
const BURST_BYTES: usize = 10_000_000;
/// The burst must reach the app in this window. The sidecar's PTY pump, not
/// the app, sets the pace: a raw WebSocket probe measured 10 MB of `yes`
/// output taking ~10 s on opencode 2.2.7, so this is a hang guard.
const DRAIN_LIMIT: Duration = Duration::from_secs(60);
/// The dock parses one event per update, so the worst single event is the
/// worst frame the burst can cost.
const FRAME_LIMIT: Duration = Duration::from_millis(100);

fn gated() -> bool {
    if support::skip_e2e() {
        eprintln!("skipping terminal dock e2e test: RUST_HARNESS_SKIP_E2E is set");
        return false;
    }
    if std::env::var_os("RUST_HARNESS_PTY_E2E").is_none() {
        eprintln!("skipping terminal dock e2e test: set RUST_HARNESS_PTY_E2E=1 to run it");
        return false;
    }
    if !support::opencode_available() {
        eprintln!("skipping terminal dock e2e test: opencode was not found");
        return false;
    }
    true
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Drain the dock for one PTY into a grid, the way `PtySession::feed` does.
struct Terminal {
    id: String,
    grid: Grid,
    output: Vec<u8>,
    events: usize,
    largest: usize,
    /// Time spent applying output to the grid, i.e. the UI thread's share.
    applied: Duration,
    /// The slowest single event, i.e. the worst frame the dock can cause.
    slowest: Duration,
}

impl Terminal {
    fn new(id: String) -> Self {
        Self {
            id,
            grid: Grid::new(120, 40),
            output: Vec::new(),
            events: 0,
            largest: 0,
            applied: Duration::ZERO,
            slowest: Duration::ZERO,
        }
    }

    /// Apply every queued event; panics on a terminal or engine failure.
    fn pump(&mut self, started: &Started) -> bool {
        let mut saw_output = false;
        while let Ok(event) = started.events.try_recv() {
            match event {
                EngineEvent::PtyOutput { id, bytes } if id == self.id => {
                    self.events += 1;
                    self.largest = self.largest.max(bytes.len());
                    let began = Instant::now();
                    self.grid.feed(&bytes);
                    let took = began.elapsed();
                    self.applied += took;
                    self.slowest = self.slowest.max(took);
                    self.output.extend_from_slice(&bytes);
                    saw_output = true;
                }
                EngineEvent::PtyClosed { id } if id == self.id => {
                    // The shell was told to exit; the test waits for output
                    // first, so a close this early is a failure.
                    panic!("terminal closed before the scripted output arrived");
                }
                EngineEvent::PtyError { id, message } if id == self.id => {
                    panic!("terminal failed: {message}");
                }
                EngineEvent::Failed { message } => panic!("engine failed: {message}"),
                _ => {}
            }
        }
        saw_output
    }

    fn text(&self) -> String {
        self.grid
            .visible()
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Pump until `predicate` holds, returning how long it took.
    fn wait_for(
        &mut self,
        started: &Started,
        deadline: Instant,
        mut predicate: impl FnMut(&Self) -> bool,
    ) -> Duration {
        let began = Instant::now();
        let mut traced = Instant::now();
        loop {
            if predicate(self) {
                return began.elapsed();
            }
            if std::env::var_os("RUST_HARNESS_TERMINAL_TRACE").is_some()
                && traced.elapsed() >= Duration::from_secs(1)
            {
                traced = Instant::now();
                eprintln!(
                    "trace: {} bytes, {} events after {:?}",
                    self.output.len(),
                    self.events,
                    began.elapsed()
                );
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out waiting for terminal output ({} bytes so far)",
                    self.output.len()
                );
            }
            if !self.pump(started) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
fn terminal_dock_streams_resizes_and_terminates() {
    if !gated() {
        return;
    }

    let project = support::project("terminal-dock");
    let cwd = std::fs::canonicalize(project.path()).expect("canonical cwd");
    let started = support::start_engine(&project);
    let mut deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&started.events, deadline);

    assert!(started.engine.create_pty(
        &started.directory,
        CreatePtyRequest {
            command: "/bin/sh".into(),
            args: Vec::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            title: "M14-03 e2e".into(),
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
    let pid = pty.pid;

    assert!(started.engine.connect_pty(&started.directory, &pty.id));
    let mut terminal = Terminal::new(pty.id.clone());

    // The marker is split in the command so the shell's echo of the input
    // cannot satisfy the search; only the command's output contains it.
    assert!(started.engine.write_pty(
        &started.directory,
        &pty.id,
        b"printf '%s%s\\n' rea dy\n".to_vec()
    ));
    deadline = Instant::now() + Duration::from_secs(30);
    terminal.wait_for(&started, deadline, |terminal| {
        terminal.text().contains("ready")
    });

    assert!(
        started
            .engine
            .resize_pty(&started.directory, &pty.id, 120, 40)
    );
    let EngineEvent::PtyCreated { pty: resized } = support::wait_for(
        &started.events,
        deadline,
        |event| matches!(event, EngineEvent::PtyCreated { pty: refreshed } if refreshed.id == pty.id),
    ) else {
        unreachable!()
    };
    assert_eq!(resized.id, pty.id);

    let burst = format!("yes | head -c {BURST_BYTES}; printf '\\n%s\\n' \"D$(printf O)NE>\"\n");
    assert!(
        started
            .engine
            .write_pty(&started.directory, &pty.id, burst.as_bytes().to_vec())
    );
    deadline = Instant::now() + Duration::from_secs(60);
    let drained = terminal.wait_for(&started, deadline, |terminal| {
        terminal.output.len() >= BURST_BYTES && terminal.text().contains("DONE>")
    });
    assert!(
        drained < DRAIN_LIMIT,
        "draining {BURST_BYTES} bytes took {drained:?}, over the {DRAIN_LIMIT:?} guard"
    );
    assert!(
        terminal.slowest < FRAME_LIMIT,
        "one {} KiB event took {:?} to apply, over the {FRAME_LIMIT:?} frame limit",
        terminal.largest / 1024,
        terminal.slowest
    );
    assert!(
        terminal.events > 1,
        "a burst arrived as one event, so batching did not apply"
    );
    assert!(
        terminal.largest <= OUTPUT_LIMIT,
        "one event carried {} bytes, over the {OUTPUT_LIMIT}-byte limit",
        terminal.largest
    );
    eprintln!(
        "terminal dock: {} bytes in {drained:?} across {} events (largest {} bytes, grid apply {:?} total, {:?} worst)",
        terminal.output.len(),
        terminal.events,
        terminal.largest,
        terminal.applied,
        terminal.slowest
    );

    assert!(alive(pid), "PTY pid {pid} exited before delete");
    assert!(started.engine.delete_pty(&started.directory, &pty.id));
    let EngineEvent::PtyClosed { id } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::PtyClosed { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(id, pty.id);

    let gone_deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < gone_deadline,
            "PTY process {pid} is still alive after delete"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // A terminal the app never deleted must not outlive the app.
    assert!(started.engine.create_pty(
        &started.directory,
        CreatePtyRequest {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 300".into()],
            cwd: cwd.to_string_lossy().into_owned(),
            title: "M14-03 quit".into(),
            env: BTreeMap::new(),
        }
    ));
    deadline = Instant::now() + Duration::from_secs(30);
    let EngineEvent::PtyCreated { pty: leftover } = support::wait_for(
        &started.events,
        deadline,
        |event| matches!(event, EngineEvent::PtyCreated { pty: created } if created.id != pty.id),
    ) else {
        unreachable!()
    };
    assert!(
        alive(leftover.pid),
        "leftover PTY pid {} exited early",
        leftover.pid
    );

    support::shutdown(started.engine, started.port);

    let orphan_deadline = Instant::now() + Duration::from_secs(5);
    while alive(leftover.pid) {
        assert!(
            Instant::now() < orphan_deadline,
            "PTY process {} outlived the app",
            leftover.pid
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
