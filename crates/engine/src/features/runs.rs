//! Multi-run groups: one prompt, one worktree and session per model.
//!
//! Every rule about what may be kept or discarded lives in
//! `harness_core::RunGroups`; this runtime executes the decisions it is handed
//! and reports state back to the view. Runs are sequential per run and
//! parallel across runs, capped by [`MAX_RUNS_PER_GROUP`] permits.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_channel::Sender;
use harness_core::{MAX_RUNS_PER_GROUP, Run, RunCleanup, RunGroup, RunState};
use harness_protocol::{
    CreateWorktreeRequest, ExecutionOutcome, PromptRequest, RemoveWorktreeRequest, StreamEvent,
    Worktree,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::api::SessionApi;
use crate::supervisor::EngineEvent;

/// One run the view asked to discard, with the server resources it still
/// holds as the model knows them.
#[derive(Debug, Clone, Default)]
pub struct RunDiscard {
    pub run_id: String,
    pub cleanup: RunCleanup,
}

/// Work sent to the run runtime by the supervisor.
#[derive(Debug)]
pub(crate) enum RunCommand {
    /// Start every run in `group`. `from` is the project root checkout.
    Start {
        group: RunGroup,
        project_id: String,
        from: String,
    },
    /// Stop and clean up one run. `cleanup` comes from the model; the runtime
    /// merges it with whatever resources it assigned itself.
    DiscardRun {
        project_id: String,
        group_id: String,
        run_id: String,
        cleanup: RunCleanup,
    },
    /// Mark one run as the group's keeper.
    Keep { group_id: String, run_id: String },
    /// Drop a group and everything it still owns.
    DiscardGroup {
        project_id: String,
        group_id: String,
        runs: Vec<RunDiscard>,
    },
    /// The event stream reconnected; verify live sessions still exist.
    Verify,
    /// Internal: a run task moved to a new state or gained a resource.
    Progress {
        group_id: String,
        run_id: String,
        state: RunState,
        session_id: Option<String>,
        worktree: Option<String>,
    },
    /// Internal: a run task prompted its session and awaits the outcome.
    Awaiting {
        group_id: String,
        run_id: String,
        session_id: String,
        done: oneshot::Sender<ExecutionOutcome>,
    },
}

/// The five-permit gate that keeps a group from opening more concurrent runs
/// than the product allows.
pub(crate) struct RunGate {
    permits: Arc<tokio::sync::Semaphore>,
}

impl RunGate {
    pub(crate) fn new() -> Self {
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_RUNS_PER_GROUP)),
        }
    }

    async fn acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.permits.clone().acquire_owned().await.ok()
    }
}

struct RunHandle {
    state: RunState,
    session_id: Option<String>,
    worktree: Option<String>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct GroupHandle {
    runs: HashMap<String, RunHandle>,
    kept: Option<String>,
    finished: bool,
}

pub(crate) struct RunGroupRuntime {
    api: SessionApi,
    events: Sender<EngineEvent>,
    commands: tokio::sync::mpsc::UnboundedReceiver<RunCommand>,
    self_tx: UnboundedSender<RunCommand>,
    stream: tokio::sync::mpsc::UnboundedReceiver<StreamEvent>,
    gate: RunGate,
    groups: HashMap<String, GroupHandle>,
    /// Session id -> the task waiting for that turn's outcome.
    waiting: HashMap<String, oneshot::Sender<ExecutionOutcome>>,
}

impl RunGroupRuntime {
    pub(crate) fn new(
        api: SessionApi,
        events: Sender<EngineEvent>,
        self_tx: UnboundedSender<RunCommand>,
        commands: tokio::sync::mpsc::UnboundedReceiver<RunCommand>,
        stream: tokio::sync::mpsc::UnboundedReceiver<StreamEvent>,
    ) -> Self {
        Self {
            api,
            events,
            commands,
            self_tx,
            stream,
            gate: RunGate::new(),
            groups: HashMap::new(),
            waiting: HashMap::new(),
        }
    }

