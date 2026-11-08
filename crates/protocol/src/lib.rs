//! Wire types for the OpenCode HTTP API.
//!
//! These target the v2.x surface exposed by `opencode serve` (verified on
//! v2.2.6): the JSON API lives under `/api`, requires HTTP basic auth when
//! `OPENCODE_SERVER_PASSWORD` is set, and wraps collections in
//! `{ "data": ... }`. Messages are tagged by `type` and assistant output is a
//! discriminated `content` array.
//!
//! Newer builds also expose `/global/health`; [`Health`] models that payload so
//! the client can detect and use it when present.

use serde::{Deserialize, Serialize};

/// Standard envelope used by the `/api` collection endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Envelope<T> {
    pub data: T,
}

/// A session as returned by `GET /api/session`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "projectID")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<ModelRef>,
    #[serde(default)]
    pub time: SessionTime,
    #[serde(default)]
    pub location: Option<SessionLocation>,
}

/// Timestamps attached to a session, in epoch milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct SessionTime {
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
}

/// The model a session was last configured to use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelRef {
    pub id: String,
    #[serde(default, rename = "providerID")]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
}

/// The project directory a session belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionLocation {
    pub directory: String,
}

/// The response body of `GET /api/session`.
pub type SessionList = Envelope<Vec<Session>>;

/// Request body for `POST /api/session`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SessionLocation>,
}

/// Request body for `PATCH /api/session/{sessionID}`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Health payload exposed by newer OpenCode servers at `/global/health`.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub healthy: bool,
    pub version: String,
}

/// One message in a session timeline, as returned by
/// `GET /api/session/{sessionID}/message`.
///
/// Variants we do not model yet (agent switches, skills, shells, compaction)
/// collapse into [`SessionMessage::Other`] so a server upgrade cannot break
/// transcript loading.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionMessage {
    User {
        id: String,
        #[serde(default)]
        time: MessageTime,
        #[serde(default)]
        text: String,
    },
    Assistant {
        id: String,
        #[serde(default)]
        time: MessageTime,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        model: Option<ModelRef>,
        #[serde(default)]
        content: Vec<AssistantPart>,
        #[serde(default)]
        finish: Option<String>,
        #[serde(default)]
        error: Option<StructuredError>,
    },
    System {
        id: String,
        #[serde(default)]
        time: MessageTime,
        #[serde(default)]
        text: String,
    },
    Synthetic {
        id: String,
        #[serde(default)]
        time: MessageTime,
        #[serde(default)]
        text: String,
        #[serde(default)]
        description: Option<String>,
    },
    Idle {
        id: String,
        #[serde(default)]
        time: MessageTime,
        #[serde(default)]
        outcome: Option<String>,
        #[serde(default)]
        error: Option<StructuredError>,
    },
    #[serde(other)]
    Other,
}

impl SessionMessage {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::User { id, .. }
            | Self::Assistant { id, .. }
            | Self::System { id, .. }
            | Self::Synthetic { id, .. }
            | Self::Idle { id, .. } => Some(id),
            Self::Other => None,
        }
    }

    pub fn created(&self) -> i64 {
        match self {
            Self::User { time, .. }
            | Self::Assistant { time, .. }
            | Self::System { time, .. }
            | Self::Synthetic { time, .. }
            | Self::Idle { time, .. } => time.created,
            Self::Other => 0,
        }
    }
}

/// Timestamps attached to a message, in epoch milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct MessageTime {
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub streamed: Option<i64>,
    #[serde(default)]
    pub completed: Option<i64>,
}

/// One element of an assistant message's `content` array.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AssistantPart {
    Text {
        #[serde(default)]
        text: String,
    },
    Reasoning {
        #[serde(default)]
        text: String,
    },
    Tool {
        id: String,
        name: String,
        /// Whether OpenCode executed the tool itself rather than delegating it.
        #[serde(default)]
        executed: bool,
        #[serde(default)]
        state: ToolState,
    },
    #[serde(other)]
    Other,
}

