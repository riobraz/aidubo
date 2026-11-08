//! The sidecar supervisor: launch, wait for readiness, run commands, stop.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures_util::StreamExt as _;
use harness_protocol::{
    AcceptedPrompt, PermissionRequest, Session, SessionList, SessionMessage, StreamEvent,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep};

use crate::api::SessionApi;
use crate::events;
use crate::{EngineCommand, EngineConfig, locate};

/// Lifecycle events and command results emitted by the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    let binary = locate::locate_binary(config.binary.as_deref()).map_err(|err| err.to_string())?;
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
        .current_dir(&config.project_dir)
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
    match run_command(&api, EngineCommand::ListSessions, &mut shutdown).await {
        CommandResult::Event(event) => {
            let _ = events.send(*event).await;
        }
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

                match run_command(&api, command, &mut shutdown).await {
                    CommandResult::Event(event) => {
                        let _ = events.send(*event).await;
                    }
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
    Shutdown,
}

fn event(event: EngineEvent) -> CommandResult {
    CommandResult::Event(Box::new(event))
}

async fn run_command(
    api: &SessionApi,
    command: EngineCommand,
    shutdown: &mut oneshot::Receiver<()>,
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
        EngineCommand::SendPrompt { session_id, text } => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return event(EngineEvent::CommandFailed {
                    message: "A prompt cannot be empty.".into(),
                });
            }
            let call_id = session_id.clone();
            tokio::select! {
                _ = &mut *shutdown => CommandResult::Shutdown,
                result = api.prompt(&call_id, text) => event(match result {
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
    }
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
