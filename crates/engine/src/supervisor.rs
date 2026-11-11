//! The sidecar supervisor: launch, wait for readiness, run commands, stop.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures_util::StreamExt as _;
use harness_protocol::{
    AcceptedPrompt, AttemptStatus, AuthStatus, CatalogKind, ModelRef, PermissionRequest, Session,
    SessionList, SessionMessage, StreamEvent,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};

use crate::api::SessionApi;
use crate::events;
use crate::features::pty::{PtyLinks, pump_pty};
use crate::{CatalogData, EngineCommand, EngineConfig, locate};

/// Lifecycle events and command results emitted by the supervisor.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// The child has been spawned and is being probed.
    Starting { port: u16 },
    /// The server answered an authenticated request.
    Ready { version: String, port: u16 },
    /// Startup or runtime failure, safe to show to the user.
    Failed { message: String },
    /// The child was stopped because the app asked it to.
    Stopped,
    /// A line from the child's stderr.
    Log(String),
    /// A full snapshot of the project's sessions.
    SessionsLoaded { sessions: Vec<Session> },
    /// A new session was created on the server.
    SessionCreated { session: Session },
    /// A session title was changed on the server.
    SessionRenamed { id: String, title: String },
    /// A session was deleted on the server.
    SessionDeleted { id: String },
    /// The session's pending forms.
    FormsLoaded {
        session_id: String,
        forms: Vec<harness_protocol::FormInfo>,
    },
    /// Schedules for a session.
    SchedulesLoaded {
        request_id: u64,
        session_id: String,
        result: Result<Vec<harness_protocol::ScheduleInfo>, String>,
    },
    /// A schedule was created on the server.
    ScheduleCreated {
        schedule: harness_protocol::ScheduleInfo,
    },
    /// A schedule was paused, resumed, or refreshed.
    ScheduleChanged {
        schedule: harness_protocol::ScheduleInfo,
    },
    /// A schedule was deleted on the server.
    ScheduleDeleted { id: String },
    /// Providers.
    ProvidersLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::ProviderInfo>, String>,
    },
    /// Integrations.
    IntegrationsLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::IntegrationInfo>, String>,
    },
    /// An auth attempt started; the user must act when `url` or `command` is set.
    AuthAttemptStarted {
        integration_id: String,
        kind: String,
        method: String,
        attempt_id: String,
        url: Option<String>,
        command: Option<String>,
        instructions: String,
    },
    /// An attempt's server-reported status changed.
    AuthAttemptUpdated {
        integration_id: String,
        attempt_id: String,
        status: harness_protocol::AuthStatus,
    },
    /// An attempt failed, expired, timed out, or could not start.
    AuthAttemptFailed {
        integration_id: String,
        message: String,
    },
    /// The user cancelled an attempt.
    AuthAttemptCancelled { integration_id: String },
    /// The server stored a credential for the integration.
    ProviderConnected { integration_id: String },
    /// Saved permission rules.
    SavedPermissionsLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::SavedPermission>, String>,
    },
    /// A saved rule was revoked.
    SavedPermissionDeleted {
        id: String,
        result: Result<(), String>,
    },
    /// MCP servers.
    McpLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::McpServer>, String>,
    },
    /// An MCP mutation finished; the UI reloads the list.
    McpChanged { name: String },
    /// An MCP mutation failed; the server snapshot is unchanged.
    McpError { name: String, message: String },
    /// Installed plugins.
    PluginsLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::PluginInfo>, String>,
    },
    /// A plugin update finished.
    PluginUpdated {
        id: String,
        result: Result<(), String>,
    },
    /// A plugin check finished.
    PluginsChecked {
        request_id: u64,
        result: Result<(), String>,
    },
    /// Provider budget limits.
    UsageLimitsLoaded {
        request_id: u64,
        result: Result<Option<harness_protocol::UsageLimits>, String>,
    },
    /// The server accepted a compaction request.
    CompactionRequested { session_id: String },
    /// The messages the model would see.
    ContextLoaded {
        session_id: String,
        messages: Vec<harness_protocol::SessionMessage>,
    },
    /// A revert was staged and is waiting for commit.
    RevertStaged {
        session_id: String,
        revert: harness_protocol::SessionRevert,
    },
    /// A staged revert was committed.
    RevertCommitted {
        session_id: String,
        message_id: String,
    },
    /// The staged revert was cleared.
    RevertCleared { session_id: String },
    /// A form was created (tests and plugins).
    FormCreated { form: harness_protocol::FormInfo },
    /// A form was answered or dismissed.
    FormSettled { form_id: String },
    /// A form operation failed.
    FormError { message: String },
    /// The session's inbox snapshot.
    InboxLoaded {
        session_id: String,
        items: Vec<harness_protocol::InboxInfo>,
    },
    /// The server forked a session.
    SessionForked { session: Session },
    /// A session's message history snapshot.
    TranscriptLoaded {
        session_id: String,
        messages: Vec<SessionMessage>,
    },
    /// The server admitted a prompt; the user message is now part of the timeline.
    PromptAccepted {
        session_id: String,
        message: AcceptedPrompt,
    },
    /// A slash command was accepted by the server.
    CommandAccepted { session_id: String },
    /// The project's working-tree status.
    VcsStatusLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::VcsFileStatus>, String>,
    },
    /// The ref the working tree is compared against.
    VcsBaseLoaded {
        request_id: u64,
        result: Result<Option<harness_protocol::VcsBase>, String>,
    },
    /// A repository was initialized.
    RepoInitialized {
        request_id: u64,
        result: Result<(), String>,
    },
    /// A diff snapshot for the project.
    VcsDiffLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::FileDiff>, String>,
    },
    /// A diff snapshot for one session.
    SessionDiffLoaded {
        request_id: u64,
        result: Result<Vec<harness_protocol::FileDiff>, String>,
    },
    /// The server accepted a model change.
    SessionModelSet { session_id: String, model: ModelRef },
    /// The server accepted an agent change.
    SessionAgentSet { session_id: String, agent: String },
    /// File search results for the `@` picker.
    FilesFound {
        request_id: u64,
        result: Result<Vec<harness_protocol::FileEntry>, String>,
    },
    /// One catalog snapshot.
    CatalogLoaded {
        request_id: u64,
        kind: CatalogKind,
        result: Result<CatalogData, String>,
    },
    /// A live update from the event stream.
    Stream(StreamEvent),
    /// The event stream (re)connected; a transcript resnapshot is due.
    StreamConnected,
    /// The event stream dropped; live updates pause until it reconnects.
    StreamDisconnected,
    /// The result of an interrupt request.
    Interrupted {
        session_id: String,
        interrupted: bool,
    },
    /// The session's pending permission requests, oldest first.
    PermissionsLoaded {
        session_id: String,
        requests: Vec<PermissionRequest>,
    },
    /// The server accepted a permission reply; the waiting turn can continue.
    PermissionReplied {
        session_id: String,
        request_id: String,
    },
    /// A PTY snapshot was created or refreshed.
    PtyCreated { pty: harness_protocol::PtyProcess },
    /// The project's PTY list.
    PtysLoaded {
        result: Result<Vec<harness_protocol::PtyProcess>, String>,
    },
    /// Coalesced output from a PTY connection.
    PtyOutput { id: String, bytes: Vec<u8> },
    /// A PTY connection closed or its process was removed.
    PtyClosed { id: String },
    /// A PTY operation failed; the message is safe to display.
    PtyError { id: String, message: String },
    /// A session export was written to `path`.
    SessionExported {
        session_id: String,
        path: PathBuf,
        bytes: u64,
    },
    /// A session export failed; the message is safe to display.
    SessionExportError { message: String },
    /// An import file was read and parsed for the preview.
    ImportPreviewed {
        path: PathBuf,
        transfer: harness_protocol::SessionTransfer,
    },
    /// A session was imported into the project.
    SessionImported { session: Session },
    /// An import failed; the message is safe to display.
    SessionImportError { message: String },
    /// Usage statistics for a range.
    StatsLoaded {
        request_id: u64,
        result: Result<harness_protocol::SessionStats, String>,
    },
    /// A command could not be completed; the message is safe to display.
    CommandFailed { message: String },
}