    pub(crate) async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else { break };
                    self.handle_command(command).await;
                }
                event = self.stream.recv() => {
                    let Some(event) = event else { break };
                    self.handle_stream(event);
                }
            }
        }
    }

    async fn handle_command(&mut self, command: RunCommand) {
        match command {
            RunCommand::Start {
                group,
                project_id,
                from,
            } => self.start(group, project_id, from).await,
            RunCommand::DiscardRun {
                project_id,
                group_id,
                run_id,
                cleanup,
            } => {
                self.discard_run(project_id, group_id, run_id, cleanup)
                    .await
            }
            RunCommand::Keep { group_id, run_id } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group.kept = Some(run_id);
                }
            }
            RunCommand::DiscardGroup {
                project_id,
                group_id,
                runs,
            } => self.discard_group(project_id, group_id, runs).await,
            RunCommand::Verify => self.verify_live_sessions(),
            RunCommand::Progress {
                group_id,
                run_id,
                state,
                session_id,
                worktree,
            } => {
                self.progress(group_id, run_id, state, session_id, worktree)
                    .await
            }
            RunCommand::Awaiting {
                group_id,
                run_id,
                session_id,
                done,
            } => {
                if self.is_discarded(&group_id, &run_id) {
                    return;
                }
                self.waiting.insert(session_id, done);
            }
        }
    }

    fn handle_stream(&mut self, event: StreamEvent) {
        if let StreamEvent::ExecutionFinished {
            session_id,
            outcome,
            ..
        } = event
            && let Some(done) = self.waiting.remove(&session_id)
            && done.send(outcome).is_err()
        {
            tracing::debug!(session = %session_id, "the run task is gone");
        }
    }

    fn is_discarded(&self, group_id: &str, run_id: &str) -> bool {
        self.groups
            .get(group_id)
            .and_then(|group| group.runs.get(run_id))
            .is_some_and(|run| run.state == RunState::Discarded)
    }

    async fn start(&mut self, group: RunGroup, project_id: String, from: String) {
        if self.groups.contains_key(&group.id) {
            return;
        }
        let mut handle = GroupHandle::default();
        for run in &group.runs {
            handle.runs.insert(
                run.id.clone(),
                RunHandle {
                    state: RunState::Pending,
                    session_id: None,
                    worktree: None,
                    cancel: Arc::new(AtomicBool::new(false)),
                },
            );
        }
        self.groups.insert(group.id.clone(), handle);
        if self
            .events
            .send(EngineEvent::RunGroupStarted {
                group: group.clone(),
            })
            .await
            .is_err()
        {
            tracing::debug!("the view stopped listening; dropping a run group");
        }

        for run in group.runs.clone() {
            let cancel = self
                .groups
                .get(&group.id)
                .and_then(|group| group.runs.get(&run.id))
                .map(|handle| handle.cancel.clone())
                .unwrap_or_default();
            let task = RunTask {
                api: self.api.clone(),
                commands: self.self_tx.clone(),
                gate: RunGate {
                    permits: self.gate.permits.clone(),
                },
                project_id: project_id.clone(),
                from: from.clone(),
                group_id: group.id.clone(),
                prompt: group.prompt.clone(),
                cancel,
                run,
            };
            tokio::spawn(task.run());
        }
    }

    async fn progress(
        &mut self,
        group_id: String,
        run_id: String,
        state: RunState,
        session_id: Option<String>,
        worktree: Option<String>,
    ) {
        let Some(run) = self
            .groups
            .get_mut(&group_id)
            .and_then(|group| group.runs.get_mut(&run_id))
        else {
            return;
        };
        // A discarded run never comes back, even if a late event arrives.
        if run.state == RunState::Discarded {
            return;
        }
        run.state = state.clone();
        run.session_id = session_id.clone();
        run.worktree = worktree.clone();
        let event = EngineEvent::RunStateChanged {
            group_id: group_id.clone(),
            run_id: run_id.clone(),
            state,
            session_id,
            worktree,
        };
        if self.events.send(event).await.is_err() {
            tracing::debug!("the view stopped listening; dropping a run update");
        }
        self.maybe_finish(&group_id).await;
    }

    async fn maybe_finish(&mut self, group_id: &str) {
        let Some(group) = self.groups.get_mut(group_id) else {
            return;
        };
        if group.finished || group.runs.is_empty() {
            return;
        }
        if group.runs.values().all(|run| run.state.is_finished()) {
            group.finished = true;
            let event = EngineEvent::RunGroupFinished {
                group_id: group_id.to_string(),
            };
            if self.events.send(event).await.is_err() {
                tracing::debug!("the view stopped listening; dropping a group update");
            }
        }
    }

    async fn discard_run(
        &mut self,
        project_id: String,
        group_id: String,
        run_id: String,
        cleanup: RunCleanup,
    ) {
        let mut handle = self
            .groups
            .get_mut(&group_id)
            .and_then(|group| group.runs.get_mut(&run_id));
        let session_id = cleanup
            .session_id
            .or_else(|| handle.as_ref().and_then(|run| run.session_id.clone()));
        let worktree = cleanup
            .worktree
            .or_else(|| handle.as_ref().and_then(|run| run.worktree.clone()));
        if let Some(handle) = handle.as_mut() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(session_id) = session_id.clone() {
            if let Some(done) = self.waiting.remove(&session_id) {
                // Dropping the sender wakes the task; its outcome no longer
                // matters because the run is leaving the group.
                drop(done);
            }
            if let Err(message) = self.api.interrupt(&session_id).await {
                tracing::debug!(session = %session_id, %message, "could not interrupt a run");
            }
            if let Err(message) = self.api.delete(&session_id).await {
                tracing::debug!(session = %session_id, %message, "could not delete a run session");
            }
        }
        self.remove_worktree(&project_id, worktree.as_deref()).await;
        if let Some(handle) = self
            .groups
            .get_mut(&group_id)
            .and_then(|group| group.runs.get_mut(&run_id))
        {
            handle.state = RunState::Discarded;
            handle.session_id = None;
            handle.worktree = None;
        }
        let event = EngineEvent::RunStateChanged {
            group_id: group_id.clone(),
            run_id,
            state: RunState::Discarded,
            session_id: None,
            worktree: None,
        };
        if self.events.send(event).await.is_err() {
            tracing::debug!("the view stopped listening; dropping a run update");
        }
        self.maybe_finish(&group_id).await;
    }

    async fn discard_group(&mut self, project_id: String, group_id: String, runs: Vec<RunDiscard>) {
        let kept = self
            .groups
            .get(&group_id)
            .and_then(|group| group.kept.clone());
        let mut seen = Vec::new();
        let mut cleanups: Vec<(String, RunCleanup)> = runs
            .into_iter()
            .map(|run| (run.run_id, run.cleanup))
            .collect();
        if let Some(group) = self.groups.get(&group_id) {
            for (run_id, handle) in &group.runs {
                if cleanups.iter().any(|(id, _)| id == run_id) {
                    continue;
                }
                cleanups.push((
                    run_id.clone(),
                    RunCleanup {
                        session_id: handle.session_id.clone(),
                        worktree: handle.worktree.clone(),
                    },
                ));
            }
        }
        for (run_id, cleanup) in cleanups {
            if kept.as_deref() == Some(run_id.as_str()) {
                continue;
            }
            let handle = self
                .groups
                .get_mut(&group_id)
                .and_then(|group| group.runs.get_mut(&run_id));
            let session_id = cleanup
                .session_id
                .or_else(|| handle.as_ref().and_then(|run| run.session_id.clone()));
            let worktree = cleanup
                .worktree
                .or_else(|| handle.as_ref().and_then(|run| run.worktree.clone()));
            if let Some(handle) = handle {
                handle.cancel.store(true, Ordering::Relaxed);
            }
            if let Some(session_id) = session_id {
                if let Some(done) = self.waiting.remove(&session_id) {
                    drop(done);
                }
                if let Err(message) = self.api.interrupt(&session_id).await {
                    tracing::debug!(session = %session_id, %message, "could not interrupt a run");
                }
                if let Err(message) = self.api.delete(&session_id).await {
                    tracing::debug!(session = %session_id, %message, "could not delete a run session");
                }
            }
            self.remove_worktree(&project_id, worktree.as_deref()).await;
            seen.push(run_id);
        }
        for run_id in seen {
            let event = EngineEvent::RunStateChanged {
                group_id: group_id.clone(),
                run_id,
                state: RunState::Discarded,
                session_id: None,
                worktree: None,
            };
            if self.events.send(event).await.is_err() {
                tracing::debug!("the view stopped listening; dropping a run update");
            }
        }
        self.groups.remove(&group_id);
        let event = EngineEvent::RunGroupFinished {
            group_id: group_id.clone(),
        };
        if self.events.send(event).await.is_err() {
            tracing::debug!("the view stopped listening; dropping a group update");
        }
    }

    async fn remove_worktree(&self, project_id: &str, directory: Option<&str>) {
        let Some(directory) = directory else {
            return;
        };
        let request = RemoveWorktreeRequest {
            project_id: project_id.to_string(),
            directory: directory.to_string(),
            force: true,
        };
        if let Err(message) = self.api.remove_worktree(&request).await {
            tracing::debug!(worktree = %directory, %message, "could not remove a run worktree");
        }
    }

    fn verify_live_sessions(&self) {
        for (group_id, group) in &self.groups {
            for (run_id, run) in &group.runs {
                if !run.state.is_live() {
                    continue;
                }
                let Some(session_id) = run.session_id.clone() else {
                    continue;
                };
                let api = self.api.clone();
                let commands = self.self_tx.clone();
                let group_id = group_id.clone();
                let run_id = run_id.clone();
                let worktree = run.worktree.clone();
                tokio::spawn(async move {
                    if api.messages(&session_id).await.is_ok() {
                        return;
                    }
                    if commands
                        .send(RunCommand::Progress {
                            group_id,
                            run_id,
                            state: RunState::Failed {
                                message: "The run's session disappeared.".to_string(),
                            },
                            session_id: None,
                            worktree,
                        })
                        .is_err()
                    {
                        tracing::debug!("the run runtime is gone; dropping a verification");
                    }
                });
            }
        }
    }
}

