//! M11-03 — one prompt across three models, each in its own worktree.
//!
//! Gated on `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`, matching the other model
//! tests. Assertions are on the filesystem, the server's worktree list, and the
//! sessions the runs created, not on local events alone.

mod support;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use harness_core::{Run, RunCleanup, RunGroup, RunState};
use harness_engine::{EngineEvent, ModelRef, RunDiscard};
use harness_protocol::{AssistantPart, SessionMessage};
use support::mock::{MockProvider, Script};

fn git(root: &Path, args: &[&str]) {
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
}

/// Three models on the same mock provider. The worktree is a checkout of the
/// project HEAD, so this config must be committed before the runs start.
fn provider_config(base_url: &str) -> String {
    format!(
        r#"{{
            "$schema": "https://opencode.ai/config.json",
            "model": "mock/mock-a",
            "provider": {{
                "mock": {{
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "Mock",
                    "options": {{ "baseURL": "{base_url}", "apiKey": "test-key" }},
                    "models": {{
                        "mock-a": {{ "name": "Mock A" }},
                        "mock-b": {{ "name": "Mock B" }},
                        "mock-c": {{ "name": "Mock C" }}
                    }}
                }}
            }}
        }}"#
    )
}

fn model(suffix: &str) -> ModelRef {
    ModelRef {
        id: format!("mock-{suffix}"),
        provider_id: Some("mock".into()),
        variant: None,
    }
}

fn group() -> RunGroup {
    RunGroup {
        id: "runs-e2e-1".into(),
        prompt: "Compare the approaches".into(),
        created: 1,
        runs: ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(index, suffix)| Run {
                id: format!("run-{}", index + 1),
                model: model(suffix),
                session_id: None,
                worktree: None,
                state: RunState::Pending,
            })
            .collect(),
        kept: None,
    }
}

fn load_worktrees(
    started: &support::Started,
    deadline: Instant,
) -> Vec<harness_protocol::Worktree> {
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

fn assistant_texts(
    engine: &harness_engine::Engine,
    events: &async_channel::Receiver<EngineEvent>,
    directory: &str,
    session_id: &str,
    deadline: Instant,
) -> Vec<String> {
    assert!(engine.load_transcript(directory, session_id));
    let EngineEvent::TranscriptLoaded { messages, .. } =
        support::wait_for(events, deadline, |event| {
            matches!(
                event,
                EngineEvent::TranscriptLoaded { session_id: loaded, .. } if loaded == session_id
            )
        })
    else {
        unreachable!("predicate only accepts TranscriptLoaded");
    };
    messages
        .iter()
        .filter_map(|message| {
            let SessionMessage::Assistant { content, .. } = message else {
                return None;
            };
            let text = content
                .iter()
                .filter_map(|part| match part {
                    AssistantPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            Some(text)
        })
        .collect()
}

/// A deleted session answers with a command failure rather than a transcript.
fn assert_session_gone(
    engine: &harness_engine::Engine,
    events: &async_channel::Receiver<EngineEvent>,
    directory: &str,
    session_id: &str,
    deadline: Instant,
) {
    assert!(engine.load_transcript(directory, session_id));
    let EngineEvent::CommandFailed { message } = support::wait_for(events, deadline, |event| {
        matches!(
            event,
            EngineEvent::CommandFailed { message }
                if message.starts_with("Could not load the conversation")
        )
    }) else {
        unreachable!("predicate only accepts CommandFailed");
    };
    assert!(!message.is_empty());
}

/// Run the group to completion and return the last state of every run,
/// keyed by run id: `(state, session id, worktree directory)`.
type FinalState = HashMap<String, (RunState, Option<String>, Option<String>)>;

fn drive_group(started: &support::Started, group: &RunGroup, deadline: Instant) -> FinalState {
    assert!(
        started
            .engine
            .start_run_group(&started.directory, group.clone())
    );
    let EngineEvent::RunGroupStarted { group: accepted } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::RunGroupStarted { .. })
        })
    else {
        unreachable!("predicate only accepts RunGroupStarted");
    };
    assert_eq!(accepted.id, group.id);

    let mut states: FinalState = HashMap::new();
    loop {
        let event = support::wait_for(&started.events, deadline, |event| {
            matches!(
                event,
                EngineEvent::RunStateChanged { .. } | EngineEvent::RunGroupFinished { .. }
            )
        });
        match event {
            EngineEvent::RunStateChanged {
                group_id,
                run_id,
                state,
                session_id,
                worktree,
            } => {
                assert_eq!(group_id, group.id);
                states.insert(run_id, (state, session_id, worktree));
            }
            EngineEvent::RunGroupFinished { group_id } => {
                assert_eq!(group_id, group.id);
                break;
            }
            _ => unreachable!("predicate only accepts run events"),
        }
    }
    states
}

