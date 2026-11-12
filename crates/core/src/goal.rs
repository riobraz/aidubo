//! The goal loop: caps, audit decisions, and relaunch policy.
//!
//! An auditor session decides whether the objective was met. This module owns
//! every rule that can stop the loop, so the engine and the UI only carry
//! state around and can never invent a new turn.

use harness_protocol::{AssistantPart, SessionMessage};
use serde::{Deserialize, Serialize};

/// One goal bound to one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub state: GoalState,
    pub continuations: u32,
    pub tokens_used: u64,
    /// Epoch milliseconds. A resume restarts the wall clock.
    pub started_at: i64,
    pub auditor_session_id: Option<String>,
}

/// Where a goal is in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalState {
    Running,
    Stopped(Stopped),
    /// The app was closed while the goal was running. Never resumed silently.
    Interrupted,
}

/// Why a goal stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stopped {
    Done { reason: String },
    Blocked { reason: String },
    Cap { which: CapKind },
    Error { message: String },
    User,
}

/// The cap that ended a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapKind {
    Continuations,
    Tokens,
    WallClock,
}

/// One persisted goal, including the project it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRecord {
    pub directory: String,
    pub goal: Goal,
    pub limits: GoalLimits,
}

/// Mandatory limits for one goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalLimits {
    pub max_continuations: u32,
    pub token_budget: u64,
    pub max_wall_clock_secs: u64,
}

impl Default for GoalLimits {
    fn default() -> Self {
        Self {
            max_continuations: 10,
            token_budget: 200_000,
            max_wall_clock_secs: 30 * 60,
        }
    }
}

/// What the loop should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalAction {
    /// The turn is idle; ask the auditor.
    AskAuditor,
    /// Send the auditor's next step to the main session.
    Prompt {
        text: String,
    },
    Stop(Stopped),
}

/// The budget counters reported when a turn goes idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalUsage {
    pub tokens: u64,
    pub now_ms: i64,
}

/// The auditor's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditDecision {
    Continue { next: String },
    Finish { reason: String },
    Block { reason: String },
}

impl AuditDecision {
    /// Parse the auditor's last assistant reply. Anything that is not one of
    /// the three documented decisions becomes a block: invalid output must
    /// never cause a new turn.
    pub fn parse(raw: &str) -> Self {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            decision: String,
            #[serde(default)]
            reason: String,
            #[serde(default)]
            next: String,
        }

        let Ok(wire) = serde_json::from_str::<Wire>(raw.trim()) else {
            return Self::Block {
                reason: "auditor returned invalid JSON".into(),
            };
        };
        match wire.decision.as_str() {
            "continue" if !wire.next.trim().is_empty() => Self::Continue {
                next: wire.next.trim().to_owned(),
            },
            "continue" => Self::Block {
                reason: "auditor returned no next step".into(),
            },
            "finish" => Self::Finish {
                reason: non_empty(wire.reason, "the auditor reported the objective met"),
            },
            "block" => Self::Block {
                reason: non_empty(wire.reason, "the auditor reported a blocker"),
            },
            _ => Self::Block {
                reason: "auditor returned invalid JSON".into(),
            },
        }
    }
}

fn non_empty(value: String, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

/// Goal state plus the limits it must enforce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalMachine {
    goal: Goal,
    limits: GoalLimits,
}

impl GoalMachine {
    /// Start a new goal. The state is forced to `Running`.
    pub fn new(mut goal: Goal, limits: GoalLimits) -> Self {
        goal.state = GoalState::Running;
        Self { goal, limits }
    }

    /// Rehydrate a persisted goal. A goal saved as running was cut short by a
    /// relaunch, so it becomes `Interrupted` and is never resumed silently.
    pub fn restore(mut goal: Goal, limits: GoalLimits) -> Self {
        if matches!(goal.state, GoalState::Running) {
            goal.state = GoalState::Interrupted;
        }
        Self { goal, limits }
    }

    /// Replace the tracked goal with an engine snapshot.
    pub fn sync_goal(&mut self, goal: Goal) {
        self.goal = goal;
    }

    pub fn goal(&self) -> &Goal {
        &self.goal
    }

    pub fn limits(&self) -> GoalLimits {
        self.limits
    }

    pub fn is_running(&self) -> bool {
        matches!(self.goal.state, GoalState::Running)
    }

    pub fn is_interrupted(&self) -> bool {
        matches!(self.goal.state, GoalState::Interrupted)
    }

    /// Resume an interrupted goal. The wall clock restarts; the token and
    /// continuation counters continue from where they stopped.
    pub fn resume(&mut self, now_ms: i64) -> bool {
        if !self.is_interrupted() {
            return false;
        }
        self.goal.state = GoalState::Running;
        self.goal.started_at = now_ms;
        true
    }