/// One run's task. It owns the sequence and reports each step through the
/// runtime so the view and the cleanup path see the same resources.
struct RunTask {
    api: SessionApi,
    commands: UnboundedSender<RunCommand>,
    gate: RunGate,
    project_id: String,
    from: String,
    group_id: String,
    prompt: String,
    run: Run,
    cancel: Arc<AtomicBool>,
}

/// How a run task ended. Cancellation is not a failure: the runtime already
/// owns the discard and emits the final state.
enum RunEnd {
    Cancelled,
    Failed(String),
}

impl RunTask {
    async fn run(self) {
        let Some(_permit) = self.gate.acquire().await else {
            return;
        };
        let mut session_id: Option<String> = None;
        let mut worktree: Option<String> = None;
        let name = worktree_name(&self.group_id, &self.run.id);

        match self.steps(&name, &mut session_id, &mut worktree).await {
            Ok(()) => {
                self.progress(RunState::Done, session_id, worktree).await;
            }
            Err(RunEnd::Cancelled) => {
                // The runtime is discarding this run; clean up anything the
                // task created after the runtime's own snapshot so a cancelled
                // run cannot leak a checkout.
                self.cleanup(session_id.as_deref(), worktree.as_deref())
                    .await;
            }
            Err(RunEnd::Failed(message)) => {
                self.cleanup(session_id.as_deref(), worktree.as_deref())
                    .await;
                self.progress(RunState::Failed { message }, None, None)
                    .await;
            }
        }
    }