pub(crate) async fn run(
    config: EngineConfig,
    events: Sender<EngineEvent>,
    commands: Receiver<EngineCommand>,
    shutdown: oneshot::Receiver<()>,
) {
    if let Err(message) = run_inner(config, &events, commands, shutdown).await {
        tracing::debug!(%message, "engine stopped");
        let _ = events.send(EngineEvent::Failed { message }).await;
    }
}

async fn run_inner(
    config: EngineConfig,
    events: &Sender<EngineEvent>,
    commands: Receiver<EngineCommand>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), String> {
    if !config.project_dir.is_dir() {
        return Err(format!(
            "project directory does not exist: {}",
            config.project_dir.display()
        ));
    }

    // The HTTP `directory` is canonicalized by `SessionApi`; the child's cwd
    // must match it, or project-scoped endpoints (VCS, worktrees) answer for a
    // different path on systems where `/var` is a symlink to `/private/var`.
    let project_dir =
        std::fs::canonicalize(&config.project_dir).unwrap_or_else(|_| config.project_dir.clone());

    let binary = locate::locate_binary(config.binary.as_deref()).map_err(|err| err.to_string())?;
    tracing::debug!(binary = %binary.display(), "located the OpenCode binary");
    let version = read_version(&binary).await;
    let port = pick_port().map_err(|err| format!("could not reserve a loopback port: {err}"))?;
    let password = random_password();

    events
        .send(EngineEvent::Starting { port })
        .await
        .map_err(|_| "the UI stopped listening".to_string())?;

    let mut child = Command::new(&binary)
        .arg("serve")
        .arg("--hostname")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .env("OPENCODE_SERVER_PASSWORD", &password)
        // v2.2 reads OPENCODE_PASSWORD; older builds read
        // OPENCODE_SERVER_PASSWORD. Set both so one binary works either way.
        .env("OPENCODE_PASSWORD", &password)
        .current_dir(&project_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("failed to launch {}: {err}", binary.display()))?;

    if let Some(stderr) = child.stderr.take() {
        let events = events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "opencode", "{line}");
                if events.send(EngineEvent::Log(line)).await.is_err() {
                    break;
                }
            }
        });
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| format!("could not build the HTTP client: {err}"))?;
    let probe_url = format!("http://127.0.0.1:{port}/api/session");
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if Instant::now() >= deadline {
            stop(&mut child).await;
            return Err(format!(
                "OpenCode did not become ready within 30s on 127.0.0.1:{port}"
            ));
        }

        tokio::select! {
            _ = &mut shutdown => {
                stop(&mut child).await;
                let _ = events.send(EngineEvent::Stopped).await;
                return Ok(());
            }
            status = child.wait() => {
                return Err(exit_message(status));
            }
            response = client
                .get(&probe_url)
                .basic_auth("opencode", Some(&password))
                .send() => {
                match response {
                    Ok(response) if response.status().is_success() => {
                        match response.json::<SessionList>().await {
                            Ok(list) => tracing::debug!(
                                sessions = list.data.len(),
                                "OpenCode responded to the readiness probe"
                            ),
                            Err(err) => tracing::debug!(
                                %err,
                                "OpenCode is ready, but the session probe did not parse"
                            ),
                        }
                        break;
                    }
                    Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
                        stop(&mut child).await;
                        return Err("OpenCode rejected the generated credentials".into());
                    }
                    Ok(_) | Err(_) => {}
                }
            }
        }

        sleep(Duration::from_millis(250)).await;
    }

    events
        .send(EngineEvent::Ready { version, port })
        .await
        .map_err(|_| "the UI stopped listening".to_string())?;

    let api = match SessionApi::new(port, password, &config.project_dir) {
        Ok(api) => api,
        Err(message) => {
            stop(&mut child).await;
            return Err(message);
        }
    };

    // Follow server events for the lifetime of the sidecar. The task is owned
    // by this runtime, so it is cancelled when the supervisor returns.
    tokio::spawn(pump_events(api.clone(), events.clone()));

    // Populate the sidebar as soon as the engine is usable.
    let mut plugin_check_at: Option<std::time::Instant> = None;
    let mut auth_pollers = AuthPollers::default();
    let mut pty_links = PtyLinks::default();
    match run_command(
        &api,
        EngineCommand::ListSessions,
        &mut shutdown,
        &mut plugin_check_at,
        events,
        &mut auth_pollers,
        &mut pty_links,
    )
    .await
    {
        CommandResult::Event(event) => {
            let _ = events.send(*event).await;
        }
        CommandResult::Events(batch) => {
            for event in batch.into_vec() {
                let _ = events.send(event).await;
            }
        }
        CommandResult::None => {}
        CommandResult::Shutdown => {
            stop(&mut child).await;
            let _ = events.send(EngineEvent::Stopped).await;
            return Ok(());
        }
    }

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                stop(&mut child).await;
                let _ = events.send(EngineEvent::Stopped).await;
                return Ok(());
            }
            status = child.wait() => {
                return Err(exit_message(status));
            }
            command = commands.recv() => {
                let Ok(command) = command else {
                    // The UI dropped its handle; stop rather than linger.
                    stop(&mut child).await;
                    let _ = events.send(EngineEvent::Stopped).await;
                    return Ok(());
                };

                match run_command(
                    &api,
                    command,
                    &mut shutdown,
                    &mut plugin_check_at,
                    events,
                    &mut auth_pollers,
                    &mut pty_links,
                )
                .await
                {
                    CommandResult::Event(event) => {
                        let _ = events.send(*event).await;
                    }
                    CommandResult::Events(batch) => {
                        for event in batch.into_vec() {
                            let _ = events.send(event).await;
                        }
                    }
                    CommandResult::None => {}
                    CommandResult::Shutdown => {
                        stop(&mut child).await;
                        let _ = events.send(EngineEvent::Stopped).await;
                        return Ok(());
                    }
                }
            }
        }
    }
}