    /// A turn finished and the session is idle. Enforce every cap before
    /// asking the auditor.
    pub fn on_idle(&mut self, usage: GoalUsage) -> GoalAction {
        if let Some(action) = self.not_running() {
            return action;
        }
        self.goal.tokens_used = self.goal.tokens_used.max(usage.tokens);

        if self.goal.continuations >= self.limits.max_continuations {
            return self.stop(Stopped::Cap {
                which: CapKind::Continuations,
            });
        }
        if self.goal.tokens_used >= self.limits.token_budget {
            return self.stop(Stopped::Cap {
                which: CapKind::Tokens,
            });
        }
        let wall_clock_ms = self.limits.max_wall_clock_secs.saturating_mul(1_000) as i64;
        if usage.now_ms.saturating_sub(self.goal.started_at) >= wall_clock_ms {
            return self.stop(Stopped::Cap {
                which: CapKind::WallClock,
            });
        }
        GoalAction::AskAuditor
    }

    /// Apply the auditor's decision.
    pub fn on_audit(&mut self, decision: AuditDecision) -> GoalAction {
        if let Some(action) = self.not_running() {
            return action;
        }
        match decision {
            AuditDecision::Continue { next } => {
                let next = next.trim().to_owned();
                if next.is_empty() {
                    return self.stop(Stopped::Blocked {
                        reason: "auditor returned an empty next step".into(),
                    });
                }
                if self.goal.continuations >= self.limits.max_continuations {
                    return self.stop(Stopped::Cap {
                        which: CapKind::Continuations,
                    });
                }
                self.goal.continuations += 1;
                GoalAction::Prompt { text: next }
            }
            AuditDecision::Finish { reason } => self.stop(Stopped::Done { reason }),
            AuditDecision::Block { reason } => self.stop(Stopped::Blocked { reason }),
        }
    }

    /// A turn failed. The loop stops; it never retries blindly.
    pub fn on_turn_failed(&mut self, message: impl Into<String>) -> GoalAction {
        self.stop(Stopped::Error {
            message: message.into(),
        })
    }

    /// The user asked to stop.
    pub fn stop_by_user(&mut self) -> GoalAction {
        self.stop(Stopped::User)
    }

    /// Stop the loop for a reason the engine observed (a failed prompt, a
    /// deleted session). Keeping the first stop reason makes the UI stable if
    /// two failures race.
    pub fn halt(&mut self, stopped: Stopped) -> Goal {
        if self.is_running() {
            self.goal.state = GoalState::Stopped(stopped);
        }
        self.goal.clone()
    }

    fn not_running(&self) -> Option<GoalAction> {
        match &self.goal.state {
            GoalState::Running => None,
            GoalState::Stopped(stopped) => Some(GoalAction::Stop(stopped.clone())),
            GoalState::Interrupted => Some(GoalAction::Stop(Stopped::Error {
                message: "the goal is interrupted".into(),
            })),
        }
    }

    fn stop(&mut self, stopped: Stopped) -> GoalAction {
        self.goal.state = GoalState::Stopped(stopped.clone());
        GoalAction::Stop(stopped)
    }
}

