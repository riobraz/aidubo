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

mod diff;
mod form;
mod inbox;
mod mcp;
mod permission_saved;
mod plugin;
mod provider;
mod pty;
mod revert;
mod schedule;
mod transfer;
mod update;
mod usage;
mod vcs;

pub use diff::*;
pub use form::*;
pub use inbox::*;
pub use mcp::*;
pub use permission_saved::*;
pub use plugin::*;
pub use provider::*;
pub use pty::*;
pub use revert::*;
pub use schedule::*;
pub use transfer::*;
pub use update::*;
pub use usage::*;
pub use vcs::*;

use serde::{Deserialize, Serialize};

/// Standard envelope used by the `/api` collection endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Envelope<T> {
    pub data: T,
}

/// Token counts for one session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub reasoning: f64,
    #[serde(default)]
    pub cache: TokenCache,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct TokenCache {
    #[serde(default)]
    pub read: f64,
    #[serde(default)]
    pub write: f64,
}

/// A session as returned by `GET /api/session`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    #[serde(default)]
    pub tokens: Option<TokenUsage>,
    #[serde(default)]
    pub cost: Option<f64>,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModelRef {
    pub id: String,
    #[serde(
        default,
        rename = "providerID",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

/// One entry returned by `GET /api/fs/find`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// One entry returned by `GET /api/command`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CommandInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// One entry returned by `GET /api/skill`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SkillInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// One entry returned by `GET /api/agent`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AgentInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

/// One entry returned by `GET /api/model`.
///
/// The server returns a large catalog with provider metadata we do not render;
/// only the identifying fields are modeled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ModelInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "modelID")]
    pub model_id: String,
    #[serde(default, rename = "providerID")]
    pub provider_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
}

impl ModelInfo {
    /// The model identifier the prompt API expects.
    pub fn wire_id(&self) -> &str {
        if self.model_id.is_empty() {
            &self.id
        } else {
            &self.model_id
        }
    }

    /// The reference used by `POST /api/session/{id}/model`.
    pub fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: self.wire_id().to_string(),
            provider_id: (!self.provider_id.is_empty()).then(|| self.provider_id.clone()),
            variant: None,
        }
    }
}

/// A catalog the client can list from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogKind {
    Commands,
    Skills,
    Agents,
    Models,
}

impl CatalogKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Skills => "skills",
            Self::Agents => "agents",
            Self::Models => "models",
        }
    }
}

/// Body for `POST /api/session/{id}/fork`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ForkRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
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

/// A range of the prompt text that names an attachment.
///
/// OpenCode's TypeScript clients use JavaScript string indices, so `start` and
/// `end` are UTF-16 code units (the range includes the sigil, `text` excludes
/// it). The end is exclusive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionRange {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Where a prompt file's content should come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FileSource {
    Uri { uri: String },
    Inline,
}

/// One file attached to a prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFile {
    /// Present in requests; the server echoes files without it and keeps the
    /// location inside `source` instead.
    #[serde(default)]
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FileSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention: Option<MentionRange>,
}

/// One agent attached to a prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAttachment {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention: Option<MentionRange>,
}

/// One skill attached to a prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillAttachment {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention: Option<MentionRange>,
}

/// Request body for `POST /api/session/{sessionID}/prompt`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PromptRequest {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<InboxDelivery>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentAttachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillAttachment>,
}

/// Request body for `POST /api/session/{sessionID}/command`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommandRequest {
    pub name: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentAttachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillAttachment>,
}

/// Request body for `POST /api/session/{sessionID}/model`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetModelRequest {
    pub model: ModelRef,
}