/// Lifecycle of a tool call, as far as the transcript needs to show it.
///
/// `input` is kept as JSON text so a partially streamed argument string and a
/// parsed object share one representation. `json_text` accepts both shapes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ToolState {
    Streaming {
        #[serde(default, deserialize_with = "json_text")]
        input: String,
    },
    Running {
        #[serde(default, deserialize_with = "json_text")]
        input: String,
    },
    Completed {
        #[serde(default, deserialize_with = "json_text")]
        input: String,
        #[serde(default)]
        content: Vec<ToolContent>,
    },
    Error {
        #[serde(default, deserialize_with = "json_text")]
        input: String,
        error: StructuredError,
        #[serde(default)]
        content: Vec<ToolContent>,
    },
    #[default]
    #[serde(other)]
    Unknown,
}

/// One element of a tool result's `content` array.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ToolContent {
    Text {
        text: String,
    },
    File {
        uri: String,
        #[serde(default)]
        mime: String,
        #[serde(default)]
        name: Option<String>,
    },
    #[serde(other)]
    Other,
}

impl ToolContent {
    /// Text a transcript should show for this content item.
    pub fn display(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::File { uri, .. } => Some(uri),
            Self::Other => None,
        }
    }
}

/// Accept either a JSON string (streaming arguments) or an object (parsed
/// arguments) and normalize both to text.
fn json_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text,
        other => serde_json::to_string(&other).unwrap_or_default(),
    })
}

/// An error returned by the provider or the agent loop.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StructuredError {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub status: Option<u16>,
}

/// A page of messages, oldest first when requested with `order=asc`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MessagePage {
    #[serde(default)]
    pub data: Vec<SessionMessage>,
    #[serde(default)]
    pub cursor: Option<MessageCursor>,
}

/// Pagination cursors returned alongside a message page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MessageCursor {
    #[serde(default)]
    pub previous: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

/// Request body for `POST /api/session/{sessionID}/prompt`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PromptRequest {
    pub text: String,
}

/// The inbox message acknowledged by `POST /api/session/{sessionID}/prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcceptedPrompt {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(default)]
    pub time: MessageTime,
    #[serde(default)]
    pub payload: PromptPayload,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PromptPayload {
    #[serde(default)]
    pub text: String,
}

/// Response body of `POST /api/session/{sessionID}/interrupt`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InterruptResponse {
    #[serde(default)]
    pub interrupted: bool,
}

/// One pending permission request, as listed by
/// `GET /api/session/{sessionID}/permission` and emitted by
/// `permission.asked`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub resources: Vec<String>,
    /// Patterns OpenCode would remember for an "always" reply.
    #[serde(default)]
    pub save: Vec<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub source: Option<PermissionSource>,
}

/// Where a permission request came from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PermissionSource {
    Tool {
        #[serde(default, rename = "messageID")]
        message_id: Option<String>,
        #[serde(default)]
        id: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// The user's answer to a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionReply {
    Once,
    Always,
    Reject,
}

/// Request body for
/// `POST /api/session/{sessionID}/permission/{requestID}/reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionReplyRequest {
    pub decision: PermissionReply,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A live update from the server's SSE stream, reduced to what the transcript
/// needs. The parser lives in `harness-engine`; this stays IO-free so
/// `harness-core` can apply events without depending on the network stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    ExecutionStarted {
        session_id: String,
    },
    ExecutionFinished {
        session_id: String,
        outcome: ExecutionOutcome,
    },
    /// The agent started an assistant message, before any text arrives.
    AssistantStarted {
        session_id: String,
        assistant_message_id: String,
        started: i64,
        agent: Option<String>,
        model: Option<ModelRef>,
    },
    TextStarted {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
    },
    TextDelta {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
        delta: String,
    },
    /// The authoritative text for a finished text part.
    TextEnded {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
        text: String,
    },
    /// A reasoning part was created; text arrives as deltas.
    ReasoningStarted {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
    },
    ReasoningDelta {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
        delta: String,
    },
    /// The authoritative text for a finished reasoning part.
    ReasoningEnded {
        session_id: String,
        assistant_message_id: String,
        ordinal: u64,
        created: i64,
        text: String,
    },
    /// A tool call is being announced; arguments stream before it runs.
    ToolInputStarted {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        name: String,
    },
    ToolInputDelta {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        delta: String,
    },
    /// The complete argument JSON for a tool call.
    ToolInputEnded {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        input: String,
    },
    /// The agent invoked the tool; execution starts now.
    ToolCalled {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        input: String,
        executed: bool,
    },
    /// The tool reported progress without finishing.
    ToolProgress {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
    },
    ToolSucceeded {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        content: Vec<ToolContent>,
    },
    ToolFailed {
        session_id: String,
        assistant_message_id: String,
        tool_id: String,
        error: StructuredError,
        content: Vec<ToolContent>,
    },
    /// The turn is blocked until the user answers this request.
    PermissionAsked {
        request: PermissionRequest,
    },
    PermissionReplied {
        session_id: String,
        request_id: String,
        reply: PermissionReply,
    },
}

