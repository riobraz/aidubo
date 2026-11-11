//! Supervises the OpenCode sidecar process.

mod api;
mod events;
mod features;
mod locate;
mod supervisor;

#[cfg(test)]
mod api_tests;

use std::path::PathBuf;

use async_channel::{Receiver, Sender};
use tokio::sync::oneshot;

pub use harness_protocol::{
    AgentInfo, CatalogKind, CommandInfo, CommandRequest, ExecutionOutcome, FileDiff, FileEntry,
    FormInfo, InboxDelivery, InboxInfo, ModelInfo, ModelRef, PermissionReply, PromptRequest,
    SkillInfo, StreamEvent,
};
pub use locate::{LocateError, locate_binary};
pub use supervisor::EngineEvent;

/// A catalog snapshot returned by the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogData {
    Commands(Vec<CommandInfo>),
    Skills(Vec<SkillInfo>),
    Agents(Vec<AgentInfo>),
    Models(Vec<ModelInfo>),
}

/// Configuration for one sidecar instance.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Project directory the agent should operate in.
    pub project_dir: PathBuf,
    /// Optional explicit path to the `opencode` binary.
    pub binary: Option<PathBuf>,
}

impl EngineConfig {
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        Self {
            project_dir: project_dir.into(),
            binary: None,
        }
    }

    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = Some(binary.into());
        self
    }
}

/// Work sent from the UI to the supervisor thread.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// Reload the project's session list from the server.
    ListSessions,
    /// Create a new session in the project and select it.
    CreateSession,
    RenameSession {
        id: String,
        title: String,
    },
    DeleteSession {
        id: String,
    },
    /// Create a form. Used by tests and plugins; the app never creates one.
    CreateForm {
        session_id: String,
        title: String,
        fields: serde_json::Value,
    },
    /// Load installed plugins.
    LoadPlugins {
        request_id: u64,
    },
    /// Run a plugin update check.
    CheckPlugins {
        request_id: u64,
    },
    /// Update one plugin.
    UpdatePlugin {
        id: String,
    },
    /// Load provider budget limits.
    LoadUsageLimits {
        request_id: u64,
    },
    /// Ask the server to compact the session.
    Compact {
        session_id: String,
    },
    /// Load the messages the model would see.
    LoadContext {
        session_id: String,
    },
    /// Stage a revert at a message.
    StageRevert {
        session_id: String,
        message_id: String,
        files: Option<bool>,
    },
    /// Commit the staged revert.
    CommitRevert {
        session_id: String,
    },
    /// Clear the staged revert.
    ClearRevert {
        session_id: String,
    },
    /// Reload a session's pending forms.
    LoadForms {
        session_id: String,
    },
    /// Answer a form.
    ReplyForm {
        session_id: String,
        form_id: String,
        answer: serde_json::Value,
    },
    /// Dismiss a form.
    DismissForm {
        session_id: String,
        form_id: String,
    },
    /// Reload a session's inbox.
    LoadInbox {
        session_id: String,
    },
    /// Remove a queued prompt.
    CancelInbox {
        session_id: String,
        inbox_id: String,
    },
    /// Change a queued prompt's delivery.
    SetInboxDelivery {
        session_id: String,
        inbox_id: String,
        delivery: InboxDelivery,
    },
    /// Fork a session before a message.
    ForkSession {
        session_id: String,
        before: Option<String>,
    },
    /// Load a session's message history.
    LoadTranscript {
        session_id: String,
    },
    /// Send a prompt; the reply streams back through the event channel.
    SendPrompt {
        session_id: String,
        request: PromptRequest,
    },
    /// Run a slash command in a session.
    RunCommand {
        session_id: String,
        request: CommandRequest,
    },
    /// Ask the server to change a session's model.
    SetSessionModel {
        session_id: String,
        model: ModelRef,
    },
    /// Ask the server to change a session's agent.
    SetSessionAgent {
        session_id: String,
        agent: String,
    },
    /// Search project files for the `@` picker.
    FindFiles {
        request_id: u64,
        query: String,
        limit: usize,
    },
    /// Load one server-backed catalog.
    ListCatalog {
        request_id: u64,
        kind: CatalogKind,
    },
    /// Interrupt the running turn.
    Interrupt {
        session_id: String,
    },
    /// Reload a session's pending permission requests.
    LoadPermissions {
        session_id: String,
    },
    /// Load the project's working-tree, branch, or committed diff.
    LoadVcsDiff {
        request_id: u64,
        mode: String,
        base: Option<String>,
        context: Option<String>,
        untracked: String,
    },
    /// Load the project's working-tree status.
    LoadVcsStatus {
        request_id: u64,
    },
    /// Load the ref the working tree is compared against.
    LoadVcsBase {
        request_id: u64,
    },
    /// Initialize a repository in the project directory.
    InitRepo {
        request_id: u64,
    },
    /// Load the files a session changed.
    LoadSessionDiff {
        request_id: u64,
        session_id: String,
        from: Option<String>,
        to: Option<String>,
        context: Option<String>,
    },
    /// Answer a permission request, unblocking the turn.
    ReplyPermission {
        session_id: String,
        request_id: String,
        reply: PermissionReply,
    },
}