enum CommandResult {
    /// Boxed to keep the transient result small next to `Shutdown`.
    Event(Box<EngineEvent>),
    /// A mutation plus the authoritative list reload that follows it.
    Events(Box<[EngineEvent]>),
    /// The command succeeded without a UI-visible event.
    None,
    Shutdown,
}

fn event(event: EngineEvent) -> CommandResult {
    CommandResult::Event(Box::new(event))
}

fn events(events: Vec<EngineEvent>) -> CommandResult {
    CommandResult::Events(events.into_boxed_slice())
}

const AUTH_POLL_INTERVAL: Duration = Duration::from_secs(1);
const AUTH_POLL_LIMIT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollStep {
    Wait,
    Finished,
    TimedOut,
}

fn poll_step(elapsed: Duration, status: &AuthStatus) -> PollStep {
    match status {
        AuthStatus::Pending | AuthStatus::Unknown(_) => {
            if elapsed >= AUTH_POLL_LIMIT {
                PollStep::TimedOut
            } else {
                PollStep::Wait
            }
        }
        AuthStatus::Complete | AuthStatus::Failed | AuthStatus::Expired => PollStep::Finished,
    }
}

fn attempt_message(status: &AttemptStatus) -> String {
    status
        .message
        .clone()
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| match status.status {
            AuthStatus::Expired => "The connection attempt expired. Start it again.".into(),
            _ => "The connection attempt failed. Start it again.".into(),
        })
}

fn auth_status_events(
    integration_id: String,
    attempt_id: String,
    status: AttemptStatus,
) -> CommandResult {
    let mut batch = vec![EngineEvent::AuthAttemptUpdated {
        integration_id: integration_id.clone(),
        attempt_id,
        status: status.status.clone(),
    }];
    match status.status {
        AuthStatus::Complete => batch.push(EngineEvent::ProviderConnected { integration_id }),
        AuthStatus::Failed | AuthStatus::Expired => batch.push(EngineEvent::AuthAttemptFailed {
            integration_id,
            message: attempt_message(&status),
        }),
        AuthStatus::Pending | AuthStatus::Unknown(_) => {}
    }
    events(batch)
}

#[derive(Debug, Clone, Copy)]
enum AttemptKind {
    OAuth,
    Command,
}

/// Attempt handles kept only so cancel can abort the poller immediately.
#[derive(Default)]
struct AuthPollers {
    tasks: HashMap<String, JoinHandle<()>>,
}

impl AuthPollers {
    fn track(&mut self, attempt_id: String, handle: JoinHandle<()>) {
        if let Some(previous) = self.tasks.insert(attempt_id, handle) {
            previous.abort();
        }
    }

    fn stop(&mut self, attempt_id: &str) {
        if let Some(handle) = self.tasks.remove(attempt_id) {
            handle.abort();
        }
    }
}