/// Request body for `POST /api/session/{sessionID}/agent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetAgentRequest {
    pub agent: String,
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
    /// Attachments the server stored with the message, echoed back in full.
    #[serde(default)]
    pub files: Vec<PromptFile>,
    #[serde(default)]
    pub agents: Vec<AgentAttachment>,
    #[serde(default)]
    pub skills: Vec<SkillAttachment>,
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
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    ExecutionStarted {
        session_id: String,
    },
    ExecutionFinished {
        session_id: String,
        /// The server's event id. Reconnect replay repeats it, so consumers can
        /// dedupe one-off effects such as notifications.
        event_id: String,
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
    /// The agent is waiting on a form answer.
    FormCreated {
        form: FormInfo,
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
    /// The agent started compacting the session.
    CompactionStarted {
        session_id: String,
    },
    /// Compaction finished.
    CompactionEnded {
        session_id: String,
    },
    /// Compaction failed.
    CompactionFailed {
        session_id: String,
        message: String,
    },
    /// The session's token and cost totals changed.
    UsageUpdated {
        session_id: String,
        tokens: TokenUsage,
        cost: Option<f64>,
    },
    /// The project's files changed on disk. The payload is not needed: the
    /// view refreshes its own snapshots.
    FilesystemChanged,
    /// An MCP server's status or resources changed. The list is reloaded
    /// instead of patching one entry, because the server is authoritative.
    McpChanged,
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
            Self::FormCreated { form } => &form.session_id,
            Self::CompactionStarted { session_id }
            | Self::CompactionEnded { session_id }
            | Self::CompactionFailed { session_id, .. } => session_id,
            Self::UsageUpdated { session_id, .. } => session_id,
            Self::FilesystemChanged | Self::McpChanged => "",
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
    fn session_parses_tokens_and_cost() {
        let json = r#"{"id":"s1","cost":0.42,"tokens":{"input":10,"output":5,"reasoning":2,"cache":{"read":3,"write":1}}}"#;
        let session: Session = serde_json::from_str(json).expect("session");
        let tokens = session.tokens.expect("tokens");
        assert_eq!(tokens.input, 10.0);
        assert_eq!(tokens.cache.read, 3.0);
        assert_eq!(session.cost, Some(0.42));
        let bare: Session = serde_json::from_str(r#"{"id":"s2"}"#).expect("bare");
        assert!(bare.tokens.is_none());
        assert!(bare.cost.is_none());
    }

    #[test]
    fn fork_request_serializes_before_and_omits_none() {
        let with = serde_json::to_string(&ForkRequest {
            before: Some("msg_1".into()),
        })
        .expect("json");
        assert_eq!(with, r#"{"before":"msg_1"}"#);
        let without = serde_json::to_string(&ForkRequest::default()).expect("json");
        assert_eq!(without, "{}");
    }

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
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"text": "hello"})
        );
    }

    #[test]
    fn prompt_request_serializes_attachments() {
        let request = PromptRequest {
            text: "read @README.md and $opencode".into(),
            delivery: None,
            files: vec![PromptFile {
                uri: "file:///tmp/README.md".into(),
                source: Some(FileSource::Uri {
                    uri: "file:///tmp/README.md".into(),
                }),
                name: Some("README.md".into()),
                mention: Some(MentionRange {
                    start: 5,
                    end: 15,
                    text: "README.md".into(),
                }),
            }],
            agents: vec![AgentAttachment {
                name: "build".into(),
                mention: None,
            }],
            skills: vec![SkillAttachment {
                id: "opencode".into(),
                mention: None,
            }],
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["text"], "read @README.md and $opencode");
        assert_eq!(value["files"][0]["uri"], "file:///tmp/README.md");
        assert_eq!(value["files"][0]["mention"]["start"], 5);
        assert_eq!(value["files"][0]["mention"]["end"], 15);
        assert_eq!(value["files"][0]["source"]["type"], "uri");
        assert_eq!(value["agents"][0], serde_json::json!({"name": "build"}));
        assert_eq!(value["skills"][0], serde_json::json!({"id": "opencode"}));
    }

    #[test]
    fn command_and_session_requests_serialize() {
        let command = CommandRequest {
            name: "review".into(),
            text: "HEAD~1".into(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            serde_json::json!({"name": "review", "text": "HEAD~1"})
        );

        let model = SetModelRequest {
            model: ModelRef {
                id: "gpt-5".into(),
                provider_id: Some("openai".into()),
                variant: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&model).unwrap(),
            serde_json::json!({"model": {"id": "gpt-5", "providerID": "openai"}})
        );

        let agent = SetAgentRequest {
            agent: "plan".into(),
        };
        assert_eq!(
            serde_json::to_value(&agent).unwrap(),
            serde_json::json!({"agent": "plan"})
        );
    }

    #[test]
    fn parses_catalog_entries_with_missing_optional_fields() {
        let files: Envelope<Vec<FileEntry>> = serde_json::from_str(
            r#"{"location":{"directory":"/tmp"},"data":[
                {"path":"src/main.rs","type":"file"},
                {"path":"src"},
                {"path":"x","type":"nonsense","future":true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(files.data[0].path, "src/main.rs");
        assert_eq!(files.data[0].kind, "file");
        assert_eq!(files.data[1].kind, "");
        assert_eq!(files.data[2].kind, "nonsense");

        let commands: Envelope<Vec<CommandInfo>> =
            serde_json::from_str(r#"{"data":[{"name":"init"},{"name":"review","extra":1}]}"#)
                .unwrap();
        assert_eq!(commands.data[0].description, "");
        assert_eq!(commands.data[1].name, "review");

        let skills: Envelope<Vec<SkillInfo>> =
            serde_json::from_str(r#"{"data":[{"id":"opencode","name":"OpenCode"}]}"#).unwrap();
        assert_eq!(skills.data[0].id, "opencode");
        assert_eq!(skills.data[0].description, "");

        let agents: Envelope<Vec<AgentInfo>> =
            serde_json::from_str(r#"{"data":[{"id":"build","name":"Build","mode":"primary"}]}"#)
                .unwrap();
        assert_eq!(agents.data[0].mode.as_deref(), Some("primary"));
        assert!(!agents.data[0].hidden);

        let models: Envelope<Vec<ModelInfo>> = serde_json::from_str(
            r#"{"data":[
                {"id":"liquid/lfm","modelID":"liquid/lfm","providerID":"openrouter","name":"LFM","enabled":true},
                {"id":"alias","name":"Alias"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(models.data[0].wire_id(), "liquid/lfm");
        assert_eq!(
            models.data[0].model_ref(),
            ModelRef {
                id: "liquid/lfm".into(),
                provider_id: Some("openrouter".into()),
                variant: None,
            }
        );
        assert_eq!(models.data[1].wire_id(), "alias");
        assert_eq!(models.data[1].model_ref().provider_id, None);
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
