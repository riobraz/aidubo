//! M10-01 — working-tree status, base, and repository initialization.

mod support;

use std::process::Command;
use std::time::{Duration, Instant};

use harness_engine::EngineEvent;

fn git(project: &support::TestProject, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(project.path())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn loads_working_tree_status_and_base() {
    if support::skip_e2e() {
        eprintln!("skipping vcs status e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping vcs status e2e test: opencode was not found");
        return;
    }
    if !git_available() {
        eprintln!("skipping vcs status e2e test: git was not found");
        return;
    }

    let project = support::project("vcs-status");
    assert!(git(&project, &["init", "-q"]));
    assert!(git(&project, &["config", "user.email", "e2e@example.com"]));
    assert!(git(&project, &["config", "user.name", "E2E"]));
    std::fs::write(project.join("a.txt"), "a\nb\nc\n").expect("write");
    assert!(git(&project, &["add", "."]));
    assert!(git(&project, &["commit", "-qm", "initial"]));

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    // The server snapshots on filesystem events; retry until it sees us.
    let mut files = Vec::new();
    for attempt in 0..15_u64 {
        std::fs::write(project.join("a.txt"), "a\nB\nc\nd\n").expect("modify");
        std::fs::write(project.join("trigger.txt"), format!("{attempt}\n")).expect("trigger");
        std::thread::sleep(Duration::from_millis(400));
        let request_id = attempt + 1;
        assert!(started.engine.load_vcs_status(request_id));
        let EngineEvent::VcsStatusLoaded {
            request_id: got,
            result,
        } = support::wait_for(
            &events,
            deadline,
            |event| matches!(event, EngineEvent::VcsStatusLoaded { request_id: id, .. } if *id == request_id),
        )
        else {
            unreachable!("predicate only accepts VcsStatusLoaded");
        };
        assert_eq!(got, request_id);
        files = result.expect("status request failed");
        if !files.is_empty() {
            break;
        }
    }

    let file = files
        .iter()
        .find(|file| file.file.ends_with("a.txt"))
        .unwrap_or_else(|| panic!("a.txt missing from {:?}", files));
    assert_eq!(file.additions, 2);
    assert_eq!(file.deletions, 1);
    assert_eq!(file.status, "modified");

    assert!(started.engine.load_vcs_base(2));
    let EngineEvent::VcsBaseLoaded { result, .. } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::VcsBaseLoaded { .. })
    }) else {
        unreachable!("predicate only accepts VcsBaseLoaded");
    };
    let base = result.expect("base request failed").expect("no base ref");
    assert!(!base.name.is_empty(), "base name is empty");
    assert!(!base.reference.is_empty(), "base ref is empty");

    support::shutdown(started.engine, started.port);
}

#[test]
fn initializes_a_repository_when_missing() {
    if support::skip_e2e() {
        eprintln!("skipping vcs init e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping vcs init e2e test: opencode was not found");
        return;
    }
    if !git_available() {
        eprintln!("skipping vcs init e2e test: git was not found");
        return;
    }

    let project = support::project("vcs-init");
    assert!(!project.join(".git").exists());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.init_repo(1));
    let EngineEvent::RepoInitialized { request_id, result } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::RepoInitialized { .. })
        })
    else {
        unreachable!("predicate only accepts RepoInitialized");
    };
    assert_eq!(request_id, 1);
    result.expect("init failed");
    assert!(project.join(".git").exists(), "no .git after init");

    support::shutdown(started.engine, started.port);
}