/// The last assistant message's text, tool output excluded. The auditor sees
/// only this text plus the objective.
pub fn last_assistant_text(messages: &[SessionMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        let SessionMessage::Assistant { content, .. } = message else {
            return None;
        };
        let text = content
            .iter()
            .filter_map(|part| match part {
                AssistantPart::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        (!text.trim().is_empty()).then_some(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal() -> Goal {
        Goal {
            id: "goal-1".into(),
            session_id: "session-1".into(),
            objective: "make the tests pass".into(),
            state: GoalState::Running,
            continuations: 0,
            tokens_used: 0,
            started_at: 1_000,
            auditor_session_id: Some("auditor-1".into()),
        }
    }

    fn machine(limits: GoalLimits) -> GoalMachine {
        GoalMachine::new(goal(), limits)
    }

    fn idle(tokens: u64, now_ms: i64) -> GoalUsage {
        GoalUsage { tokens, now_ms }
    }

    #[test]
    fn goal_continues_until_finish() {
        let mut machine = machine(GoalLimits::default());
        assert_eq!(machine.on_idle(idle(10, 2_000)), GoalAction::AskAuditor);
        assert_eq!(
            machine.on_audit(AuditDecision::Continue {
                next: "fix the failing test".into()
            }),
            GoalAction::Prompt {
                text: "fix the failing test".into()
            }
        );
        assert_eq!(machine.goal().continuations, 1);
        assert_eq!(machine.on_idle(idle(20, 3_000)), GoalAction::AskAuditor);
        assert_eq!(
            machine.on_audit(AuditDecision::Finish {
                reason: "all tests pass".into()
            }),
            GoalAction::Stop(Stopped::Done {
                reason: "all tests pass".into()
            })
        );
        assert!(!machine.is_running());
    }

    #[test]
    fn goal_stops_at_continuation_cap() {
        let mut machine = machine(GoalLimits {
            max_continuations: 2,
            ..GoalLimits::default()
        });
        for turn in 0..2 {
            assert_eq!(
                machine.on_idle(idle(10, 2_000 + turn)),
                GoalAction::AskAuditor
            );
            assert!(matches!(
                machine.on_audit(AuditDecision::Continue {
                    next: format!("step {turn}")
                }),
                GoalAction::Prompt { .. }
            ));
        }
        assert_eq!(
            machine.on_idle(idle(10, 4_000)),
            GoalAction::Stop(Stopped::Cap {
                which: CapKind::Continuations
            })
        );
    }

    #[test]
    fn goal_stops_at_token_budget() {
        let mut machine = machine(GoalLimits {
            token_budget: 100,
            ..GoalLimits::default()
        });
        assert_eq!(
            machine.on_idle(idle(101, 2_000)),
            GoalAction::Stop(Stopped::Cap {
                which: CapKind::Tokens
            })
        );
    }

    #[test]
    fn goal_stops_at_wall_clock() {
        let mut machine = machine(GoalLimits {
            max_wall_clock_secs: 1,
            ..GoalLimits::default()
        });
        assert_eq!(
            machine.on_idle(idle(10, 1_000 + 1_000)),
            GoalAction::Stop(Stopped::Cap {
                which: CapKind::WallClock
            })
        );
    }

    #[test]
    fn goal_invalid_audit_json_blocks() {
        let decision = AuditDecision::parse("not json at all");
        assert_eq!(
            decision,
            AuditDecision::Block {
                reason: "auditor returned invalid JSON".into()
            }
        );
        let mut machine = machine(GoalLimits::default());
        assert_eq!(
            machine.on_audit(decision),
            GoalAction::Stop(Stopped::Blocked {
                reason: "auditor returned invalid JSON".into()
            })
        );
        assert_eq!(machine.goal().continuations, 0);
    }

    #[test]
    fn goal_turn_failure_stops() {
        let mut machine = machine(GoalLimits::default());
        assert_eq!(
            machine.on_turn_failed("the provider rejected the request"),
            GoalAction::Stop(Stopped::Error {
                message: "the provider rejected the request".into()
            })
        );
        assert!(!machine.is_running());
    }

    #[test]
    fn goal_relaunch_is_interrupted() {
        let mut restored = GoalMachine::restore(goal(), GoalLimits::default());
        assert_eq!(restored.goal().state, GoalState::Interrupted);
        assert!(!restored.is_running());
        assert_eq!(
            restored.on_idle(idle(0, 2_000)),
            GoalAction::Stop(Stopped::Error {
                message: "the goal is interrupted".into()
            })
        );
    }

    #[test]
    fn goal_resume_restarts_the_wall_clock() {
        let mut restored = GoalMachine::restore(goal(), GoalLimits::default());
        assert!(restored.resume(50_000));
        assert!(restored.is_running());
        assert_eq!(restored.goal().started_at, 50_000);
        assert_eq!(restored.on_idle(idle(10, 51_000)), GoalAction::AskAuditor);
    }

    #[test]
    fn audit_parse_accepts_the_three_decisions() {
        assert_eq!(
            AuditDecision::parse(r#"{"decision":"continue","next":"run the linter"}"#),
            AuditDecision::Continue {
                next: "run the linter".into()
            }
        );
        assert_eq!(
            AuditDecision::parse(r#"{"decision":"finish","reason":"done"}"#),
            AuditDecision::Finish {
                reason: "done".into()
            }
        );
        assert_eq!(
            AuditDecision::parse(r#"{"decision":"block","reason":"needs a key"}"#),
            AuditDecision::Block {
                reason: "needs a key".into()
            }
        );
    }

    #[test]
    fn last_reply_ignores_tool_output() {
        let messages = vec![
            SessionMessage::Assistant {
                id: "a1".into(),
                time: Default::default(),
                agent: None,
                model: None,
                content: vec![
                    AssistantPart::Text {
                        text: "first".into(),
                    },
                    AssistantPart::Tool {
                        id: "t1".into(),
                        name: "bash".into(),
                        executed: true,
                        state: Default::default(),
                    },
                ],
                finish: None,
                error: None,
            },
            SessionMessage::User {
                id: "u2".into(),
                time: Default::default(),
                text: "more".into(),
            },
        ];
        assert_eq!(last_assistant_text(&messages).as_deref(), Some("first"));
    }
}