fn spawn_attempt_poller(
    pollers: &mut AuthPollers,
    api: SessionApi,
    event_tx: Sender<EngineEvent>,
    kind: AttemptKind,
    integration_id: String,
    attempt_id: String,
) {
    let tracked = attempt_id.clone();
    let handle = tokio::spawn(async move {
        let started = Instant::now();
        loop {
            sleep(AUTH_POLL_INTERVAL).await;
            let status = match kind {
                AttemptKind::OAuth => api.oauth_status(&integration_id, &attempt_id).await,
                AttemptKind::Command => api.command_status(&integration_id, &attempt_id).await,
            };
            let status = match status {
                Ok(status) => status,
                Err(message) => {
                    let _ = event_tx
                        .send(EngineEvent::AuthAttemptFailed {
                            integration_id,
                            message,
                        })
                        .await;
                    return;
                }
            };
            match poll_step(started.elapsed(), &status.status) {
                PollStep::Wait => continue,
                PollStep::TimedOut => {
                    let _ = event_tx
                        .send(EngineEvent::AuthAttemptFailed {
                            integration_id,
                            message: "The connection attempt timed out. Start it again.".into(),
                        })
                        .await;
                    return;
                }
                PollStep::Finished => {
                    let CommandResult::Events(batch) =
                        auth_status_events(integration_id, attempt_id, status)
                    else {
                        return;
                    };
                    for event in batch.into_vec() {
                        if event_tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    return;
                }
            }
        }
    });
    pollers.track(tracked, handle);
}

/// Pause or resume, then reload the list so `nextDueAt` is server-provided.
async fn schedule_mutation(
    api: &SessionApi,
    shutdown: &mut oneshot::Receiver<()>,
    request_id: u64,
    session_id: String,
    schedule_id: String,
    action: &str,
) -> CommandResult {
    tokio::select! {
        _ = &mut *shutdown => CommandResult::Shutdown,
        result = async {
            let schedule = api.schedule_action(&schedule_id, action).await?;
            let schedules = api.schedules(&session_id).await;
            Ok::<_, String>((schedule, schedules))
        } => match result {
            Ok((schedule, schedules)) => events(vec![
                EngineEvent::ScheduleChanged { schedule },
                EngineEvent::SchedulesLoaded { request_id, session_id, result: schedules },
            ]),
            Err(message) => event(EngineEvent::CommandFailed {
                message: format!("Could not {action} the schedule: {message}"),
            }),
        },
    }
}

async fn run_command(
    api: &SessionApi,
    command: EngineCommand,
    shutdown: &mut oneshot::Receiver<()>,
    plugin_check_at: &mut Option<std::time::Instant>,
    event_tx: &Sender<EngineEvent>,
    pollers: &mut AuthPollers,
    pty_links: &mut PtyLinks,
) -> CommandResult {
    match command {
        EngineCommand::ListSessions => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.list() => event(match result {
                Ok(sessions) => {
                    tracing::debug!(count = sessions.len(), "loaded project sessions");
                    EngineEvent::SessionsLoaded { sessions }
                }
                Err(message) => EngineEvent::CommandFailed {
                    message: format!("Could not load sessions: {message}"),
                },
            }),
        },
        EngineCommand::CreateSession => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.create() => event(match result {
                Ok(session) => {
                    tracing::debug!(session = %session.id, "created session");
                    EngineEvent::SessionCreated { session }
                }
                Err(message) => EngineEvent::CommandFailed {
                    message: format!("Could not create a session: {message}"),
                },
            }),
        },
        EngineCommand::RenameSession { id, title } => {
            let title = title.trim().to_string();
            if title.is_empty() {
                return event(EngineEvent::CommandFailed {
                    message: "A session title cannot be empty.".into(),
                });
            }
            let call_id = id.clone();
            let call_title = title.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.rename(&call_id, call_title) => event(match result {
                    Ok(()) => {
                        tracing::debug!(session = %call_id, "renamed session");
                        EngineEvent::SessionRenamed { id, title }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not rename the session: {message}"),
                    },
                }),
            }
        }
        EngineCommand::LoadSchedules {
            request_id,
            session_id,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.schedules(&session_id) => {
                event(EngineEvent::SchedulesLoaded { request_id, session_id, result })
            }
        },
        EngineCommand::CreateSchedule {
            request_id,
            request,
        } => {
            let session_id = request.session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = async {
                    let schedule = api.create_schedule(*request).await?;
                    let schedules = api.schedules(&session_id).await;
                    Ok::<_, String>((schedule, schedules))
                } => match result {
                    Ok((schedule, schedules)) => events(vec![
                        EngineEvent::ScheduleCreated { schedule },
                        EngineEvent::SchedulesLoaded { request_id, session_id, result: schedules },
                    ]),
                    Err(message) => event(EngineEvent::CommandFailed {
                        message: format!("Could not create the schedule: {message}"),
                    }),
                },
            }
        }
        EngineCommand::PauseSchedule {
            request_id,
            session_id,
            schedule_id,
        } => schedule_mutation(api, shutdown, request_id, session_id, schedule_id, "pause").await,
        EngineCommand::ResumeSchedule {
            request_id,
            session_id,
            schedule_id,
        } => schedule_mutation(api, shutdown, request_id, session_id, schedule_id, "resume").await,
        EngineCommand::DeleteSchedule {
            request_id,
            session_id,
            schedule_id,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = async {
                api.delete_schedule(&schedule_id).await?;
                api.schedules(&session_id).await
            } => match result {
                Ok(schedules) => events(vec![
                    EngineEvent::ScheduleDeleted { id: schedule_id },
                    EngineEvent::SchedulesLoaded { request_id, session_id, result: Ok(schedules) },
                ]),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not delete the schedule: {message}"),
                }),
            },
        },
        EngineCommand::LoadProviders { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.providers() => event(EngineEvent::ProvidersLoaded { request_id, result }),
        },
        EngineCommand::LoadIntegrations { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.integrations() => event(EngineEvent::IntegrationsLoaded { request_id, result }),
        },
        EngineCommand::StartOAuth {
            integration_id,
            method_id,
        } => {
            let call_integration = integration_id.clone();
            let call_method = method_id.clone();
            let poll_api = api.clone();
            let poll_events = event_tx.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.start_oauth(&call_integration, &call_method) => match result {
                    Ok(attempt) if attempt.attempt_id.is_empty() => event(EngineEvent::AuthAttemptFailed {
                        integration_id,
                        message: "OpenCode returned an attempt without an id.".into(),
                    }),
                    Ok(attempt) => {
                        spawn_attempt_poller(
                            pollers,
                            poll_api,
                            poll_events,
                            AttemptKind::OAuth,
                            integration_id.clone(),
                            attempt.attempt_id.clone(),
                        );
                        event(EngineEvent::AuthAttemptStarted {
                            integration_id,
                            kind: "oauth".into(),
                            method: method_id,
                            attempt_id: attempt.attempt_id,
                            url: (!attempt.url.is_empty()).then_some(attempt.url),
                            command: None,
                            instructions: attempt.instructions,
                        })
                    }
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::PollOAuth {
            integration_id,
            attempt_id,
        } => {
            let call_integration = integration_id.clone();
            let call_attempt = attempt_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.oauth_status(&call_integration, &call_attempt) => match result {
                    Ok(status) => auth_status_events(integration_id, attempt_id, status),
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::CancelOAuth {
            integration_id,
            attempt_id,
        } => {
            pollers.stop(&attempt_id);
            let call_integration = integration_id.clone();
            let call_attempt = attempt_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.cancel_oauth(&call_integration, &call_attempt) => match result {
                    Ok(()) => event(EngineEvent::AuthAttemptCancelled { integration_id }),
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::CompleteOAuth {
            integration_id,
            attempt_id,
            code,
        } => {
            let call_integration = integration_id.clone();
            let call_attempt = attempt_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = async {
                    api.complete_oauth(&call_integration, &call_attempt, code).await?;
                    api.oauth_status(&call_integration, &call_attempt).await
                } => match result {
                    Ok(status) => auth_status_events(integration_id, attempt_id, status),
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::StartCommandAuth {
            integration_id,
            method_id,
            command,
        } => {
            let call_integration = integration_id.clone();
            let call_method = method_id.clone();
            let poll_api = api.clone();
            let poll_events = event_tx.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.start_command(&call_integration, &call_method) => match result {
                    Ok(attempt) if attempt.attempt_id.is_empty() => event(EngineEvent::AuthAttemptFailed {
                        integration_id,
                        message: "OpenCode returned an attempt without an id.".into(),
                    }),
                    Ok(attempt) => {
                        spawn_attempt_poller(
                            pollers,
                            poll_api,
                            poll_events,
                            AttemptKind::Command,
                            integration_id.clone(),
                            attempt.attempt_id.clone(),
                        );
                        event(EngineEvent::AuthAttemptStarted {
                            integration_id,
                            kind: "command".into(),
                            method: method_id,
                            attempt_id: attempt.attempt_id,
                            url: None,
                            command: Some(command),
                            instructions: String::new(),
                        })
                    }
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::PollCommandAuth {
            integration_id,
            attempt_id,
        } => {
            let call_integration = integration_id.clone();
            let call_attempt = attempt_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.command_status(&call_integration, &call_attempt) => match result {
                    Ok(status) => auth_status_events(integration_id, attempt_id, status),
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::CancelCommandAuth {
            integration_id,
            attempt_id,
        } => {
            pollers.stop(&attempt_id);
            let call_integration = integration_id.clone();
            let call_attempt = attempt_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.cancel_command(&call_integration, &call_attempt) => match result {
                    Ok(()) => event(EngineEvent::AuthAttemptCancelled { integration_id }),
                    Err(message) => event(EngineEvent::AuthAttemptFailed { integration_id, message }),
                }
            }
        }
        EngineCommand::SendKey {
            integration_id,
            key,
        } => {
            let call_integration = integration_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.send_key(&call_integration, &key) => event(match result {
                    Ok(()) => EngineEvent::ProviderConnected { integration_id },
                    Err(message) => EngineEvent::AuthAttemptFailed { integration_id, message },
                }),
            }
        }
        EngineCommand::ConnectExternal {
            integration_id,
            method_id,
        } => {
            let call_integration = integration_id.clone();
            let call_method = method_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.connect_external(&call_integration, &call_method) => event(match result {
                    Ok(()) => EngineEvent::ProviderConnected { integration_id },
                    Err(message) => EngineEvent::AuthAttemptFailed { integration_id, message },
                }),
            }
        }
        EngineCommand::LoadSavedPermissions {
            request_id,
            project_id,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.saved_permissions(&project_id) => {
                event(EngineEvent::SavedPermissionsLoaded { request_id, result })
            }
        },
        EngineCommand::DeleteSavedPermission { id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.delete_saved_permission(&id) => {
                event(EngineEvent::SavedPermissionDeleted { id, result })
            }
        },
        EngineCommand::LoadMcp { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.mcp_servers() => event(EngineEvent::McpLoaded { request_id, result }),
        },
        EngineCommand::AddMcp { name, config } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.add_mcp(&name, config) => match result {
                Ok(()) => event(EngineEvent::McpChanged { name }),
                Err(message) => event(EngineEvent::McpError {
                    name,
                    message: format!("Could not add the MCP server: {message}"),
                }),
            },
        },
        EngineCommand::RemoveMcp { name } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.remove_mcp(&name) => match result {
                Ok(()) => event(EngineEvent::McpChanged { name }),
                Err(message) => event(EngineEvent::McpError {
                    name,
                    message: format!("Could not remove the MCP server: {message}"),
                }),
            },
        },
        EngineCommand::ConnectMcp { name } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.connect_mcp(&name) => match result {
                Ok(()) => event(EngineEvent::McpChanged { name }),
                Err(message) => event(EngineEvent::McpError {
                    name,
                    message: format!("Could not connect the MCP server: {message}"),
                }),
            },
        },
        EngineCommand::DisconnectMcp { name } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.disconnect_mcp(&name) => match result {
                Ok(()) => event(EngineEvent::McpChanged { name }),
                Err(message) => event(EngineEvent::McpError {
                    name,
                    message: format!("Could not disconnect the MCP server: {message}"),
                }),
            },
        },
        EngineCommand::LoadPlugins { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.plugins() => event(EngineEvent::PluginsLoaded { request_id, result }),
        },
        EngineCommand::CheckPlugins { request_id } => {
            let now = std::time::Instant::now();
            if plugin_check_at
                .is_some_and(|at| now.duration_since(at) < std::time::Duration::from_secs(60))
            {
                event(EngineEvent::PluginsChecked {
                    request_id,
                    result: Ok(()),
                })
            } else {
                *plugin_check_at = Some(now);
                tokio::select! {
                    _ = &mut *shutdown => CommandResult::Shutdown,
                    result = api.check_plugins() => event(EngineEvent::PluginsChecked { request_id, result }),
                }
            }
        }
        EngineCommand::UpdatePlugin { id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.update_plugins(vec![id.clone()]) => event(EngineEvent::PluginUpdated { id, result }),
        },
        EngineCommand::LoadUsageLimits { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.usage_limits() => event(EngineEvent::UsageLimitsLoaded { request_id, result }),
        },
        EngineCommand::Compact { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.compact(&session_id) => match result {
                Ok(()) => event(EngineEvent::CompactionRequested { session_id }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not compact the session: {message}"),
                }),
            },
        },
        EngineCommand::LoadContext { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.context(&session_id) => match result {
                Ok(messages) => event(EngineEvent::ContextLoaded { session_id, messages }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not load the context: {message}"),
                }),
            },
        },
        EngineCommand::StageRevert {
            session_id,
            message_id,
            files,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.stage_revert(&session_id, message_id, files) => match result {
                Ok(revert) => event(EngineEvent::RevertStaged { session_id, revert }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not stage the revert: {message}"),
                }),
            },
        },
        EngineCommand::CommitRevert { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.commit_revert(&session_id) => match result {
                Ok(()) => event(EngineEvent::RevertCommitted {
                    session_id,
                    message_id: String::new(),
                }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not commit the revert: {message}"),
                }),
            },
        },
        EngineCommand::ClearRevert { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.clear_revert(&session_id) => match result {
                Ok(()) => event(EngineEvent::RevertCleared { session_id }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not clear the revert: {message}"),
                }),
            },
        },
        EngineCommand::CreateForm {
            session_id,
            title,
            fields,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.create_form(&session_id, title, fields) => match result {
                Ok(form) => event(EngineEvent::FormCreated { form }),
                Err(message) => event(EngineEvent::FormError {
                    message: format!("Could not create the form: {message}"),
                }),
            },
        },
        EngineCommand::LoadForms { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.forms(&session_id) => match result {
                Ok(forms) => event(EngineEvent::FormsLoaded { session_id, forms }),
                Err(message) => event(EngineEvent::FormError {
                    message: format!("Could not load forms: {message}"),
                }),
            },
        },
        EngineCommand::ReplyForm {
            session_id,
            form_id,
            answer,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.reply_form(&session_id, &form_id, answer) => match result {
                Ok(()) => event(EngineEvent::FormSettled { form_id }),
                Err(message) => event(EngineEvent::FormError {
                    message: format!("Could not answer the form: {message}"),
                }),
            },
        },
        EngineCommand::DismissForm {
            session_id,
            form_id,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.dismiss_form(&session_id, &form_id) => match result {
                Ok(()) => event(EngineEvent::FormSettled { form_id }),
                Err(message) => event(EngineEvent::FormError {
                    message: format!("Could not dismiss the form: {message}"),
                }),
            },
        },
        EngineCommand::LoadInbox { session_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.inbox(&session_id) => match result {
                Ok(items) => event(EngineEvent::InboxLoaded { session_id, items }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not load the inbox: {message}"),
                }),
            },
        },
        EngineCommand::CancelInbox {
            session_id,
            inbox_id,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = async {
                api.cancel_inbox(&session_id, &inbox_id).await?;
                api.inbox(&session_id).await
            } => match result {
                Ok(items) => event(EngineEvent::InboxLoaded { session_id, items }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not cancel the prompt: {message}"),
                }),
            },
        },
        EngineCommand::SetInboxDelivery {
            session_id,
            inbox_id,
            delivery,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = async {
                api.set_inbox_delivery(&session_id, &inbox_id, delivery).await?;
                api.inbox(&session_id).await
            } => match result {
                Ok(items) => event(EngineEvent::InboxLoaded { session_id, items }),
                Err(message) => event(EngineEvent::CommandFailed {
                    message: format!("Could not change the delivery: {message}"),
                }),
            },
        },
        EngineCommand::ForkSession { session_id, before } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.fork(&session_id, before) => event(match result {
                Ok(session) => EngineEvent::SessionForked { session },
                Err(message) => EngineEvent::CommandFailed {
                    message: format!("Could not fork the session: {message}"),
                },
            }),
        },
        EngineCommand::DeleteSession { id } => {
            let call_id = id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.delete(&call_id) => event(match result {
                    Ok(()) => {
                        tracing::debug!(session = %call_id, "deleted session");
                        EngineEvent::SessionDeleted { id }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not delete the session: {message}"),
                    },
                }),
            }
        }
        EngineCommand::LoadTranscript { session_id } => {
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.messages(&call_id) => event(match result {
                    Ok(messages) => {
                        tracing::debug!(
                            session = %call_id,
                            count = messages.len(),
                            "loaded transcript"
                        );
                        EngineEvent::TranscriptLoaded { session_id, messages }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not load the conversation: {message}"),
                    },
                }),
            }
        }
        EngineCommand::SendPrompt {
            session_id,
            request,
        } => {
            if request.text.trim().is_empty()
                && request.files.is_empty()
                && request.agents.is_empty()
                && request.skills.is_empty()
            {
                return event(EngineEvent::CommandFailed {
                    message: "A prompt cannot be empty.".into(),
                });
            }
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.prompt(&call_id, request) => event(match result {
                    Ok(message) => {
                        tracing::debug!(session = %call_id, "queued prompt");
                        EngineEvent::PromptAccepted { session_id, message }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not send the message: {message}"),
                    },
                }),
            }
        }
        EngineCommand::RunCommand {
            session_id,
            request,
        } => {
            if request.name.trim().is_empty() {
                return event(EngineEvent::CommandFailed {
                    message: "Choose a command before sending.".into(),
                });
            }
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.command(&call_id, request) => event(match result {
                    Ok(()) => {
                        tracing::debug!(session = %call_id, "ran command");
                        EngineEvent::CommandAccepted { session_id }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not run the command: {message}"),
                    },
                }),
            }
        }
        EngineCommand::SetSessionModel { session_id, model } => {
            let call_id = session_id.clone();
            let call_model = model.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.set_model(&call_id, call_model) => event(match result {
                    Ok(()) => {
                        tracing::debug!(session = %call_id, "changed model");
                        EngineEvent::SessionModelSet { session_id, model }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not change the model: {message}"),
                    },
                }),
            }
        }
        EngineCommand::SetSessionAgent { session_id, agent } => {
            let call_id = session_id.clone();
            let call_agent = agent.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.set_agent(&call_id, call_agent) => event(match result {
                    Ok(()) => {
                        tracing::debug!(session = %call_id, "changed agent");
                        EngineEvent::SessionAgentSet { session_id, agent }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not change the agent: {message}"),
                    },
                }),
            }
        }
        EngineCommand::FindFiles {
            request_id,
            query,
            limit,
        } => {
            let call_query = query.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.find_files(&call_query, limit) => event(EngineEvent::FilesFound {
                    request_id,
                    result,
                }),
            }
        }
        EngineCommand::LoadVcsStatus { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.vcs_status() => event(EngineEvent::VcsStatusLoaded { request_id, result }),
        },
        EngineCommand::LoadVcsBase { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.vcs_base() => event(EngineEvent::VcsBaseLoaded { request_id, result }),
        },
        EngineCommand::InitRepo { request_id } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.vcs_init() => event(EngineEvent::RepoInitialized { request_id, result }),
        },
        EngineCommand::LoadVcsDiff {
            request_id,
            mode,
            base,
            context,
            untracked,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.vcs_diff(&mode, base.as_deref(), context.as_deref(), &untracked) => {
                event(EngineEvent::VcsDiffLoaded { request_id, result })
            }
        },
        EngineCommand::LoadSessionDiff {
            request_id,
            session_id,
            from,
            to,
            context,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.session_diff(&session_id, from.as_deref(), to.as_deref(), context.as_deref()) => {
                event(EngineEvent::SessionDiffLoaded { request_id, result })
            }
        },
        EngineCommand::ListCatalog { request_id, kind } => {
            let load = async {
                match kind {
                    CatalogKind::Commands => api.commands().await.map(CatalogData::Commands),
                    CatalogKind::Skills => api.skills().await.map(CatalogData::Skills),
                    CatalogKind::Agents => api.agents().await.map(CatalogData::Agents),
                    CatalogKind::Models => api.models().await.map(CatalogData::Models),
                }
            };
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = load => event(EngineEvent::CatalogLoaded { request_id, kind, result }),
            }
        }
        EngineCommand::Interrupt { session_id } => {
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.interrupt(&call_id) => event(match result {
                    Ok(interrupted) => {
                        tracing::debug!(session = %call_id, interrupted, "interrupted turn");
                        EngineEvent::Interrupted { session_id, interrupted }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not stop the turn: {message}"),
                    },
                }),
            }
        }
        EngineCommand::LoadPermissions { session_id } => {
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.permissions(&call_id) => event(match result {
                    Ok(requests) => {
                        tracing::debug!(
                            session = %call_id,
                            count = requests.len(),
                            "loaded pending permissions"
                        );
                        EngineEvent::PermissionsLoaded { session_id, requests }
                    }
                    Err(message) => EngineEvent::CommandFailed {
                        message: format!("Could not load permission requests: {message}"),
                    },
                }),
            }
        }
        EngineCommand::ReplyPermission {
            session_id,
            request_id,
            reply,
        } => {
            let call_id = session_id.clone();
            let call_request = request_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.reply_permission(&call_id, &call_request, reply) => {
                    event(match result {
                        Ok(()) => {
                            tracing::debug!(
                                session = %call_id,
                                request = %call_request,
                                ?reply,
                                "answered permission request"
                            );
                            EngineEvent::PermissionReplied { session_id, request_id }
                        }
                        Err(message) => EngineEvent::CommandFailed {
                            message: format!("Could not answer the permission request: {message}"),
                        },
                    })
                }
            }
        }
        EngineCommand::CreatePty { request } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.create_pty(request) => match result {
                Ok(pty) => event(EngineEvent::PtyCreated { pty }),
                Err(message) => event(EngineEvent::PtyError {
                    id: String::new(),
                    message: format!("Could not create the terminal: {message}"),
                }),
            },
        },
        EngineCommand::ListPtys => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.ptys() => event(EngineEvent::PtysLoaded { result }),
        },
        EngineCommand::ResizePty { id, cols, rows } => {
            let call_id = id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.resize_pty(&call_id, cols, rows) => match result {
                    Ok(pty) => event(EngineEvent::PtyCreated { pty }),
                    Err(message) => event(EngineEvent::PtyError {
                        id,
                        message: format!("Could not resize the terminal: {message}"),
                    }),
                },
            }
        }
        EngineCommand::WritePty { id, bytes } => match pty_links.write(&id, bytes) {
            Ok(()) => CommandResult::None,
            Err(message) => event(EngineEvent::PtyError { id, message }),
        },
        EngineCommand::DeletePty { id } => {
            // Stop the reader before the server tears the process down, so the
            // close event has a single source.
            pty_links.close(&id);
            let call_id = id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.delete_pty(&call_id) => match result {
                    Ok(()) => event(EngineEvent::PtyClosed { id }),
                    Err(message) => event(EngineEvent::PtyError {
                        id,
                        message: format!("Could not delete the terminal: {message}"),
                    }),
                },
            }
        }
        EngineCommand::ConnectPty { id } => {
            if pty_links.is_connected(&id) {
                return CommandResult::None;
            }
            let ticket = tokio::select! {
                _ = &mut *shutdown => return CommandResult::Shutdown,
                result = api.pty_ticket(&id) => result,
            };
            let ticket = match ticket {
                Ok(ticket) => ticket,
                Err(message) => {
                    return event(EngineEvent::PtyError {
                        id,
                        message: format!("Could not authorize the terminal connection: {message}"),
                    });
                }
            };
            let url = match api.pty_connect_url(&id, &ticket.ticket, None) {
                Ok(url) => url,
                Err(message) => return event(EngineEvent::PtyError { id, message }),
            };
            let (input, input_rx) = async_channel::unbounded();
            let task = tokio::spawn(pump_pty(id.clone(), url, input_rx, event_tx.clone()));
            pty_links.insert(id, input, task);
            CommandResult::None
        }
        EngineCommand::ExportSession {
            session_id,
            sanitize,
            path,
        } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = write_export(api, &session_id, sanitize, &path) => match result {
                Ok(bytes) => event(EngineEvent::SessionExported {
                    session_id,
                    path,
                    bytes,
                }),
                Err(message) => event(EngineEvent::SessionExportError { message }),
            },
        },
        EngineCommand::PreviewImport { path } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = read_import(&path) => match result {
                Ok(transfer) => event(EngineEvent::ImportPreviewed { path, transfer }),
                Err(message) => event(EngineEvent::SessionImportError { message }),
            },
        },
        EngineCommand::ImportSession { path, copy } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = async {
                let transfer = read_import(&path).await?;
                let transfer = if copy {
                    crate::features::transfer::import_as_copy(transfer)
                } else {
                    transfer
                };
                api.import_session(&transfer).await
            } => match result {
                Ok(session) => event(EngineEvent::SessionImported { session }),
                Err(message) => event(EngineEvent::SessionImportError { message }),
            },
        },
        EngineCommand::LoadStats { request_id, range } => tokio::select! {
            _ = &mut *shutdown => CommandResult::Shutdown,
            result = api.session_stats(range) => event(EngineEvent::StatsLoaded { request_id, result }),
        },
    }
}