    async fn steps(
        &self,
        name: &str,
        session_id: &mut Option<String>,
        worktree: &mut Option<String>,
    ) -> Result<(), RunEnd> {
        let created = self
            .create_or_reuse_worktree(name)
            .await
            .map_err(|message| {
                RunEnd::Failed(format!("Could not create the run worktree: {message}"))
            })?;
        *worktree = Some(created.directory);
        self.progress(RunState::Starting, session_id.clone(), worktree.clone())
            .await;
        if self.cancelled() {
            return Err(RunEnd::Cancelled);
        }

        let session = self
            .api
            .create(worktree.as_deref().unwrap_or_default())
            .await
            .map_err(|message| {
                RunEnd::Failed(format!("Could not create the run session: {message}"))
            })?;
        *session_id = Some(session.id.clone());
        self.progress(RunState::Starting, session_id.clone(), worktree.clone())
            .await;
        if self.cancelled() {
            return Err(RunEnd::Cancelled);
        }

        self.api
            .set_model(&session.id, self.run.model.clone())
            .await
            .map_err(|message| {
                RunEnd::Failed(format!("Could not select the run model: {message}"))
            })?;

        let (done_tx, done_rx) = oneshot::channel();
        if self
            .commands
            .send(RunCommand::Awaiting {
                group_id: self.group_id.clone(),
                run_id: self.run.id.clone(),
                session_id: session.id.clone(),
                done: done_tx,
            })
            .is_err()
        {
            return Err(RunEnd::Failed(
                "The run runtime is shutting down.".to_string(),
            ));
        }
        if self.cancelled() {
            return Err(RunEnd::Cancelled);
        }

        self.api
            .prompt(
                &session.id,
                PromptRequest {
                    text: self.prompt.clone(),
                    ..PromptRequest::default()
                },
            )
            .await
            .map_err(|message| {
                RunEnd::Failed(format!("Could not send the run prompt: {message}"))
            })?;
        self.progress(RunState::Running, session_id.clone(), worktree.clone())
            .await;

        match done_rx.await {
            Ok(ExecutionOutcome::Succeeded) => Ok(()),
            Ok(ExecutionOutcome::Interrupted) => {
                Err(RunEnd::Failed("The turn was interrupted.".to_string()))
            }
            Ok(ExecutionOutcome::Failed { message }) => Err(RunEnd::Failed(message)),
            Err(_) => Err(RunEnd::Failed(
                "The run ended without a result.".to_string(),
            )),
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn create_or_reuse_worktree(&self, name: &str) -> Result<Worktree, String> {
        let request = CreateWorktreeRequest {
            project_id: self.project_id.clone(),
            from: self.from.clone(),
            branch: None,
            directory: None,
            name: name.to_string(),
        };
        match self.api.create_worktree(&request).await {
            Ok(worktree) => Ok(worktree),
            Err(message) if already_exists(&message) => {
                let worktrees = self.api.worktrees(&self.project_id).await?;
                worktrees
                    .into_iter()
                    .find(|worktree| {
                        Path::new(&worktree.directory)
                            .file_name()
                            .and_then(|file| file.to_str())
                            == Some(name)
                    })
                    .ok_or(message)
            }
            Err(message) => Err(message),
        }
    }

    async fn cleanup(&self, session_id: Option<&str>, worktree: Option<&str>) {
        if let Some(session_id) = session_id
            && let Err(message) = self.api.delete(session_id).await
        {
            tracing::debug!(session = %session_id, %message, "could not delete a failed run session");
        }
        if let Some(worktree) = worktree {
            let request = RemoveWorktreeRequest {
                project_id: self.project_id.clone(),
                directory: worktree.to_string(),
                force: true,
            };
            if let Err(message) = self.api.remove_worktree(&request).await {
                tracing::debug!(worktree = %worktree, %message, "could not remove a failed run worktree");
            }
        }
    }

    async fn progress(
        &self,
        state: RunState,
        session_id: Option<String>,
        worktree: Option<String>,
    ) {
        if self
            .commands
            .send(RunCommand::Progress {
                group_id: self.group_id.clone(),
                run_id: self.run.id.clone(),
                state,
                session_id,
                worktree,
            })
            .is_err()
        {
            tracing::debug!("the run runtime is gone; dropping a run update");
        }
    }
}

fn already_exists(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("already exists") || message.contains("exist")
}

/// A filesystem- and branch-safe worktree name derived from the group and run.
pub(crate) fn worktree_name(group_id: &str, run_id: &str) -> String {
    let mut name: String = format!("{group_id}-{run_id}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect();
    if name.len() > 80 {
        name.truncate(80);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn run_group_caps_concurrency_at_five() {
        let gate = Arc::new(RunGate::new());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..7 {
            let gate = gate.clone();
            let active = active.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                let _permit = gate.acquire().await.expect("permit");
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for task in tasks {
            task.await.expect("task");
        }
        assert_eq!(peak.load(Ordering::SeqCst), MAX_RUNS_PER_GROUP);
    }

    #[test]
    fn worktree_names_are_path_safe_and_bounded() {
        let name = worktree_name("runs-1731/ab", "run 1");
        assert_eq!(name, "runs-1731-ab-run-1");
        assert!(name.len() <= 80);
    }

    #[test]
    fn existing_worktree_errors_are_recognized() {
        assert!(already_exists("worktree already exists at /tmp/x"));
        assert!(already_exists(
            "Failed to create worktree: directory exists"
        ));
        assert!(!already_exists("could not reach the server"));
    }
}
