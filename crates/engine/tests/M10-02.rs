//! M10-02 — the VCS diff data path against a real repository.

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

#[test]
fn loads_the_working_tree_diff_from_a_real_repository() {
    if support::skip_e2e() {
        eprintln!("skipping diff e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping diff e2e test: opencode was not found");
        return;
    }
    if !git(&support::project("probe"), &["--version"]) {
        eprintln!("skipping diff e2e test: git was not found");
        return;
    }

    let project = support::project("diff");
    assert!(git(&project, &["init", "-q"]), "git init failed");
    assert!(git(&project, &["config", "user.email", "e2e@example.com"]));
    assert!(git(&project, &["config", "user.name", "E2E"]));
    std::fs::write(project.join("a.txt"), "a\nb\nc\n").expect("write");
    assert!(git(&project, &["add", "."]), "git add failed");
    assert!(
        git(&project, &["commit", "-qm", "initial"]),
        "git commit failed"
    );
    std::fs::write(project.join("a.txt"), "a\nB\nc\nd\n").expect("modify");

    if std::env::var_os("RUST_HARNESS_DEBUG_EVENTS").is_some() {
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(project.path())
            .output()
            .expect("git status");
        eprintln!("project: {}", project.path().display());
        eprintln!("status: {}", String::from_utf8_lossy(&status.stdout));
    }

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    // OpenCode snapshots the working tree from its file watcher. The first
    // events after startup can be dropped while the watcher warms up, so keep
    // touching a file and asking until the snapshot includes the change.
    let mut diffs = Vec::new();
    for attempt in 0..15_u64 {
        std::fs::write(project.join("a.txt"), "a\nB\nc\nd\n").expect("modify");
        std::fs::write(project.join("trigger.txt"), format!("{attempt}\n")).expect("trigger");
        std::thread::sleep(Duration::from_millis(400));

        let request_id = attempt + 1;
        assert!(
            started
                .engine
                .load_vcs_diff(request_id, "working", None, None, "files"),
            "engine stopped before the diff request"
        );
        let EngineEvent::VcsDiffLoaded {
            request_id: got,
            result,
        } = support::wait_for(
            &events,
            deadline,
            |event| matches!(event, EngineEvent::VcsDiffLoaded { request_id: id, .. } if *id == request_id),
        )
        else {
            unreachable!("predicate only accepts VcsDiffLoaded");
        };
        assert_eq!(got, request_id);
        diffs = result.expect("diff request failed");
        if !diffs.is_empty() {
            break;
        }
    }

    let file = diffs
        .iter()
        .find(|diff| diff.file.ends_with("a.txt"))
        .unwrap_or_else(|| panic!("a.txt missing from {:?}", diffs));
    assert_eq!(file.additions, 2, "additions for {:?}", file);
    assert_eq!(file.deletions, 1, "deletions for {:?}", file);
    assert!(!file.patch.is_empty());
    assert!(!file.more);

    support::shutdown(started.engine, started.port);
}