/// Fetch an export and write it to the user's file. The payload is written
/// pretty-printed so a person can inspect what leaves the machine.
async fn write_export(
    api: &SessionApi,
    session_id: &str,
    sanitize: bool,
    path: &Path,
) -> Result<u64, String> {
    let transfer = api.export_session(session_id, sanitize).await?;
    let bytes = serde_json::to_vec_pretty(&transfer)
        .map_err(|err| format!("Could not encode the export: {err}"))?;
    tokio::fs::write(path, &bytes)
        .await
        .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
    Ok(bytes.len() as u64)
}

/// Read and structurally parse an export file. The core layer owns the
/// product-level rules; this only rejects a file that is not an export at all.
async fn read_import(path: &Path) -> Result<harness_protocol::SessionTransfer, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|err| format!("Could not read {}: {err}", path.display()))?;
    let transfer = harness_protocol::SessionTransfer::parse(&bytes)?;
    if transfer.session_id().trim().is_empty() {
        return Err("This export has no session id.".into());
    }
    Ok(transfer)
}

/// Keep one SSE connection open, forwarding parsed updates to the UI.
///
/// Reconnects with exponential backoff; the server replays nothing, so the
/// view reloads the transcript when a turn ends or the stream reopens.
async fn pump_events(api: SessionApi, events: Sender<EngineEvent>) {
    let mut backoff = Duration::from_millis(250);
    // Only announce a drop once per outage, not on every reconnect attempt.
    let mut announced_down = false;

    loop {
        match api.subscribe().await {
            Ok(response) => {
                backoff = Duration::from_millis(250);
                announced_down = false;
                tracing::debug!("event stream connected");
                if events.send(EngineEvent::StreamConnected).await.is_err() {
                    return;
                }
                let mut stream = response.bytes_stream();
                let mut buffer = String::new();

                while let Some(chunk) = stream.next().await {
                    let Ok(bytes) = chunk else { break };
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(end) = buffer.find("\n\n") {
                        let frame: String = buffer.drain(..end + 2).collect();
                        let Some(data) = frame_data(&frame) else {
                            continue;
                        };
                        let Some(event) = events::parse_event(&data) else {
                            continue;
                        };
                        if events.send(EngineEvent::Stream(event)).await.is_err() {
                            return;
                        }
                    }
                }
                tracing::debug!("event stream disconnected");
                if !announced_down {
                    announced_down = true;
                    if events.send(EngineEvent::StreamDisconnected).await.is_err() {
                        return;
                    }
                }
            }
            Err(err) => {
                tracing::debug!(%err, "could not open the event stream");
                if !announced_down {
                    announced_down = true;
                    if events.send(EngineEvent::StreamDisconnected).await.is_err() {
                        return;
                    }
                }
            }
        }

        sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

/// The concatenated `data:` lines of one SSE frame, if it has any.
fn frame_data(frame: &str) -> Option<String> {
    let mut data = String::new();
    for line in frame.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest.trim_start());
    }
    (!data.is_empty()).then_some(data)
}

