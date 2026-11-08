//! Supervises the OpenCode sidecar process.

mod api;
mod events;
mod locate;
mod supervisor;

use std::path::PathBuf;

use async_channel::{Receiver, Sender};
use tokio::sync::oneshot;

pub use harness_protocol::{ExecutionOutcome, PermissionReply, StreamEvent};
pub use locate::{LocateError, locate_binary};
pub use supervisor::EngineEvent;

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
    /// Load a session's message history.
    LoadTranscript {
        session_id: String,
    },
    /// Send a prompt; the reply streams back through the event channel.
    SendPrompt {
        session_id: String,
        text: String,
    },
    /// Interrupt the running turn.
    Interrupt {
        session_id: String,
    },
    /// Reload a session's pending permission requests.
    LoadPermissions {
        session_id: String,
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

    /// Load a session's message history.
    pub fn load_transcript(&self, id: impl Into<String>) -> bool {
        self.send(EngineCommand::LoadTranscript {
            session_id: id.into(),
        })
    }

    /// Send a prompt to a session. The reply arrives as [`EngineEvent::Stream`]
    /// updates on [`Engine::events`].
    pub fn send_prompt(&self, id: impl Into<String>, text: impl Into<String>) -> bool {
        self.send(EngineCommand::SendPrompt {
            session_id: id.into(),
            text: text.into(),
        })
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
