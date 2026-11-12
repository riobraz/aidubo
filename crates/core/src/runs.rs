//! Multi-run groups: one prompt sent to several models in isolated worktrees.
//!
//! The group is app-local. Every rule that decides what may be kept, discarded
//! or interrupted lives here so the engine only executes cleanups and the view
//! only renders state.

use harness_protocol::ModelRef;
use serde::{Deserialize, Serialize};

/// The engine refuses to start more than this many runs in one group.
pub const MAX_RUNS_PER_GROUP: usize = 5;

/// Why a run stopped after a relaunch; also the message the UI shows.
pub const INTERRUPTED: &str = "Interrupted by relaunch";

/// Where one run of a group is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    Pending,
    Starting,
    Running,
    Done,
    Failed { message: String },
    Discarded,
}

impl RunState {
    /// Terminal states never change again on their own.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. } | Self::Discarded)
    }

    /// States a relaunch must not resume.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Pending | Self::Starting | Self::Running)
    }
}

/// One model's attempt at the group's prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub model: ModelRef,
    pub session_id: Option<String>,
    pub worktree: Option<String>,
    pub state: RunState,
}

/// One prompt fanned out to several runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunGroup {
    pub id: String,
    pub prompt: String,
    pub created: i64,
    pub runs: Vec<Run>,
    /// The run whose worktree and session survive a discard. Exclusive.
    pub kept: Option<String>,
}

/// What the engine must delete when a run leaves the group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunCleanup {
    pub session_id: Option<String>,
    pub worktree: Option<String>,
}

impl RunCleanup {
    pub fn is_empty(&self) -> bool {
        self.session_id.is_none() && self.worktree.is_none()
    }
}

/// Every group of the active project.
#[derive(Debug, Clone, Default)]
pub struct RunGroups {
    directory: Option<String>,
    groups: Vec<RunGroup>,
}

impl RunGroups {
    /// Point the store at a project. Switching projects clears the rows; the
    /// app-local file keeps each project's groups.
    pub fn begin(&mut self, directory: impl Into<String>) -> bool {
        let directory = directory.into();
        if self.directory.as_deref() == Some(directory.as_str()) {
            return false;
        }
        self.directory = Some(directory);
        self.groups.clear();
        true
    }

    pub fn directory(&self) -> Option<&str> {
        self.directory.as_deref()
    }

    pub fn groups(&self) -> &[RunGroup] {
        &self.groups
    }

    pub fn group(&self, id: &str) -> Option<&RunGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Restore a project's saved groups. Live runs become `Failed` with
    /// [`INTERRUPTED`]; nothing resumes.
    pub fn restore(&mut self, directory: impl Into<String>, groups: Vec<RunGroup>) {
        self.directory = Some(directory.into());
        self.groups = groups
            .into_iter()
            .map(|mut group| {
                for run in &mut group.runs {
                    if run.state.is_live() {
                        run.state = RunState::Failed {
                            message: INTERRUPTED.to_string(),
                        };
                    }
                }
                group
            })
            .collect();
    }

    /// Register a new group. The engine starts its runs after this succeeds.
    pub fn start(&mut self, group: RunGroup) -> Result<(), String> {
        if group.runs.is_empty() {
            return Err("A run group needs at least one model.".to_string());
        }
        if group.runs.len() > MAX_RUNS_PER_GROUP {
            return Err(format!("At most {MAX_RUNS_PER_GROUP} models per group."));
        }
        if self.groups.iter().any(|existing| existing.id == group.id) {
            return Err("That run group already exists.".to_string());
        }
        let mut seen = std::collections::HashSet::new();
        if group.runs.iter().any(|run| !seen.insert(&run.id)) {
            return Err("Run ids must be unique.".to_string());
        }
        self.groups.push(group);
        Ok(())
    }

