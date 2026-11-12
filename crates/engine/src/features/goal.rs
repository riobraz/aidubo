//! The app-side goal loop.
//!
//! A goal owns one main session and one auditor session. The runtime watches
//! `ExecutionFinished` for both, asks the auditor after the main turn, and
//! prompts the main session only when the auditor says `continue`. Every cap is
//! enforced by `harness_core::GoalMachine`, not here.

use async_channel::Sender;
use harness_core::{
    AuditDecision, Goal, GoalAction, GoalLimits, GoalMachine, GoalState, GoalUsage, Stopped,
    last_assistant_text, now_ms,
};
use harness_protocol::{ExecutionOutcome, ModelInfo, ModelRef, PromptRequest, StreamEvent};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::api::SessionApi;
use crate::supervisor::EngineEvent;

/// Work sent to the goal runtime by the supervisor.
#[derive(Debug, Clone)]
pub(crate) enum GoalCommand {
    Start {
        session_id: String,
        objective: String,
        limits: GoalLimits,
    },
    Stop {
        goal_id: String,
    },
    Resume {
        goal: Goal,
        limits: GoalLimits,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitingMain,
    WaitingAuditor,
}

pub(crate) struct GoalRuntime {
    api: SessionApi,
    events: Sender<EngineEvent>,
    directory: String,
    auditor_model: Option<ModelRef>,
    commands: UnboundedReceiver<GoalCommand>,
    stream: UnboundedReceiver<StreamEvent>,
    machine: Option<GoalMachine>,
    phase: Phase,
    tokens: u64,
    /// SSE replay repeats the last finish event after a reconnect; the id keeps
    /// one turn from being audited or prompted twice.
    last_finish_event_id: Option<String>,
}

impl GoalRuntime {
    pub(crate) fn new(
        api: SessionApi,
        events: Sender<EngineEvent>,
        directory: String,
        auditor_model: Option<ModelRef>,
        commands: UnboundedReceiver<GoalCommand>,
        stream: UnboundedReceiver<StreamEvent>,
    ) -> Self {
        Self {
            api,
            events,
            directory,
            auditor_model,
            commands,
            stream,
            machine: None,
            phase: Phase::Idle,
            tokens: 0,
            last_finish_event_id: None,
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
                    self.handle_stream(event).await;
                }
            }
        }
    }

    async fn handle_command(&mut self, command: GoalCommand) {
        match command {
            GoalCommand::Start {
                session_id,
                objective,
                limits,
            } => self.start(session_id, objective, limits).await,
            GoalCommand::Stop { goal_id } => self.stop(goal_id).await,
            GoalCommand::Resume { goal, limits } => self.resume(goal, limits).await,
        }
    }

    async fn handle_stream(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::UsageUpdated {
                session_id, tokens, ..
            } => {
                let Some(goal) = self.machine.as_ref().map(|machine| machine.goal()) else {
                    return;
                };
                if session_id == goal.session_id {
                    self.tokens = self.tokens.max(tokens.total());
                }
            }
            StreamEvent::ExecutionFinished {
                session_id,
                event_id,
                outcome,
            } => {
                if self.last_finish_event_id.as_deref() == Some(event_id.as_str()) {
                    return;
                }
                self.last_finish_event_id = Some(event_id);
                self.handle_finished(&session_id, outcome).await;
            }
            _ => {}
        }
    }

    async fn start(&mut self, session_id: String, objective: String, limits: GoalLimits) {
        if self.is_running() {
            self.fail("A goal is already running.");
            return;
        }
        let auditor_model = match self.resolve_auditor_model().await {
            Ok(model) => model,
            Err(message) => {
                self.fail(message);
                return;
            }
        };
        let auditor = match self.api.create(&self.directory).await {
            Ok(session) => session,
            Err(message) => {
                self.fail(format!(
                    "Could not create the goal auditor session: {message}"
                ));
                return;
            }
        };
        if let Err(message) = self.api.set_model(&auditor.id, auditor_model).await {
            self.fail(format!(
                "Could not select the goal auditor model: {message}"
            ));
            return;
        }

        let goal = Goal {
            id: format!("goal-{}-{:08x}", now_ms(), rand::random::<u32>()),
            session_id: session_id.clone(),
            objective: objective.clone(),
            state: GoalState::Running,
            continuations: 0,
            tokens_used: 0,
            started_at: now_ms(),
            auditor_session_id: Some(auditor.id),
        };
        self.tokens = 0;
        self.machine = Some(GoalMachine::new(goal.clone(), limits));
        self.phase = Phase::WaitingMain;
        self.emit(EngineEvent::GoalStarted { goal, limits }).await;

        if let Err(message) = self.prompt(&session_id, &objective).await {
            self.finish(Stopped::Error {
                message: format!("Could not start the goal: {message}"),
            })
            .await;
        }
    }

    async fn stop(&mut self, goal_id: String) {
        let Some(machine) = self.machine.as_mut() else {
            self.fail("No goal is running.");
            return;
        };
        if machine.goal().id != goal_id {
            self.fail("No goal is running.");
            return;
        }
        if !machine.is_running() {
            self.fail("The goal has already stopped.");
            return;
        }
        let action = machine.stop_by_user();
        self.apply(action).await;
    }

    async fn resume(&mut self, goal: Goal, limits: GoalLimits) {
        if self.is_running() {
            self.fail("A goal is already running.");
            return;
        }
        let mut machine = GoalMachine::restore(goal, limits);
        if !machine.resume(now_ms()) {
            self.fail("Only an interrupted goal can be resumed.");
            return;
        }
        self.tokens = machine.goal().tokens_used;
        self.phase = Phase::Idle;
        let snapshot = machine.goal().clone();
        self.machine = Some(machine);
        self.emit(EngineEvent::GoalStarted {
            goal: snapshot,
            limits,
        })
        .await;
        self.ask_auditor().await;
    }

    async fn handle_finished(&mut self, session_id: &str, outcome: ExecutionOutcome) {
        let main_session = self
            .machine
            .as_ref()
            .map(|machine| machine.goal().session_id.clone());
        let auditor_session = self
            .machine
            .as_ref()
            .and_then(|machine| machine.goal().auditor_session_id.clone());

        if self.phase == Phase::WaitingMain && main_session.as_deref() == Some(session_id) {
            match outcome {
                ExecutionOutcome::Succeeded => {
                    self.phase = Phase::Idle;
                    let Some(machine) = self.machine.as_mut() else {
                        return;
                    };
                    let action = machine.on_idle(GoalUsage {
                        tokens: self.tokens,
                        now_ms: now_ms(),
                    });
                    self.apply(action).await;
                }
                ExecutionOutcome::Interrupted => {
                    self.finish(Stopped::Error {
                        message: "the turn was interrupted".into(),
                    })
                    .await
                }
                ExecutionOutcome::Failed { message } => {
                    self.finish(Stopped::Error { message }).await
                }
            }
        } else if self.phase == Phase::WaitingAuditor
            && auditor_session.as_deref() == Some(session_id)
        {
            match outcome {
                ExecutionOutcome::Succeeded => {
                    self.phase = Phase::Idle;
                    self.audit().await;
                }
                ExecutionOutcome::Interrupted => {
                    self.finish(Stopped::Error {
                        message: "the goal auditor was interrupted".into(),
                    })
                    .await
                }
                ExecutionOutcome::Failed { message } => {
                    self.finish(Stopped::Error {
                        message: format!("the goal auditor failed: {message}"),
                    })
                    .await
                }
            }
        }
    }

    async fn ask_auditor(&mut self) {
        let Some(machine) = self.machine.as_ref() else {
            return;
        };
        let goal = machine.goal().clone();
        let Some(auditor_id) = goal.auditor_session_id.clone() else {
            self.finish(Stopped::Error {
                message: "the goal has no auditor session".into(),
            })
            .await;
            return;
        };
        let reply = match self.api.messages(&goal.session_id).await {
            Ok(messages) => last_assistant_text(&messages)
                .unwrap_or_else(|| "(the session produced no text reply)".into()),
            Err(message) => {
                self.finish(Stopped::Error {
                    message: format!("Could not read the last reply: {message}"),
                })
                .await;
                return;
            }
        };
        let prompt = auditor_prompt(&goal.objective, &reply);
        if let Err(message) = self.prompt(&auditor_id, &prompt).await {
            self.finish(Stopped::Error {
                message: format!("Could not ask the goal auditor: {message}"),
            })
            .await;
            return;
        }
        self.phase = Phase::WaitingAuditor;
    }

    async fn audit(&mut self) {
        let Some(machine) = self.machine.as_ref() else {
            return;
        };
        let goal = machine.goal().clone();
        let Some(auditor_id) = goal.auditor_session_id.clone() else {
            self.finish(Stopped::Error {
                message: "the goal has no auditor session".into(),
            })
            .await;
            return;
        };
        let raw = match self.api.messages(&auditor_id).await {
            Ok(messages) => last_assistant_text(&messages).unwrap_or_default(),
            Err(message) => {
                self.finish(Stopped::Error {
                    message: format!("Could not read the auditor's reply: {message}"),
                })
                .await;
                return;
            }
        };
        let decision = AuditDecision::parse(&raw);
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let action = machine.on_audit(decision.clone());
        let snapshot = machine.goal().clone();
        self.emit(EngineEvent::GoalAudited {
            goal: snapshot,
            decision,
        })
        .await;
        self.apply(action).await;
    }

    async fn apply(&mut self, action: GoalAction) {
        match action {
            GoalAction::AskAuditor => self.ask_auditor().await,
            GoalAction::Prompt { text } => {
                let Some(machine) = self.machine.as_ref() else {
                    return;
                };
                let session_id = machine.goal().session_id.clone();
                let goal = machine.goal().clone();
                if let Err(message) = self.prompt(&session_id, &text).await {
                    self.finish(Stopped::Error {
                        message: format!("Could not continue the goal: {message}"),
                    })
                    .await;
                    return;
                }
                self.phase = Phase::WaitingMain;
                self.emit(EngineEvent::GoalContinued { goal }).await;
            }
            GoalAction::Stop(stopped) => self.finish(stopped).await,
        }
    }

    async fn finish(&mut self, stopped: Stopped) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let goal = machine.halt(stopped.clone());
        self.phase = Phase::Idle;
        self.emit(EngineEvent::GoalStopped {
            goal,
            reason: stopped,
        })
        .await;
    }

    fn is_running(&self) -> bool {
        self.machine
            .as_ref()
            .map(GoalMachine::is_running)
            .unwrap_or(false)
    }

    async fn resolve_auditor_model(&self) -> Result<ModelRef, String> {
        if let Some(model) = &self.auditor_model {
            return Ok(model.clone());
        }
        let models = self
            .api
            .models(&self.directory)
            .await
            .map_err(|message| format!("Could not load the model catalog: {message}"))?;
        cheapest_model(&models)
            .ok_or_else(|| "No model is available for the goal auditor.".to_string())
    }

    async fn prompt(&self, session_id: &str, text: &str) -> Result<(), String> {
        self.api
            .prompt(
                session_id,
                PromptRequest {
                    text: text.to_owned(),
                    ..PromptRequest::default()
                },
            )
            .await
            .map(|_| ())
    }

    fn fail(&self, message: impl Into<String>) {
        // The runtime is driven by the supervisor future; a closed UI channel
        // means the app is already shutting down.
        if self
            .events
            .try_send(EngineEvent::CommandFailed {
                message: message.into(),
            })
            .is_err()
        {
            tracing::debug!("the UI stopped listening; a goal error was dropped");
        }
    }

    async fn emit(&self, event: EngineEvent) {
        if self.events.send(event).await.is_err() {
            tracing::debug!("the UI stopped listening; a goal event was dropped");
        }
    }
}