#[test]
fn three_models_run_in_parallel_and_cleans_up_each_run() {
    if support::skip_e2e() {
        eprintln!("skipping multi-run e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping multi-run e2e test: opencode was not found");
        return;
    }
    if std::env::var_os("RUST_HARNESS_MOCK_SCRIPTED_PROVIDER").is_none() {
        eprintln!("skipping multi-run e2e test: RUST_HARNESS_MOCK_SCRIPTED_PROVIDER is not set");
        return;
    }
    if Command::new("git")
        .arg("--version")
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping multi-run e2e test: git was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("run complete"));
    let project = support::project("multi-run");
    project.write("opencode.json", &provider_config(&mock.base_url()));
    project.write("README.md", "multi-run e2e\n");
    git(project.path(), &["init", "-q"]);
    git(project.path(), &["config", "user.email", "e2e@example.com"]);
    git(project.path(), &["config", "user.name", "E2E"]);
    git(project.path(), &["add", "."]);
    git(project.path(), &["commit", "-qm", "initial"]);

    let started = support::start_engine(&project);
    let deadline = Instant::now() + Duration::from_secs(240);
    support::wait_for_stream(&started.events, deadline);

    // The worktree endpoints need the server-side project record, which a
    // session in the root checkout materializes.
    assert!(started.engine.create_session(&started.directory));
    let EngineEvent::SessionCreated {
        session: root_session,
    } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    })
    else {
        unreachable!("predicate only accepts SessionCreated");
    };

    let group = group();
    let states = drive_group(&started, &group, deadline);

    assert_eq!(states.len(), 3, "every run must report a final state");
    let mut sessions = Vec::new();
    let mut worktrees = Vec::new();
    for run in &group.runs {
        let (state, session, worktree) = states
            .get(&run.id)
            .unwrap_or_else(|| panic!("run {} reported no state", run.id));
        assert_eq!(state, &RunState::Done, "run {} must finish", run.id);
        let session = session
            .clone()
            .unwrap_or_else(|| panic!("run {} has no session", run.id));
        let worktree = worktree
            .clone()
            .unwrap_or_else(|| panic!("run {} has no worktree", run.id));
        assert!(
            Path::new(&worktree).is_dir(),
            "the run worktree must exist on disk: {worktree}"
        );
        sessions.push(session);
        worktrees.push(worktree);
    }
    sessions.sort_unstable();
    sessions.dedup();
    worktrees.sort_unstable();
    worktrees.dedup();
    assert_eq!(sessions.len(), 3, "every run gets its own session");
    assert_eq!(worktrees.len(), 3, "every run gets its own worktree");

    // Durable effects: each session holds the mock's reply, and the server
    // lists all three worktrees next to the project root.
    for (session, worktree) in sessions.iter().zip(worktrees.iter()) {
        let texts = assistant_texts(
            &started.engine,
            &started.events,
            worktree,
            session,
            deadline,
        );
        assert_eq!(texts, vec!["run complete".to_string()]);
    }
    let listed = load_worktrees(&started, deadline);
    for worktree in &worktrees {
        assert!(
            listed.iter().any(|entry| &entry.directory == worktree),
            "the server list must include {worktree}: {listed:?}"
        );
    }

    // Discard one run: only its worktree and session disappear.
    let discarded = group.runs[0].id.clone();
    let (_, discarded_session, discarded_worktree) =
        states.get(&discarded).cloned().expect("state");
    let discarded_session = discarded_session.expect("session");
    let discarded_worktree = discarded_worktree.expect("worktree");
    assert!(
        started.engine.discard_run(
            &started.directory,
            &group.id,
            &discarded,
            RunCleanup {
                session_id: Some(discarded_session.clone()),
                worktree: Some(discarded_worktree.clone()),
            },
        ),
        "engine stopped before the discard"
    );
    let EngineEvent::RunStateChanged {
        run_id: discarded_run,
        state: discarded_state,
        ..
    } = support::wait_for(&started.events, deadline, |event| {
        matches!(
            event,
            EngineEvent::RunStateChanged { run_id, state: RunState::Discarded, .. }
                if run_id == &discarded
        )
    })
    else {
        unreachable!("predicate only accepts RunStateChanged");
    };
    assert_eq!(discarded_run, discarded);
    assert_eq!(discarded_state, RunState::Discarded);
    assert!(
        !Path::new(&discarded_worktree).exists(),
        "the discarded worktree must be gone: {discarded_worktree}"
    );
    let listed = load_worktrees(&started, deadline);
    assert!(
        listed
            .iter()
            .all(|entry| entry.directory != discarded_worktree),
        "the server list must forget the discarded worktree: {listed:?}"
    );
    assert_session_gone(
        &started.engine,
        &started.events,
        &started.directory,
        &discarded_session,
        deadline,
    );
    for worktree in worktrees
        .iter()
        .filter(|worktree| worktree.as_str() != discarded_worktree)
    {
        assert!(
            Path::new(worktree).is_dir(),
            "the other runs must keep their worktree: {worktree}"
        );
    }

    // Discard the group: every remaining run's resources go with it.
    let remaining: Vec<RunDiscard> = group
        .runs
        .iter()
        .filter(|run| run.id != discarded)
        .map(|run| {
            let (_, session, worktree) = states.get(&run.id).cloned().expect("state");
            RunDiscard {
                run_id: run.id.clone(),
                cleanup: RunCleanup {
                    session_id: session,
                    worktree,
                },
            }
        })
        .collect();
    assert!(
        started
            .engine
            .discard_run_group(&started.directory, &group.id, remaining),
        "engine stopped before the group discard"
    );
    let EngineEvent::RunGroupFinished { group_id } =
        support::wait_for(&started.events, deadline, |event| {
            matches!(event, EngineEvent::RunGroupFinished { .. })
        })
    else {
        unreachable!("predicate only accepts RunGroupFinished");
    };
    assert_eq!(group_id, group.id);

    for worktree in &worktrees {
        assert!(
            !Path::new(worktree).exists(),
            "the group discard must remove every worktree: {worktree}"
        );
    }
    let listed = load_worktrees(&started, deadline);
    assert!(
        listed
            .iter()
            .all(|entry| entry.directory == started.directory),
        "only the project root may remain: {listed:?}"
    );
    for run in group.runs.iter().filter(|run| run.id != discarded) {
        let (_, session, _) = states.get(&run.id).cloned().expect("state");
        assert_session_gone(
            &started.engine,
            &started.events,
            &started.directory,
            &session.expect("session"),
            deadline,
        );
    }

    // The group's cleanup must not touch sessions it did not create.
    assert!(started.engine.refresh_sessions(&started.directory));
    let EngineEvent::SessionsLoaded {
        sessions: remaining_sessions,
    } = support::wait_for(&started.events, deadline, |event| {
        matches!(event, EngineEvent::SessionsLoaded { .. })
    })
    else {
        unreachable!("predicate only accepts SessionsLoaded");
    };
    assert!(
        remaining_sessions
            .iter()
            .any(|session| session.id == root_session.id),
        "the root session must survive: {remaining_sessions:?}"
    );

    support::shutdown(started.engine, started.port);
}