    /// The single mutation path for run state. Unknown ids are ignored so a
    /// late event cannot resurrect a group the user already closed, and a
    /// terminal state never changes again.
    pub fn set_state(&mut self, group_id: &str, run_id: &str, state: RunState) -> bool {
        let Some(run) = self.run_mut(group_id, run_id) else {
            return false;
        };
        if run.state == state || (run.state.is_finished() && state != RunState::Discarded) {
            return false;
        }
        run.state = state;
        true
    }

    pub fn set_session(&mut self, group_id: &str, run_id: &str, session_id: String) -> bool {
        match self.run_mut(group_id, run_id) {
            Some(run) => {
                run.session_id = Some(session_id);
                true
            }
            None => false,
        }
    }

    pub fn set_worktree(&mut self, group_id: &str, run_id: &str, worktree: String) -> bool {
        match self.run_mut(group_id, run_id) {
            Some(run) => {
                run.worktree = Some(worktree);
                true
            }
            None => false,
        }
    }

    /// Mark one run as the winner. Keeping a second run replaces the first.
    pub fn keep(&mut self, group_id: &str, run_id: &str) -> Result<(), String> {
        let Some(group) = self.groups.iter_mut().find(|group| group.id == group_id) else {
            return Err("That run group is gone.".to_string());
        };
        let Some(run) = group.runs.iter().find(|run| run.id == run_id) else {
            return Err("That run is gone.".to_string());
        };
        if run.state == RunState::Discarded {
            return Err("That run was discarded.".to_string());
        }
        group.kept = Some(run_id.to_string());
        Ok(())
    }

    /// Mark one run discarded and report what must be deleted on the server.
    pub fn discard_run(&mut self, group_id: &str, run_id: &str) -> Option<RunCleanup> {
        let group = self.groups.iter_mut().find(|group| group.id == group_id)?;
        if group.kept.as_deref() == Some(run_id) {
            group.kept = None;
        }
        let run = group.runs.iter_mut().find(|run| run.id == run_id)?;
        if run.state == RunState::Discarded {
            return None;
        }
        run.state = RunState::Discarded;
        let cleanup = cleanup_for(run);
        if cleanup.is_empty() {
            None
        } else {
            Some(cleanup)
        }
    }

    /// Drop a whole group. The kept run keeps its session and worktree; every
    /// other run that still holds server resources is reported for cleanup.
    pub fn discard_group(&mut self, group_id: &str) -> Vec<RunCleanup> {
        let Some(index) = self.groups.iter().position(|group| group.id == group_id) else {
            return Vec::new();
        };
        let group = self.groups.remove(index);
        group
            .runs
            .iter()
            .filter(|run| group.kept.as_deref() != Some(run.id.as_str()))
            .filter(|run| run.state != RunState::Discarded)
            .map(cleanup_for)
            .filter(|cleanup| !cleanup.is_empty())
            .collect()
    }

    fn run_mut(&mut self, group_id: &str, run_id: &str) -> Option<&mut Run> {
        self.groups
            .iter_mut()
            .find(|group| group.id == group_id)?
            .runs
            .iter_mut()
            .find(|run| run.id == run_id)
    }
}