/// Handle to a running (or starting) sidecar supervisor.
///
/// Dropping the handle asks the supervisor to stop the child process and waits
/// for its thread to exit, so the app cannot leave an orphaned server behind on
/// a normal quit.
pub struct Engine {
    events: Receiver<EngineEvent>,
    commands: Sender<EngineCommand>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(config: EngineConfig) -> Self {
        let (event_tx, event_rx) = async_channel::unbounded();
        let (command_tx, command_rx) = async_channel::unbounded();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let thread = std::thread::Builder::new()
            .name("opencode-engine".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("failed to build the engine runtime");
                runtime.block_on(supervisor::run(config, event_tx, command_rx, shutdown_rx));
            })
            .expect("failed to spawn the engine thread");

        Self {
            events: event_rx,
            commands: command_tx,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    /// A cloneable stream of engine lifecycle events.
    pub fn events(&self) -> Receiver<EngineEvent> {
        self.events.clone()
    }

    /// Reload the session list. Returns false once the engine is shutting down.
    pub fn refresh_sessions(&self) -> bool {
        self.send(EngineCommand::ListSessions)
    }

    /// Create a session in the project.
    pub fn create_session(&self) -> bool {
        self.send(EngineCommand::CreateSession)
    }

    /// Rename a session. The title is trimmed by the supervisor.
    pub fn rename_session(&self, id: impl Into<String>, title: impl Into<String>) -> bool {
        self.send(EngineCommand::RenameSession {
            id: id.into(),
            title: title.into(),
        })
    }

    /// Delete a session.
    pub fn delete_session(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::DeleteSession { id: id.into() })
    }

    /// Create a form. Exposed for tests and plugins; the app never calls it.
    pub fn create_form(
        &self,
        id: impl Into<String>,
        title: impl Into<String>,
        fields: serde_json::Value,
    ) -> bool {
        self.send(EngineCommand::CreateForm {
            session_id: id.into(),
            title: title.into(),
            fields,
        })
    }

    /// Load installed plugins.
    pub fn load_plugins(&self, request_id: u64) -> bool {
        self.send(EngineCommand::LoadPlugins { request_id })
    }

    /// Run a plugin update check.
    pub fn check_plugins(&self, request_id: u64) -> bool {
        self.send(EngineCommand::CheckPlugins { request_id })
    }