async fn stop(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn exit_message(status: io::Result<std::process::ExitStatus>) -> String {
    match status {
        Ok(status) => format!("OpenCode exited unexpectedly ({status})"),
        Err(err) => format!("OpenCode process error: {err}"),
    }
}

async fn read_version(binary: &Path) -> String {
    match Command::new(binary).arg("--version").output().await {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stdout);
            let raw = raw.trim();
            raw.strip_prefix("opencode ").unwrap_or(raw).to_string()
        }
        _ => "unknown".to_string(),
    }
}

fn pick_port() -> io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn random_password() -> String {
    format!("{:032x}", rand::random::<u128>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_poll_stops_at_timeout() {
        assert_eq!(
            poll_step(
                AUTH_POLL_LIMIT - Duration::from_secs(1),
                &AuthStatus::Pending
            ),
            PollStep::Wait
        );
        assert_eq!(
            poll_step(AUTH_POLL_LIMIT, &AuthStatus::Pending),
            PollStep::TimedOut
        );
        assert_eq!(
            poll_step(AUTH_POLL_LIMIT, &AuthStatus::Complete),
            PollStep::Finished,
            "a completed attempt wins over the elapsed cap"
        );
        assert_eq!(
            poll_step(Duration::from_secs(1), &AuthStatus::Failed),
            PollStep::Finished
        );
        assert_eq!(
            poll_step(
                Duration::from_secs(1),
                &AuthStatus::Unknown("queued".into())
            ),
            PollStep::Wait
        );
    }

    #[test]
    fn auth_status_events_report_completion_and_failure() {
        let completed = auth_status_events(
            "acme".into(),
            "att_1".into(),
            AttemptStatus {
                status: AuthStatus::Complete,
                message: None,
            },
        );
        let CommandResult::Events(batch) = completed else {
            panic!("expected a batch");
        };
        assert!(matches!(
            batch.as_ref(),
            [
                EngineEvent::AuthAttemptUpdated { .. },
                EngineEvent::ProviderConnected { .. }
            ]
        ));

        let failed = auth_status_events(
            "acme".into(),
            "att_1".into(),
            AttemptStatus {
                status: AuthStatus::Expired,
                message: None,
            },
        );
        let CommandResult::Events(batch) = failed else {
            panic!("expected a batch");
        };
        assert!(matches!(
            batch.as_ref(),
            [
                EngineEvent::AuthAttemptUpdated { .. },
                EngineEvent::AuthAttemptFailed { message, .. }
            ] if message.contains("expired")
        ));
    }
}