fn cleanup_for(run: &Run) -> RunCleanup {
    RunCleanup {
        session_id: run.session_id.clone(),
        worktree: run.worktree.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelRef {
        ModelRef {
            id: id.to_string(),
            provider_id: Some("mock".to_string()),
            variant: None,
        }
    }

    fn run(id: &str) -> Run {
        Run {
            id: id.to_string(),
            model: model(id),
            session_id: None,
            worktree: None,
            state: RunState::Pending,
        }
    }

    fn group(runs: &[&str]) -> RunGroup {
        RunGroup {
            id: "group-1".to_string(),
            prompt: "Explain the bug".to_string(),
            created: 1,
            runs: runs.iter().map(|id| run(id)).collect(),
            kept: None,
        }
    }

    #[test]
    fn run_group_partial_failure_keeps_others() {
        let mut groups = RunGroups::default();
        groups.start(group(&["a", "b", "c"])).expect("start");
        assert!(groups.set_state(
            "group-1",
            "b",
            RunState::Failed {
                message: "worktree failed".to_string()
            }
        ));

        let group = groups.group("group-1").expect("group");
        assert_eq!(
            group.runs[1].state,
            RunState::Failed {
                message: "worktree failed".to_string()
            }
        );
        assert_eq!(group.runs[0].state, RunState::Pending);
        assert_eq!(group.runs[2].state, RunState::Pending);
        assert!(groups.set_state("group-1", "a", RunState::Running));
        assert!(!groups.set_state("group-1", "b", RunState::Running));
    }

    #[test]
    fn run_group_keep_is_exclusive() {
        let mut groups = RunGroups::default();
        groups.start(group(&["a", "b"])).expect("start");
        groups.keep("group-1", "a").expect("keep a");
        assert_eq!(
            groups.group("group-1").and_then(|group| group.kept.clone()),
            Some("a".to_string())
        );
        groups.keep("group-1", "b").expect("keep b");
        assert_eq!(
            groups.group("group-1").and_then(|group| group.kept.clone()),
            Some("b".to_string())
        );
        assert!(groups.keep("group-1", "missing").is_err());
        assert!(groups.keep("missing", "a").is_err());
    }

    #[test]
    fn run_group_relaunch_marks_interrupted() {
        let mut saved = group(&["a", "b", "c"]);
        saved.runs[0].state = RunState::Running;
        saved.runs[1].state = RunState::Done;
        saved.runs[2].state = RunState::Pending;

        let mut groups = RunGroups::default();
        groups.restore("/tmp/project", vec![saved]);
        let group = groups.group("group-1").expect("group");
        assert_eq!(
            group.runs[0].state,
            RunState::Failed {
                message: INTERRUPTED.to_string()
            }
        );
        assert_eq!(group.runs[1].state, RunState::Done);
        assert_eq!(
            group.runs[2].state,
            RunState::Failed {
                message: INTERRUPTED.to_string()
            }
        );
        assert!(group.runs.iter().all(|run| !run.state.is_live()));
        assert_eq!(groups.directory(), Some("/tmp/project"));
    }

    #[test]
    fn run_group_discard_cleans_existing_only() {
        let mut saved = group(&["a", "b"]);
        saved.runs[0].session_id = Some("ses_a".to_string());
        saved.runs[0].worktree = Some("/tmp/wt-a".to_string());
        saved.kept = Some("b".to_string());

        let mut groups = RunGroups::default();
        groups.start(saved).expect("start");

        let cleanup = groups.discard_run("group-1", "a").expect("cleanup");
        assert_eq!(
            cleanup,
            RunCleanup {
                session_id: Some("ses_a".to_string()),
                worktree: Some("/tmp/wt-a".to_string()),
            }
        );
        assert_eq!(groups.discard_run("group-1", "a"), None);
        assert_eq!(groups.discard_run("group-1", "b"), None);
        assert!(
            groups
                .group("group-1")
                .is_some_and(|group| group.kept.is_none())
        );

        let mut saved = group(&["a", "b"]);
        saved.runs[0].session_id = Some("ses_a".to_string());
        saved.runs[0].worktree = Some("/tmp/wt-a".to_string());
        saved.runs[1].session_id = Some("ses_b".to_string());
        saved.runs[1].worktree = Some("/tmp/wt-b".to_string());
        saved.kept = Some("b".to_string());
        let mut groups = RunGroups::default();
        groups.start(saved).expect("start");

        let cleanups = groups.discard_group("group-1");
        assert_eq!(cleanups.len(), 1);
        assert_eq!(cleanups[0].session_id.as_deref(), Some("ses_a"));
        assert_eq!(cleanups[0].worktree.as_deref(), Some("/tmp/wt-a"));
        assert!(groups.group("group-1").is_none());
        assert!(groups.discard_group("group-1").is_empty());
    }
}