    /// Update one plugin. The UI confirms before calling this.
    pub fn update_plugin(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::UpdatePlugin { id: id.into() })
    }

    /// Load provider budget limits.
    pub fn load_usage_limits(&self, request_id: u64) -> bool {
        self.send(EngineCommand::LoadUsageLimits { request_id })
    }

    /// Ask the server to compact the session.
    pub fn compact(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::Compact {
            session_id: id.into(),
        })
    }

    /// Load the messages the model would see.
    pub fn load_context(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadContext {
            session_id: id.into(),
        })
    }

    /// Stage a revert at a message.
    pub fn stage_revert(
        &self,
        id: impl Into<String>,
        message_id: impl Into<String>,
        files: Option<bool>,
    ) -> bool {
        self.send(EngineCommand::StageRevert {
            session_id: id.into(),
            message_id: message_id.into(),
            files,
        })
    }

    /// Commit the staged revert.
    pub fn commit_revert(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::CommitRevert {
            session_id: id.into(),
        })
    }

    /// Clear the staged revert.
    pub fn clear_revert(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::ClearRevert {
            session_id: id.into(),
        })
    }

    /// Reload a session's pending forms.
    pub fn load_forms(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadForms {
            session_id: id.into(),
        })
    }

    /// Answer a form.
    pub fn reply_form(
        &self,
        id: impl Into<String>,
        form_id: impl Into<String>,
        answer: serde_json::Value,
    ) -> bool {
        self.send(EngineCommand::ReplyForm {
            session_id: id.into(),
            form_id: form_id.into(),
            answer,
        })
    }

    /// Dismiss a form.
    pub fn dismiss_form(&self, id: impl Into<String>, form_id: impl Into<String>) -> bool {
        self.send(EngineCommand::DismissForm {
            session_id: id.into(),
            form_id: form_id.into(),
        })
    }

    /// Reload a session's inbox.
    pub fn load_inbox(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadInbox {
            session_id: id.into(),
        })
    }

    /// Remove a queued prompt.
    pub fn cancel_inbox(&self, id: impl Into<String>, inbox_id: impl Into<String>) -> bool {
        self.send(EngineCommand::CancelInbox {
            session_id: id.into(),
            inbox_id: inbox_id.into(),
        })
    }

    /// Change a queued prompt's delivery.
    pub fn set_inbox_delivery(
        &self,
        id: impl Into<String>,
        inbox_id: impl Into<String>,
        delivery: InboxDelivery,
    ) -> bool {
        self.send(EngineCommand::SetInboxDelivery {
            session_id: id.into(),
            inbox_id: inbox_id.into(),
            delivery,
        })
    }

    /// Fork a session before a message (or the whole session).
    pub fn fork_session(&self, id: impl Into<String>, before: Option<String>) -> bool {
        self.send(EngineCommand::ForkSession {
            session_id: id.into(),
            before,
        })
    }

    /// Load a session's message history.
    pub fn load_transcript(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadTranscript {
            session_id: id.into(),
        })
    }

    /// Send a prompt to a session. The reply arrives as [`EngineEvent::Stream`]
    /// updates on [`Engine::events`].
    pub fn send_prompt(&self, id: impl Into<String>, request: PromptRequest) -> bool {
        self.send(EngineCommand::SendPrompt {
            session_id: id.into(),
            request,
        })
    }

    /// Run a slash command in a session.
    pub fn run_command(&self, id: impl Into<String>, request: CommandRequest) -> bool {
        self.send(EngineCommand::RunCommand {
            session_id: id.into(),
            request,
        })
    }

    /// Ask the server to switch the session's model.
    pub fn set_session_model(&self, id: impl Into<String>, model: ModelRef) -> bool {
        self.send(EngineCommand::SetSessionModel {
            session_id: id.into(),
            model,
        })
    }

    /// Ask the server to switch the session's agent.
    pub fn set_session_agent(&self, id: impl Into<String>, agent: impl Into<String>) -> bool {
        self.send(EngineCommand::SetSessionAgent {
            session_id: id.into(),
            agent: agent.into(),
        })
    }

    /// Load the project's working-tree status.
    pub fn load_vcs_status(&self, request_id: u64) -> bool {
        self.send(EngineCommand::LoadVcsStatus { request_id })
    }

    /// Load the ref the working tree is compared against.
    pub fn load_vcs_base(&self, request_id: u64) -> bool {
        self.send(EngineCommand::LoadVcsBase { request_id })
    }

    /// Initialize a git repository in the project directory.
    pub fn init_repo(&self, request_id: u64) -> bool {
        self.send(EngineCommand::InitRepo { request_id })
    }

    /// Load the project's diff. `mode` is `working`, `branch`, or `committed`.
    pub fn load_vcs_diff(
        &self,
        request_id: u64,
        mode: impl Into<String>,
        base: Option<String>,
        context: Option<String>,
        untracked: impl Into<String>,
    ) -> bool {
        self.send(EngineCommand::LoadVcsDiff {
            request_id,
            mode: mode.into(),
            base,
            context,
            untracked: untracked.into(),
        })
    }

    /// Load the files a session changed between two messages.
    pub fn load_session_diff(
        &self,
        request_id: u64,
        session_id: impl Into<String>,
        from: Option<String>,
        to: Option<String>,
        context: Option<String>,
    ) -> bool {
        self.send(EngineCommand::LoadSessionDiff {
            request_id,
            session_id: session_id.into(),
            from,
            to,
            context,
        })
    }

    /// Search project files for the composer's `@` picker.
    pub fn find_files(&self, request_id: u64, query: impl Into<String>, limit: usize) -> bool {
        self.send(EngineCommand::FindFiles {
            request_id,
            query: query.into(),
            limit,
        })
    }

    /// Load one server-backed catalog.
    pub fn list_catalog(&self, request_id: u64, kind: CatalogKind) -> bool {
        self.send(EngineCommand::ListCatalog { request_id, kind })
    }

    /// Interrupt the running turn in a session.
    pub fn interrupt(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::Interrupt {
            session_id: id.into(),
        })
    }

    /// Reload a session's pending permission requests.
    pub fn load_permissions(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadPermissions {
            session_id: id.into(),
        })
    }

    /// Answer a permission request.
    pub fn reply_permission(
        &self,
        session_id: impl Into<String>,
        request_id: impl Into<String>,
        reply: PermissionReply,
    ) -> bool {
        self.send(EngineCommand::ReplyPermission {
            session_id: session_id.into(),
            request_id: request_id.into(),
            reply,
        })
    }

    fn send(&self, command: EngineCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    /// Ask the supervisor to stop and wait for it. Idempotent.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
