//! M11-01 — multiple projects and location routing.
//!
//! Creates a session in each of two temp directories, then proves the server
//! scopes listings by `directory`: each project sees only its own session.
//! Restarting the engine re-reads both from the server, so the assertion is
//! durable rather than a local cache.

mod support;

use std::time::{Duration, Instant};

use harness_engine::{EngineEvent, ProjectId};
use harness_protocol::Session;
use support::{Started, TestProject};

fn canonical(project: &TestProject) -> String {
    std::fs::canonicalize(project.path())
        .expect("canonical project")
        .to_string_lossy()
        .into_owned()
}

fn select(started: &Started, id: &str, deadline: Instant) -> Vec<Session> {
    assert!(started.engine.select_project(id, ProjectId::from(id)));
    let EngineEvent::ProjectSelected { canonical, .. } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::ProjectSelected { .. })
        })
    else {
        unreachable!("predicate only accepts ProjectSelected");
    };
    assert_eq!(canonical, id, "the selected directory is canonical");
    let EngineEvent::SessionsLoaded { sessions } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::SessionsLoaded { .. })
        })
    else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    sessions
}

fn create_session(started: &Started, directory: &str, deadline: Instant) -> Session {
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

#[test]
fn projects_scope_sessions_by_directory() {
    if support::skip_e2e() {
        eprintln!("skipping project routing e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping project routing e2e test: opencode was not found");
        return;
    }

    let first = support::project("projects-first");
    let second = support::project("projects-second");
    let first_dir = canonical(&first);
    let second_dir = canonical(&second);

    let started = support::start_engine(&first);
    support::wait_for_stream(&started.events, Instant::now() + Duration::from_secs(30));

    // A symlinked path addresses the same folder, but the server compares
    // canonical strings, so the supervisor must reject it explicitly.
    #[cfg(unix)]
    {
        let link =
            std::env::temp_dir().join(format!("rust-harness-project-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&first_dir, &link).expect("create symlink");
        assert!(started.engine.refresh_sessions(link.to_string_lossy()));
        let deadline = Instant::now() + Duration::from_secs(30);
        let EngineEvent::CommandFailed { message } =
            support::wait_for(&started.events, deadline, |event| {
                matches!(event, EngineEvent::CommandFailed { .. })
            })
        else {
            unreachable!("predicate only accepts CommandFailed")
        };
        assert!(message.contains("must be canonical"), "{message}");
        assert!(message.contains(&first_dir), "{message}");
        let _ = std::fs::remove_file(&link);
    }

    let first_session = create_session(
        &started,
        &first_dir,
        Instant::now() + Duration::from_secs(30),
    );
    if let Some(location) = &first_session.location {
        assert_eq!(location.directory, first_dir);
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    assert!(
        started
            .engine
            .select_project(&second_dir, ProjectId::from(second_dir.as_str()))
    );
    let EngineEvent::ProjectSelected { .. } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::ProjectSelected { .. })
        })
    else {
        unreachable!()
    };
    let second_session = create_session(&started, &second_dir, deadline);
    if let Some(location) = &second_session.location {
        assert_eq!(location.directory, second_dir);
    }

    let first_ids = select(&started, &first_dir, deadline)
        .into_iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    assert_eq!(
        first_ids,
        vec![first_session.id.clone()],
        "the first project must not list the second project's session"
    );

    let second_ids = select(&started, &second_dir, deadline)
        .into_iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    assert_eq!(
        second_ids,
        vec![second_session.id.clone()],
        "the second project must not list the first project's session"
    );

    // The server materialized both directories as projects when sessions were
    // created for them.
    assert!(started.engine.list_projects(&first_dir));
    let EngineEvent::ProjectsLoaded { result } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::ProjectsLoaded { .. })
        })
    else {
        unreachable!()
    };
    let projects = result.expect("project list");
    let canonicals = projects
        .iter()
        .map(|project| project.canonical.as_str())
        .collect::<Vec<_>>();
    assert!(canonicals.contains(&first_dir.as_str()), "{canonicals:?}");
    assert!(canonicals.contains(&second_dir.as_str()), "{canonicals:?}");

    support::shutdown(started.engine, started.port);

    // Restart: the session-to-project association lives on the server.
    let restarted = support::start_engine(&first);
    let deadline = Instant::now() + Duration::from_secs(30);
    let EngineEvent::SessionsLoaded { sessions } =
        support::wait_for(&restarted.events, deadline, |event| {
            matches!(event, EngineEvent::SessionsLoaded { .. })
        })
    else {
        unreachable!()
    };
    assert_eq!(
        sessions
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        vec![first_session.id.clone()]
    );

    let second_ids = select(&restarted, &second_dir, deadline)
        .into_iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    assert_eq!(second_ids, vec![second_session.id.clone()]);
    support::shutdown(restarted.engine, restarted.port);
}
