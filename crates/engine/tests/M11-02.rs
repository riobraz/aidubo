//! M11-02 — worktrees against a real git repository.
//!
//! Creates a worktree through the server, proves the directory exists and git
//! knows about it, opens a session inside it, then removes it and proves both
//! the directory and the git registration are gone. The assertions are on the
//! filesystem and git, not on local events.

mod support;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::Started;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Remove the server-chosen worktree on drop, so a failed assertion cannot
/// leak a directory outside the temp project.
struct Cleanup(Option<String>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn create_session(
    started: &Started,
    directory: &str,
    deadline: Instant,
) -> harness_protocol::Session {
    assert!(started.engine.create_session(directory));
    let EngineEvent::SessionCreated { session } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::SessionCreated { .. })
        })
    else {
        unreachable!("predicate only accepts SessionCreated");
    };
    session
}

fn load_worktrees(started: &Started, deadline: Instant) -> Vec<harness_protocol::Worktree> {
    assert!(started.engine.load_worktrees(&started.directory, 1));
    let EngineEvent::WorktreesLoaded { directory, result } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::WorktreesLoaded { .. })
        })
    else {
        unreachable!("predicate only accepts WorktreesLoaded");
    };
    assert_eq!(directory, started.directory);
    result.expect("worktree list")
}

#[test]
fn creates_opens_and_removes_a_worktree() {
    if support::skip_e2e() {
        eprintln!("skipping worktree e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping worktree e2e test: opencode was not found");
        return;
    }
    if Command::new("git")
        .arg("--version")
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping worktree e2e test: git was not found");
        return;
    }

    let project = support::project("worktree");
    git(project.path(), &["init", "-q"]);
    git(project.path(), &["config", "user.email", "e2e@example.com"]);
    git(project.path(), &["config", "user.name", "E2E"]);
    project.write("README.md", "worktree e2e\n");
    git(project.path(), &["add", "."]);
    git(project.path(), &["commit", "-qm", "initial"]);

    let started = support::start_engine(&project);
    let deadline = Instant::now() + Duration::from_secs(90);
    support::wait_for_stream(&started.events, deadline);

    // The worktree endpoints need the server-side project record, which a
    // session materializes.
    create_session(&started, &started.directory, deadline);
    // The server lists the project itself in the worktree collection; only
    // real worktrees count.
    let initial = load_worktrees(&started, deadline);
    assert!(
        initial
            .iter()
            .all(|entry| entry.directory.trim_end_matches('/')
                == started.directory.trim_end_matches('/')),
        "unexpected worktrees before create: {initial:?}"
    );

    assert!(
        started
            .engine
            .create_worktree(&started.directory, &started.directory, "", "feature"),
        "engine stopped before the create request"
    );
    let EngineEvent::WorktreeCreated { worktree } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::WorktreeCreated { .. })
        })
    else {
        unreachable!("predicate only accepts WorktreeCreated");
    };
    let mut cleanup = Cleanup(Some(worktree.directory.clone()));

    assert!(
        Path::new(&worktree.directory).is_dir(),
        "the server-reported directory must exist: {}",
        worktree.directory
    );
    let listed = git(project.path(), &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&worktree.directory),
        "git worktree list must include {}:\n{listed}",
        worktree.directory
    );
    // The server's strategy checks out a detached copy of the base ref.
    assert_eq!(
        git(Path::new(&worktree.directory), &["rev-parse", "HEAD"]),
        git(project.path(), &["rev-parse", "HEAD"]),
        "the worktree must start from the project HEAD"
    );

    let loaded = load_worktrees(&started, deadline);
    assert!(
        loaded
            .iter()
            .any(|entry| entry.directory == worktree.directory),
        "the server list must include {:?}: {loaded:?}",
        worktree.directory
    );

    let worktree_directory = std::fs::canonicalize(&worktree.directory)
        .expect("canonical worktree")
        .to_string_lossy()
        .into_owned();
    let session = create_session(&started, &worktree_directory, deadline);
    assert_eq!(
        session
            .location
            .as_ref()
            .map(|location| location.directory.as_str()),
        Some(worktree_directory.as_str())
    );

    // The server scopes listings by directory; this is the same query the
    // sidebar uses to group sessions under a worktree.
    assert!(started.engine.refresh_sessions(&worktree_directory));
    let EngineEvent::SessionsLoaded { sessions } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::SessionsLoaded { .. })
        })
    else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec![session.id.as_str()],
        "a worktree query must not see the project root's sessions"
    );

    assert!(
        started
            .engine
            .list_sessions_across(&started.directory, vec![worktree_directory.clone()],)
    );
    let EngineEvent::SessionsLoaded { sessions } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::SessionsLoaded { .. })
        })
    else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    let mut ids = sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "both directories must contribute sessions");
    assert!(ids.contains(&session.id.as_str()));

    assert!(
        started
            .engine
            .remove_worktree(&started.directory, &worktree.directory, true),
        "engine stopped before the remove request"
    );
    let EngineEvent::WorktreeRemoved { directory } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::WorktreeRemoved { .. })
        })
    else {
        unreachable!("predicate only accepts WorktreeRemoved");
    };
    assert_eq!(directory, worktree.directory);
    cleanup.0 = None;

    assert!(
        !Path::new(&worktree.directory).exists(),
        "the worktree directory must be gone: {}",
        worktree.directory
    );
    let listed = git(project.path(), &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(&worktree.directory),
        "git must forget the worktree:\n{listed}"
    );

    let remaining = load_worktrees(&started, deadline);
    assert!(
        remaining
            .iter()
            .all(|entry| entry.directory != worktree.directory),
        "the server list must forget the removed worktree: {remaining:?}"
    );

    support::shutdown(started.engine, started.port);
}