impl StreamEvent {
    pub fn session_id(&self) -> &str {
        match self {
            Self::ExecutionStarted { session_id }
            | Self::ExecutionFinished { session_id, .. }
            | Self::AssistantStarted { session_id, .. }
            | Self::TextStarted { session_id, .. }
            | Self::TextDelta { session_id, .. }
            | Self::TextEnded { session_id, .. }
            | Self::ReasoningStarted { session_id, .. }
            | Self::ReasoningDelta { session_id, .. }
            | Self::ReasoningEnded { session_id, .. }
            | Self::ToolInputStarted { session_id, .. }
            | Self::ToolInputDelta { session_id, .. }
            | Self::ToolInputEnded { session_id, .. }
            | Self::ToolCalled { session_id, .. }
            | Self::ToolProgress { session_id, .. }
            | Self::ToolSucceeded { session_id, .. }
            | Self::ToolFailed { session_id, .. }
            | Self::PermissionReplied { session_id, .. } => session_id,
            Self::PermissionAsked { request } => &request.session_id,
        }
    }
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Succeeded,
    Interrupted,
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_envelope() {
        let json = r#"{
            "data": [
                {
                    "id": "ses_1",
                    "title": "Refactor",
                    "projectID": "p1",
                    "agent": "build",
                    "model": {"id": "sonnet", "providerID": "anthropic", "variant": "default"},
                    "cost": 0,
                    "tokens": {"input": 0, "output": 0},
                    "time": {"created": 1000, "updated": 2000},
                    "location": {"directory": "/Users/me/project"}
                },
                {"id": "ses_2"}
            ]
        }"#;
        let list: SessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.data.len(), 2);

        let first = &list.data[0];
        assert_eq!(first.id, "ses_1");
        assert_eq!(first.title.as_deref(), Some("Refactor"));
        assert_eq!(first.project_id.as_deref(), Some("p1"));
        assert_eq!(first.agent.as_deref(), Some("build"));
        assert_eq!(first.time.created, 1000);
        assert_eq!(first.time.updated, 2000);
        assert_eq!(
            first.location.as_ref().map(|l| l.directory.as_str()),
            Some("/Users/me/project")
        );
        let model = first.model.as_ref().unwrap();
        assert_eq!(model.id, "sonnet");
        assert_eq!(model.provider_id.as_deref(), Some("anthropic"));
        assert_eq!(model.variant.as_deref(), Some("default"));

        let second = &list.data[1];
        assert_eq!(second.id, "ses_2");
        assert_eq!(second.project_id, None);
        assert_eq!(second.time, SessionTime::default());
    }

    #[test]
    fn create_request_omits_empty_fields() {
        let request = CreateSessionRequest {
            title: None,
            location: Some(SessionLocation {
                directory: "/tmp/project".into(),
            }),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"location": {"directory": "/tmp/project"}})
        );
        assert_eq!(
            serde_json::to_value(CreateSessionRequest::default()).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn update_request_serializes_title() {
        let request = UpdateSessionRequest {
            title: Some("New title".into()),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"title": "New title"})
        );
    }

    #[test]
    fn parses_health() {
        let health: Health = serde_json::from_str(r#"{"healthy":true,"version":"2.2.6"}"#).unwrap();
        assert!(health.healthy);
        assert_eq!(health.version, "2.2.6");
    }

    #[test]
    fn parses_a_message_page() {
        // Captured from OpenCode v2.2.6 after a turn that failed to reach the
        // provider.
        let json = r#"{
            "data": [
                {
                    "id": "msg_1",
                    "time": {"created": 1000},
                    "text": "hello",
                    "type": "user"
                },
                {
                    "id": "msg_2",
                    "time": {"created": 1001, "completed": 1200},
                    "type": "assistant",
                    "agent": "build",
                    "model": {"id": "liquid/lfm-3-1.7b", "providerID": "openrouter"},
                    "content": [{"type": "text", "text": "Hi"}],
                    "finish": "stop"
                },
                {
                    "id": "msg_3",
                    "time": {"created": 1002},
                    "type": "idle",
                    "outcome": "failed",
                    "error": {"type": "provider.quota", "message": "No credits", "status": 402}
                },
                {
                    "id": "msg_4",
                    "time": {"created": 1003},
                    "type": "agent-switched",
                    "agent": "plan"
                }
            ],
            "cursor": {"previous": null, "next": null}
        }"#;
        let page: MessagePage = serde_json::from_str(json).unwrap();
        assert_eq!(page.data.len(), 4);

        assert_eq!(page.data[0].id(), Some("msg_1"));
        assert!(matches!(
            &page.data[0],
            SessionMessage::User { text, .. } if text == "hello"
        ));

        let SessionMessage::Assistant {
            agent,
            model,
            content,
            finish,
            time,
            ..
        } = &page.data[1]
        else {
            panic!("expected an assistant message");
        };
        assert_eq!(agent.as_deref(), Some("build"));
        assert_eq!(
            model.as_ref().and_then(|m| m.provider_id.as_deref()),
            Some("openrouter")
        );
        assert_eq!(finish.as_deref(), Some("stop"));
        assert_eq!(time.completed, Some(1200));
        assert!(matches!(&content[0], AssistantPart::Text { text } if text == "Hi"));

        let SessionMessage::Idle { outcome, error, .. } = &page.data[2] else {
            panic!("expected an idle message");
        };
        assert_eq!(outcome.as_deref(), Some("failed"));
        let error = error.as_ref().unwrap();
        assert_eq!(error.kind, "provider.quota");
        assert_eq!(error.status, Some(402));

        // Unknown message kinds survive as `Other` instead of failing.
        assert_eq!(page.data[3], SessionMessage::Other);
    }

    #[test]
    fn parses_assistant_content_parts() {
        let json = r#"[
            {"type": "text", "text": "done"},
            {"type": "reasoning", "text": "thinking", "time": {"created": 1}},
            {"type": "tool", "id": "t1", "name": "bash", "executed": true,
             "state": {"status": "completed", "input": {"cmd": "ls"},
                       "content": [{"type": "text", "text": "ok"}],
                       "metadata": {}},
             "time": {"created": 2}},
            {"type": "image", "url": "x"}
        ]"#;
        let parts: Vec<AssistantPart> = serde_json::from_str(json).unwrap();
        assert!(matches!(&parts[0], AssistantPart::Text { text } if text == "done"));
        assert!(matches!(&parts[1], AssistantPart::Reasoning { text } if text == "thinking"));
        let AssistantPart::Tool {
            id,
            name,
            executed,
            state,
        } = &parts[2]
        else {
            panic!("expected a tool part");
        };
        assert_eq!(id, "t1");
        assert_eq!(name, "bash");
        assert!(*executed);
        assert!(matches!(
            state,
            ToolState::Completed { input, content }
                if input == r#"{"cmd":"ls"}"#
                    && content == &vec![ToolContent::Text { text: "ok".into() }]
        ));
        assert_eq!(parts[3], AssistantPart::Other);
    }

    #[test]
    fn parses_every_tool_state() {
        // Verbatim shapes captured from OpenCode v2.2.6.
        let streaming: ToolState =
            serde_json::from_str(r#"{"status": "streaming", "input": "{\"path\":\"/tmp/he"}"#)
                .unwrap();
        assert!(matches!(
            streaming,
            ToolState::Streaming { input } if input == r#"{"path":"/tmp/he"#
        ));

        let running: ToolState =
            serde_json::from_str(r#"{"status":"running","input":{"path":"/tmp/a"},"metadata":{}}"#)
                .unwrap();
        assert!(matches!(
            running,
            ToolState::Running { input } if input == r#"{"path":"/tmp/a"}"#
        ));

        let completed: ToolState = serde_json::from_str(
            r#"{"status":"completed","input":{"path":"/tmp/a"},
                "content":[{"type":"text","text":"hello"},
                           {"type":"file","uri":"file:///tmp/a","mime":"text/plain"}]}"#,
        )
        .unwrap();
        let ToolState::Completed { input, content } = completed else {
            panic!("expected a completed state");
        };
        assert_eq!(input, r#"{"path":"/tmp/a"}"#);
        assert_eq!(content[0].display(), Some("hello"));
        assert_eq!(content[1].display(), Some("file:///tmp/a"));

        let failed: ToolState = serde_json::from_str(
            r#"{"status":"error","input":{"path":"/nope"},
                "error":{"type":"tool.execution","message":"not found"},
                "content":[{"type":"text","text":"details"}]}"#,
        )
        .unwrap();
        assert!(matches!(
            failed,
            ToolState::Error { error, content, .. }
                if error.message == "not found" && content.len() == 1
        ));

        let unknown: ToolState = serde_json::from_str(r#"{"status":"paused","input":{}}"#).unwrap();
        assert_eq!(unknown, ToolState::Unknown);
    }

    #[test]
    fn prompt_request_serializes_text_only() {
        let request = PromptRequest {
            text: "hello".into(),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"text": "hello"})
        );
    }

    #[test]
    fn parses_prompt_acceptance_and_interrupt() {
        let accepted: Envelope<AcceptedPrompt> = serde_json::from_str(
            r#"{"data":{"id":"msg_1","sessionID":"ses_1",
                 "time":{"created":5},"type":"user",
                 "payload":{"text":"hello"},"delivery":"steer"}}"#,
        )
        .unwrap();
        assert_eq!(accepted.data.id, "msg_1");
        assert_eq!(accepted.data.session_id, "ses_1");
        assert_eq!(accepted.data.payload.text, "hello");
        assert_eq!(accepted.data.time.created, 5);

        let interrupt: InterruptResponse = serde_json::from_str(r#"{"interrupted":true}"#).unwrap();
        assert!(interrupt.interrupted);
    }

    #[test]
    fn parses_permission_requests() {
        // Captured from OpenCode v2.2.6.
        let json = r#"{"data":[{
            "id":"per_1bcd6ee39001Pr1u4fmvsdkUHT",
            "sessionID":"ses_1",
            "action":"shell",
            "resources":["echo hello-from-bash"],
            "save":["echo *"],
            "source":{"type":"tool","messageID":"msg_2","id":"call_bash"}
        }]}"#;
        let list: Envelope<Vec<PermissionRequest>> = serde_json::from_str(json).unwrap();
        let request = &list.data[0];
        assert_eq!(request.id, "per_1bcd6ee39001Pr1u4fmvsdkUHT");
        assert_eq!(request.session_id, "ses_1");
        assert_eq!(request.action, "shell");
        assert_eq!(request.resources, ["echo hello-from-bash"]);
        assert_eq!(request.save, ["echo *"]);
        assert_eq!(
            request.source,
            Some(PermissionSource::Tool {
                message_id: Some("msg_2".into()),
                id: Some("call_bash".into()),
            })
        );
    }

    #[test]
    fn serializes_permission_replies() {
        for (reply, expected) in [
            (PermissionReply::Once, r#"{"decision":"once"}"#),
            (PermissionReply::Always, r#"{"decision":"always"}"#),
            (PermissionReply::Reject, r#"{"decision":"reject"}"#),
        ] {
            let request = PermissionReplyRequest {
                decision: reply,
                message: None,
            };
            assert_eq!(serde_json::to_string(&request).unwrap(), expected);
        }
    }
}