/// Pick the cheapest enabled model; a catalog with no prices still yields a
/// model rather than failing the goal.
fn cheapest_model(models: &[ModelInfo]) -> Option<ModelRef> {
    let enabled: Vec<&ModelInfo> = models.iter().filter(|model| model.enabled).collect();
    let candidates = if enabled.is_empty() {
        models.iter().collect()
    } else {
        enabled
    };
    candidates
        .into_iter()
        .min_by(|left, right| left.input_cost().total_cmp(&right.input_cost()))
        .map(ModelInfo::model_ref)
}

/// The auditor's entire input: the objective and the last assistant text.
pub(crate) fn auditor_prompt(objective: &str, last_reply: &str) -> String {
    format!(
        "You audit whether an autonomous agent met its objective.\n\n\
         Objective:\n{objective}\n\n\
         Latest assistant reply:\n{last_reply}\n\n\
         Reply with strict JSON only, without markdown fences:\n\
         {{\"decision\":\"continue\",\"reason\":\"why not done\",\"next\":\"the next step\"}}\n\
         when more work is needed,\n\
         {{\"decision\":\"finish\",\"reason\":\"why it is done\"}} when the objective is met,\n\
         or {{\"decision\":\"block\",\"reason\":\"what blocks it\"}} when it cannot continue."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, enabled: bool, input: f64) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            model_id: id.into(),
            provider_id: "mock".into(),
            name: id.into(),
            enabled,
            cost: vec![harness_protocol::ModelCost {
                input,
                output: 200.0,
            }],
        }
    }

    #[test]
    fn auditor_prompt_contains_only_objective_and_reply() {
        let prompt = auditor_prompt("ship the fix", "the fix is shipped");
        assert!(prompt.contains("ship the fix"));
        assert!(prompt.contains("the fix is shipped"));
        assert_eq!(prompt.matches("ship the fix").count(), 1);
        assert_eq!(prompt.matches("the fix is shipped").count(), 1);
    }

    #[test]
    fn goal_auditor_prefers_the_cheapest_enabled_model() {
        let models = vec![
            model("expensive", true, 12.0),
            model("disabled-and-free", false, 0.0),
            model("cheap", true, 0.1),
        ];
        let chosen = cheapest_model(&models).expect("a model");
        assert_eq!(chosen.id, "cheap");
        assert_eq!(chosen.provider_id.as_deref(), Some("mock"));
    }

    #[test]
    fn goal_auditor_falls_back_to_the_catalog_when_none_are_enabled() {
        let models = vec![model("only", false, 3.0)];
        assert_eq!(
            cheapest_model(&models).map(|model| model.id),
            Some("only".into())
        );
    }

    #[test]
    fn goal_auditor_token_total_counts_every_bucket() {
        let tokens = harness_protocol::TokenUsage {
            input: 10.0,
            output: 5.0,
            reasoning: 2.0,
            cache: harness_protocol::TokenCache {
                read: 3.0,
                write: 1.0,
            },
        };
        assert_eq!(tokens.total(), 21);
    }
}
