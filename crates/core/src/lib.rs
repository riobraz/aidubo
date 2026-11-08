//! UI-facing state with no GPUI or network dependency.
//!
//! Keeping these types pure makes the interesting transitions testable in a
//! plain `cargo test` run.

use harness_protocol::{
    AssistantPart, ExecutionOutcome, ModelRef, PermissionRequest, Session, SessionMessage,
    StreamEvent, StructuredError, ToolContent, ToolState,
};

/// Lifecycle of the OpenCode sidecar as the UI needs to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    /// Locating the binary and launching the process.
    Starting { port: Option<u16> },
    /// The server answered an authenticated request.
    Ready { version: String, port: u16 },
    /// Startup failed; the message is safe to show to the user.
    Failed { message: String },
    /// The engine was asked to stop, or exited cleanly.
    Stopped,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self::Starting { port: None }
    }
}

impl EngineStatus {
    /// Short label suitable for a status badge.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Starting { .. } => "Connecting",
            Self::Ready { .. } => "Connected",
            Self::Failed { .. } => "Engine failed",
            Self::Stopped => "Stopped",
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Sessions mirrored from the engine, newest activity first.
///
/// The store owns selection so the UI and tests share one set of rules: a
/// selected session survives a refresh, and removing the selected session falls
/// back to the first remaining session.
#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: Vec<Session>,
    selected: Option<String>,
}

impl SessionStore {
    /// Replace the list with a server snapshot.
    pub fn replace(&mut self, mut sessions: Vec<Session>) {
        sort_newest_first(&mut sessions);
        self.sessions = sessions;

        let selected_survived = self
            .selected
            .as_ref()
            .is_some_and(|id| self.sessions.iter().any(|session| &session.id == id));
        if !selected_survived {
            self.selected = self.sessions.first().map(|session| session.id.clone());
        }
    }

    /// Insert a new session or replace an existing one.
    pub fn upsert(&mut self, session: Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
        sort_newest_first(&mut self.sessions);
    }

    /// Remove a session, selecting the first remaining one if it was selected.
    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|session| session.id != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.sessions.first().map(|session| session.id.clone());
        }
    }

    /// Change a session's title in place. Ordering is left alone.
    pub fn rename(&mut self, id: &str, title: impl Into<String>) {
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.title = Some(title.into());
        }
    }

    /// Select a session if it is present.
    pub fn select(&mut self, id: &str) {
        if self.sessions.iter().any(|session| session.id == id) {
            self.selected = Some(id.to_owned());
        }
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn selected(&self) -> Option<&Session> {
        let id = self.selected.as_deref()?;
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

fn sort_newest_first(sessions: &mut [Session]) {
    // Stable: sessions with the same timestamp keep the server's order.
    sessions.sort_by_key(|session| std::cmp::Reverse(session.time.updated));
}

/// The open session's conversation, mirroring the server timeline.
///
/// Entries are ordered by server timestamps and updated by ID, so a late
/// snapshot or a repeated event can never duplicate a message. Streaming text
/// arrives as deltas keyed by message and part ordinal; the authoritative text
/// replaces the accumulated buffer when the part ends.
#[derive(Debug, Default)]
pub struct TranscriptStore {
    session_id: Option<String>,
    entries: Vec<Entry>,
    busy: bool,
    loading: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User {
        id: String,
        text: String,
        created: i64,
    },
    Assistant(AssistantEntry),
    System {
        id: String,
        text: String,
        created: i64,
    },
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Self::User { id, .. } | Self::System { id, .. } => id,
            Self::Assistant(assistant) => &assistant.id,
        }
    }

    pub fn created(&self) -> i64 {
        match self {
            Self::User { created, .. } | Self::System { created, .. } => *created,
            Self::Assistant(assistant) => assistant.created,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantEntry {
    pub id: String,
    pub created: i64,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub parts: Vec<Content>,
    pub error: Option<String>,
}

impl AssistantEntry {
    /// The streaming text buffer for one part, created on first use.
    fn text_mut(&mut self, ordinal: u64) -> &mut String {
        let position = match self
            .parts
            .iter()
            .position(|part| matches!(part, Content::Text { ordinal: o, .. } if *o == ordinal))
        {
            Some(position) => position,
            None => {
                self.parts.push(Content::Text {
                    ordinal,
                    text: String::new(),
                });
                self.parts.len() - 1
            }
        };
        match &mut self.parts[position] {
            Content::Text { text, .. } => text,
            _ => unreachable!("position was matched as a text part"),
        }
    }

    /// The streaming reasoning buffer for one part, created on first use.
    fn reasoning_mut(&mut self, ordinal: u64) -> &mut String {
        let position =
            match self.parts.iter().position(
                |part| matches!(part, Content::Reasoning { ordinal: o, .. } if *o == ordinal),
            ) {
                Some(position) => position,
                None => {
                    self.parts.push(Content::Reasoning {
                        ordinal,
                        text: String::new(),
                    });
                    self.parts.len() - 1
                }
            };
        match &mut self.parts[position] {
            Content::Reasoning { text, .. } => text,
            _ => unreachable!("position was matched as a reasoning part"),
        }
    }

    fn tool_mut(&mut self, id: &str) -> Option<&mut ToolCall> {
        self.parts.iter_mut().find_map(|part| match part {
            Content::Tool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
    }

    /// The tool call with this ID, created if an event arrived before its
    /// `input.started` frame (for example after a stream reconnect).
    fn tool_entry(&mut self, id: &str) -> &mut ToolCall {
        let known = self
            .parts
            .iter()
            .any(|part| matches!(part, Content::Tool(tool) if tool.id == id));
        if !known {
            self.parts.push(Content::Tool(ToolCall {
                id: id.to_owned(),
                name: String::new(),
                status: ToolStatus::Streaming,
                input: String::new(),
                output: None,
                error: None,
            }));
        }
        self.tool_mut(id).expect("tool was just ensured")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text { ordinal: u64, text: String },
    Reasoning { ordinal: u64, text: String },
    Tool(ToolCall),
}

/// A tool call in the transcript. Every field is updated in place as live
/// events arrive, so the card never duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub status: ToolStatus,
    /// Argument JSON: partial while streaming, compact once parsed.
    pub input: String,
    /// Joined text from the tool's result content.
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Streaming,
    Running,
    Completed,
    Error,
    Unknown,
}

impl ToolStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Running => "running",
            Self::Completed => "done",
            Self::Error => "failed",
            Self::Unknown => "…",
        }
    }
}

impl TranscriptStore {
    /// Point the store at a session; the next snapshot replaces the contents.
    pub fn begin(&mut self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if self.session_id.as_deref() == Some(session_id.as_str()) {
            return;
        }
        self.session_id = Some(session_id);
        self.entries.clear();
        self.busy = false;
        self.loading = true;
        self.error = None;
    }

    /// Replace the timeline with a server snapshot. Ignores stale responses for
    /// a session the user has since left.
    pub fn apply_snapshot(&mut self, session_id: &str, messages: Vec<SessionMessage>) -> bool {
        if self.session_id.as_deref() != Some(session_id) {
            return false;
        }
        self.entries = entries_from(messages);
        self.loading = false;
        true
    }

    /// Record the user message the server admitted.
    pub fn accept_prompt(&mut self, session_id: &str, id: &str, text: &str, created: i64) -> bool {
        if self.session_id.as_deref() != Some(session_id) {
            return false;
        }
        match self.entries.iter_mut().find(|entry| entry.id() == id) {
            Some(Entry::User {
                text: existing,
                created: existing_created,
                ..
            }) => {
                *existing = text.to_owned();
                *existing_created = created;
            }
            Some(_) => return false,
            None => {
                self.entries.push(Entry::User {
                    id: id.to_owned(),
                    text: text.to_owned(),
                    created,
                });
                self.sort_entries();
            }
        }
        true
    }

    /// Apply one live event. Returns whether the view needs to redraw.
    pub fn apply_stream(&mut self, event: StreamEvent) -> bool {
        if self.session_id.as_deref() != Some(event.session_id()) {
            return false;
        }

        match event {
            StreamEvent::ExecutionStarted { .. } => {
                self.busy = true;
                self.error = None;
                true
            }
            StreamEvent::ExecutionFinished { outcome, .. } => {
                self.busy = false;
                if let ExecutionOutcome::Failed { message } = outcome {
                    self.error = Some(message);
                }
                true
            }
            StreamEvent::AssistantStarted {
                assistant_message_id,
                started,
                agent,
                model,
                ..
            } => {
                let model = model.map(display_model);
                let assistant = self.assistant_mut(&assistant_message_id, started);
                if assistant.agent.is_none() {
                    assistant.agent = agent;
                }
                if assistant.model.is_none() {
                    assistant.model = model;
                }
                true
            }
            StreamEvent::TextStarted {
                assistant_message_id,
                ordinal,
                created,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, created)
                    .text_mut(ordinal);
                true
            }
            StreamEvent::TextDelta {
                assistant_message_id,
                ordinal,
                created,
                delta,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, created)
                    .text_mut(ordinal)
                    .push_str(&delta);
                true
            }
            StreamEvent::TextEnded {
                assistant_message_id,
                ordinal,
                created,
                text,
                ..
            } => {
                *self
                    .assistant_mut(&assistant_message_id, created)
                    .text_mut(ordinal) = text;
                true
            }
            StreamEvent::ReasoningStarted {
                assistant_message_id,
                ordinal,
                created,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, created)
                    .reasoning_mut(ordinal);
                true
            }
            StreamEvent::ReasoningDelta {
                assistant_message_id,
                ordinal,
                created,
                delta,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, created)
                    .reasoning_mut(ordinal)
                    .push_str(&delta);
                true
            }
            StreamEvent::ReasoningEnded {
                assistant_message_id,
                ordinal,
                created,
                text,
                ..
            } => {
                *self
                    .assistant_mut(&assistant_message_id, created)
                    .reasoning_mut(ordinal) = text;
                true
            }
            StreamEvent::ToolInputStarted {
                assistant_message_id,
                tool_id,
                name,
                ..
            } => {
                let tool = self
                    .assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id);
                if tool.name.is_empty() {
                    tool.name = name;
                }
                tool.status = ToolStatus::Streaming;
                true
            }
            StreamEvent::ToolInputDelta {
                assistant_message_id,
                tool_id,
                delta,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id)
                    .input
                    .push_str(&delta);
                true
            }
            StreamEvent::ToolInputEnded {
                assistant_message_id,
                tool_id,
                input,
                ..
            } => {
                let tool = self
                    .assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id);
                tool.input = input;
                true
            }
            StreamEvent::ToolCalled {
                assistant_message_id,
                tool_id,
                input,
                ..
            } => {
                let tool = self
                    .assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id);
                tool.input = input;
                tool.status = ToolStatus::Running;
                true
            }
            StreamEvent::ToolProgress {
                assistant_message_id,
                tool_id,
                ..
            } => {
                self.assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id)
                    .status = ToolStatus::Running;
                true
            }
            StreamEvent::ToolSucceeded {
                assistant_message_id,
                tool_id,
                content,
                ..
            } => {
                let tool = self
                    .assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id);
                tool.status = ToolStatus::Completed;
                tool.output = tool_output(content);
                tool.error = None;
                true
            }
            StreamEvent::ToolFailed {
                assistant_message_id,
                tool_id,
                error,
                content,
                ..
            } => {
                let tool = self
                    .assistant_mut(&assistant_message_id, 0)
                    .tool_entry(&tool_id);
                tool.status = ToolStatus::Error;
                tool.output = tool_output(content);
                tool.error = tool_error(error);
                true
            }
            // Permissions are session state, not transcript state; the view
            // tracks them in a PermissionQueue.
            StreamEvent::PermissionAsked { .. } | StreamEvent::PermissionReplied { .. } => false,
        }
    }

    fn assistant_mut(&mut self, id: &str, created: i64) -> &mut AssistantEntry {
        let position = self
            .entries
            .iter()
            .position(|entry| matches!(entry, Entry::Assistant(assistant) if assistant.id == id));
        let position = match position {
            Some(position) => position,
            None => {
                // Fall back to the newest entry's timestamp when the event did
                // not carry one, so the message still lands last.
                let created = if created > 0 {
                    created
                } else {
                    self.entries.last().map(Entry::created).unwrap_or(0)
                };
                self.entries.push(Entry::Assistant(AssistantEntry {
                    id: id.to_owned(),
                    created,
                    agent: None,
                    model: None,
                    parts: Vec::new(),
                    error: None,
                }));
                self.sort_entries();
                self.entries
                    .iter()
                    .position(
                        |entry| matches!(entry, Entry::Assistant(assistant) if assistant.id == id),
                    )
                    .expect("just inserted")
            }
        };
        match &mut self.entries[position] {
            Entry::Assistant(assistant) => assistant,
            _ => unreachable!("position was matched as an assistant entry"),
        }
    }

    fn sort_entries(&mut self) {
        self.entries.sort_by_key(Entry::created);
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }
}

/// Pending permission requests for the selected session, oldest first.
///
/// The queue is session-scoped like the transcript: requests that arrive for a
/// session the user has left are dropped, and each request appears once even if
/// both a live event and a snapshot report it.
#[derive(Debug, Default)]
pub struct PermissionQueue {
    session_id: Option<String>,
    requests: Vec<PermissionRequest>,
}

impl PermissionQueue {
    /// Point the queue at a session, clearing it when the selection changes.
    pub fn begin(&mut self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if self.session_id.as_deref() == Some(session_id.as_str()) {
            return;
        }
        self.session_id = Some(session_id);
        self.requests.clear();
    }

    /// Replace the queue with the server's snapshot. Stale responses for a
    /// session the user has since left are ignored.
    pub fn replace(&mut self, session_id: &str, requests: Vec<PermissionRequest>) -> bool {
        if self.session_id.as_deref() != Some(session_id) {
            return false;
        }
        if self.requests == requests {
            return false;
        }
        self.requests = requests;
        true
    }

    /// Add a request, replacing any earlier copy with the same ID.
    pub fn push(&mut self, request: PermissionRequest) -> bool {
        if self.session_id.as_deref() != Some(request.session_id.as_str()) {
            return false;
        }
        match self
            .requests
            .iter_mut()
            .find(|known| known.id == request.id)
        {
            Some(known) => {
                if *known == request {
                    return false;
                }
                *known = request;
            }
            None => self.requests.push(request),
        }
        true
    }

    /// Drop a request once it has been answered.
    pub fn remove(&mut self, request_id: &str) -> bool {
        let before = self.requests.len();
        self.requests.retain(|request| request.id != request_id);
        self.requests.len() != before
    }

    /// The oldest pending request; the one the UI should present.
    pub fn front(&self) -> Option<&PermissionRequest> {
        self.requests.first()
    }

    pub fn contains(&self, request_id: &str) -> bool {
        self.requests.iter().any(|request| request.id == request_id)
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

fn entries_from(messages: Vec<SessionMessage>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for message in messages {
        match message {
            SessionMessage::User { id, time, text } => entries.push(Entry::User {
                id,
                text,
                created: time.created,
            }),
            SessionMessage::Assistant {
                id,
                time,
                agent,
                model,
                content,
                error,
                ..
            } => entries.push(Entry::Assistant(AssistantEntry {
                id,
                created: time.created,
                agent,
                model: model.map(display_model),
                parts: content
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, part)| match part {
                        AssistantPart::Text { text } => Some(Content::Text {
                            ordinal: index as u64,
                            text,
                        }),
                        AssistantPart::Reasoning { text } => Some(Content::Reasoning {
                            ordinal: index as u64,
                            text,
                        }),
                        AssistantPart::Tool {
                            id, name, state, ..
                        } => Some(Content::Tool(tool_from_state(id, name, state))),
                        AssistantPart::Other => None,
                    })
                    .collect(),
                error: error.and_then(transcript_error),
            })),
            SessionMessage::System { id, time, text } => entries.push(Entry::System {
                id,
                text,
                created: time.created,
            }),
            SessionMessage::Synthetic { .. }
            | SessionMessage::Idle { .. }
            | SessionMessage::Other => {}
        }
    }
    entries.sort_by_key(Entry::created);
    entries
}

fn tool_from_state(id: String, name: String, state: ToolState) -> ToolCall {
    let (status, input, output, error) = match state {
        ToolState::Streaming { input } => (ToolStatus::Streaming, input, None, None),
        ToolState::Running { input } => (ToolStatus::Running, input, None, None),
        ToolState::Completed { input, content } => {
            (ToolStatus::Completed, input, tool_output(content), None)
        }
        ToolState::Error {
            input,
            error,
            content,
        } => (
            ToolStatus::Error,
            input,
            tool_output(content),
            tool_error(error),
        ),
        ToolState::Unknown => (ToolStatus::Unknown, String::new(), None, None),
    };
    ToolCall {
        id,
        name,
        status,
        input,
        output,
        error,
    }
}

/// Join the text items of a tool result; `None` when there is nothing to show.
fn tool_output(content: Vec<ToolContent>) -> Option<String> {
    let text = content
        .iter()
        .filter_map(ToolContent::display)
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// The error text worth surfacing for a stored assistant message.
///
/// OpenCode records a `{ "type": "aborted" }` error on a turn the user stopped
/// on purpose; showing that would make a deliberate Stop look like a failure.
fn transcript_error(error: StructuredError) -> Option<String> {
    if error.kind == "aborted" || error.message.trim().is_empty() {
        return None;
    }
    Some(error.message)
}

/// The error text worth surfacing on a tool card.
///
/// Unlike a turn-level error, an `aborted` tool error is often the user's own
/// doing: declining a permission request fails the tool with
/// "The user declined this tool call", which the card must show.
fn tool_error(error: StructuredError) -> Option<String> {
    if error.message.trim().is_empty() {
        return None;
    }
    Some(error.message)
}

/// `provider/model (variant)`, the label shown above an assistant turn.
fn display_model(model: ModelRef) -> String {
    let provider = model.provider_id.unwrap_or_default();
    let mut name = if provider.is_empty() {
        model.id
    } else {
        format!("{provider}/{}", model.id)
    };
    if let Some(variant) = model.variant.filter(|variant| !variant.is_empty()) {
        name = format!("{name} ({variant})");
    }
    name
}

/// Current wall-clock time in epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// A compact "3m ago" label for a session timestamp.
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    if then_ms <= 0 {
        return String::new();
    }
    let seconds = now_ms.saturating_sub(then_ms).max(0) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        86_400..=604_799 => format!("{}d ago", seconds / 86_400),
        604_800..=2_591_999 => format!("{}w ago", seconds / 604_800),
        2_592_000..=31_535_999 => format!("{}mo ago", seconds / 2_592_000),
        _ => format!("{}y ago", seconds / 31_536_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::SessionTime;

    fn session(id: &str, updated: i64) -> Session {
        Session {
            id: id.into(),
            title: Some(format!("Session {id}")),
            project_id: Some("p1".into()),
            agent: None,
            model: None,
            time: SessionTime {
                created: updated,
                updated,
            },
            location: None,
        }
    }

    #[test]
    fn defaults_to_starting() {
        assert_eq!(
            EngineStatus::default(),
            EngineStatus::Starting { port: None }
        );
    }

    #[test]
    fn labels_match_state() {
        let ready = EngineStatus::Ready {
            version: "v2.1.4".into(),
            port: 4096,
        };
        assert_eq!(ready.label(), "Connected");
        assert!(ready.is_ready());
        assert!(!ready.is_failed());

        let failed = EngineStatus::Failed {
            message: "boom".into(),
        };
        assert_eq!(failed.label(), "Engine failed");
        assert!(failed.is_failed());
    }

    #[test]
    fn replace_sorts_newest_first_and_selects_first() {
        let mut store = SessionStore::default();
        store.replace(vec![
            session("old", 10),
            session("new", 30),
            session("mid", 20),
        ]);

        let ids: Vec<_> = store.sessions().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
        assert_eq!(store.selected_id(), Some("new"));
    }

    #[test]
    fn replace_keeps_a_live_selection() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10), session("b", 20)]);
        store.select("a");
        store.replace(vec![session("b", 30), session("a", 10)]);
        assert_eq!(store.selected_id(), Some("a"));
    }

    #[test]
    fn replace_drops_a_selection_that_is_gone() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10), session("b", 20)]);
        store.select("a");
        store.replace(vec![session("b", 20)]);
        assert_eq!(store.selected_id(), Some("b"));
    }

    #[test]
    fn upsert_inserts_and_sorts() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10)]);
        store.upsert(session("b", 20));
        let ids: Vec<_> = store.sessions().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn upsert_replaces_without_duplicating() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10)]);
        let mut updated = session("a", 10);
        updated.title = Some("Renamed".into());
        store.upsert(updated);
        assert_eq!(store.len(), 1);
        assert_eq!(store.sessions()[0].title.as_deref(), Some("Renamed"));
    }

    #[test]
    fn remove_selected_falls_back_to_the_first_remaining() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10), session("b", 20)]);
        store.select("b");
        store.remove("b");
        assert_eq!(store.selected_id(), Some("a"));

        store.remove("a");
        assert_eq!(store.selected_id(), None);
        assert!(store.is_empty());
    }

    #[test]
    fn select_ignores_unknown_ids() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10)]);
        store.select("missing");
        assert_eq!(store.selected_id(), Some("a"));
    }

    #[test]
    fn rename_updates_in_place() {
        let mut store = SessionStore::default();
        store.replace(vec![session("a", 10)]);
        store.rename("a", "New title");
        assert_eq!(store.sessions()[0].title.as_deref(), Some("New title"));
    }

    fn user_message(id: &str, text: &str, created: i64) -> SessionMessage {
        SessionMessage::User {
            id: id.into(),
            time: harness_protocol::MessageTime {
                created,
                streamed: None,
                completed: None,
            },
            text: text.into(),
        }
    }

    fn assistant_message(id: &str, created: i64, content: Vec<AssistantPart>) -> SessionMessage {
        SessionMessage::Assistant {
            id: id.into(),
            time: harness_protocol::MessageTime {
                created,
                streamed: None,
                completed: Some(created + 10),
            },
            agent: Some("build".into()),
            model: Some(ModelRef {
                id: "sonnet".into(),
                provider_id: Some("anthropic".into()),
                variant: None,
            }),
            content,
            finish: Some("stop".into()),
            error: None,
        }
    }

    fn delta(id: &str, ordinal: u64, created: i64, text: &str) -> StreamEvent {
        StreamEvent::TextDelta {
            session_id: "ses_1".into(),
            assistant_message_id: id.into(),
            ordinal,
            created,
            delta: text.into(),
        }
    }

    #[test]
    fn snapshot_orders_messages_and_maps_parts() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        assert!(store.is_loading());

        let applied = store.apply_snapshot(
            "ses_1",
            vec![
                assistant_message(
                    "msg_2",
                    20,
                    vec![
                        AssistantPart::Text { text: "Hi".into() },
                        AssistantPart::Reasoning {
                            text: "thinking".into(),
                        },
                        AssistantPart::Tool {
                            id: "t1".into(),
                            name: "bash".into(),
                            executed: true,
                            state: ToolState::Completed {
                                input: r#"{"cmd":"ls"}"#.into(),
                                content: vec![ToolContent::Text { text: "ok".into() }],
                            },
                        },
                    ],
                ),
                user_message("msg_1", "hello", 10),
                SessionMessage::Idle {
                    id: "msg_3".into(),
                    time: harness_protocol::MessageTime {
                        created: 30,
                        streamed: None,
                        completed: None,
                    },
                    outcome: Some("succeeded".into()),
                    error: None,
                },
            ],
        );

        assert!(applied);
        assert!(!store.is_loading());
        let ids: Vec<_> = store.entries().iter().map(Entry::id).collect();
        assert_eq!(ids, ["msg_1", "msg_2"]);

        let Entry::Assistant(assistant) = &store.entries()[1] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(assistant.agent.as_deref(), Some("build"));
        assert_eq!(assistant.model.as_deref(), Some("anthropic/sonnet"));
        assert_eq!(assistant.parts.len(), 3);
        let Content::Tool(tool) = &assistant.parts[2] else {
            panic!("expected a tool part");
        };
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.status, ToolStatus::Completed);
        assert_eq!(tool.input, r#"{"cmd":"ls"}"#);
        assert_eq!(tool.output.as_deref(), Some("ok"));
        assert_eq!(tool.error, None);
    }

    #[test]
    fn reasoning_streams_and_settles() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");

        store.apply_stream(StreamEvent::ReasoningStarted {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            ordinal: 0,
            created: 10,
        });
        store.apply_stream(StreamEvent::ReasoningDelta {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            ordinal: 0,
            created: 10,
            delta: "Let me ".into(),
        });
        store.apply_stream(StreamEvent::ReasoningDelta {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            ordinal: 0,
            created: 10,
            delta: "think.".into(),
        });

        let Entry::Assistant(assistant) = &store.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert!(matches!(
            &assistant.parts[0],
            Content::Reasoning { text, .. } if text == "Let me think."
        ));

        // The ended frame is authoritative and does not duplicate the part.
        store.apply_stream(StreamEvent::ReasoningEnded {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            ordinal: 0,
            created: 10,
            text: "Let me think carefully.".into(),
        });
        let Entry::Assistant(assistant) = &store.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(assistant.parts.len(), 1);
        assert!(matches!(
            &assistant.parts[0],
            Content::Reasoning { text, .. } if text == "Let me think carefully."
        ));
    }

    #[test]
    fn tool_lifecycle_updates_one_card_in_place() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");

        store.apply_stream(StreamEvent::ToolInputStarted {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            name: "read".into(),
        });
        store.apply_stream(StreamEvent::ToolInputDelta {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            delta: r#"{"path":"/tmp/a"#.into(),
        });

        let tool = tool_of(&store, "call_1");
        assert_eq!(tool.name, "read");
        assert_eq!(tool.status, ToolStatus::Streaming);
        assert_eq!(tool.input, r#"{"path":"/tmp/a"#);

        store.apply_stream(StreamEvent::ToolInputEnded {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            input: r#"{"path":"/tmp/a"}"#.into(),
        });
        store.apply_stream(StreamEvent::ToolCalled {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            input: r#"{"path":"/tmp/a"}"#.into(),
            executed: false,
        });
        assert_eq!(tool_of(&store, "call_1").status, ToolStatus::Running);

        store.apply_stream(StreamEvent::ToolSucceeded {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            content: vec![ToolContent::Text {
                text: "hello".into(),
            }],
        });
        let tool = tool_of(&store, "call_1");
        assert_eq!(tool.status, ToolStatus::Completed);
        assert_eq!(tool.output.as_deref(), Some("hello"));
        assert_eq!(tool.error, None);
        // Every event updated the same card.
        assert_eq!(assistant_of(&store).parts.len(), 1);
    }

    #[test]
    fn failed_tool_keeps_its_message() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");

        store.apply_stream(StreamEvent::ToolFailed {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            error: StructuredError {
                kind: "tool.execution".into(),
                message: "not found".into(),
                status: None,
            },
            content: vec![ToolContent::Text {
                text: "details".into(),
            }],
        });

        let tool = tool_of(&store, "call_1");
        assert_eq!(tool.status, ToolStatus::Error);
        assert_eq!(tool.error.as_deref(), Some("not found"));
        assert_eq!(tool.output.as_deref(), Some("details"));
    }

    #[test]
    fn tool_events_for_other_sessions_are_ignored() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        assert!(!store.apply_stream(StreamEvent::ToolInputStarted {
            session_id: "ses_2".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            name: "read".into(),
        }));
        assert!(store.entries().is_empty());
    }

    fn assistant_of(store: &TranscriptStore) -> &AssistantEntry {
        let Entry::Assistant(assistant) = &store.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assistant
    }

    fn tool_of<'a>(store: &'a TranscriptStore, id: &str) -> &'a ToolCall {
        assistant_of(store)
            .parts
            .iter()
            .find_map(|part| match part {
                Content::Tool(tool) if tool.id == id => Some(tool),
                _ => None,
            })
            .expect("expected a tool part")
    }

    #[test]
    fn intentional_interrupts_do_not_read_as_failures() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");

        let mut aborted = assistant_message(
            "msg_2",
            20,
            vec![AssistantPart::Text {
                text: "working".into(),
            }],
        );
        let mut failed = assistant_message("msg_3", 30, vec![]);
        if let SessionMessage::Assistant { error, .. } = &mut aborted {
            *error = Some(StructuredError {
                kind: "aborted".into(),
                message: "Step interrupted".into(),
                status: None,
            });
        }
        if let SessionMessage::Assistant { error, .. } = &mut failed {
            *error = Some(StructuredError {
                kind: "provider.quota".into(),
                message: "No credits".into(),
                status: Some(402),
            });
        }

        assert!(store.apply_snapshot("ses_1", vec![aborted, failed]));

        let Entry::Assistant(aborted) = &store.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(aborted.error, None);
        assert!(matches!(
            &aborted.parts[0],
            Content::Text { text, .. } if text == "working"
        ));

        let Entry::Assistant(failed) = &store.entries()[1] else {
            panic!("expected a second assistant entry");
        };
        assert_eq!(failed.error.as_deref(), Some("No credits"));
    }

    #[test]
    fn stale_snapshots_are_ignored() {
        let mut store = TranscriptStore::default();
        store.begin("ses_2");
        assert!(!store.apply_snapshot("ses_1", vec![user_message("msg_1", "old", 1)]));
        assert!(store.entries().is_empty());
    }

    #[test]
    fn streaming_deltas_accumulate_and_final_text_replaces() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");

        assert!(store.apply_stream(StreamEvent::ExecutionStarted {
            session_id: "ses_1".into()
        }));
        assert!(store.is_busy());

        store.apply_stream(StreamEvent::AssistantStarted {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_2".into(),
            started: 20,
            agent: Some("build".into()),
            model: None,
        });
        store.apply_stream(delta("msg_2", 0, 20, "Hel"));
        store.apply_stream(delta("msg_2", 0, 20, "lo"));
        store.apply_stream(StreamEvent::TextEnded {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_2".into(),
            ordinal: 0,
            created: 20,
            text: "Hello there".into(),
        });

        let Entry::Assistant(assistant) = &store.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(
            assistant.parts,
            vec![Content::Text {
                ordinal: 0,
                text: "Hello there".into(),
            }]
        );

        assert!(store.apply_stream(StreamEvent::ExecutionFinished {
            session_id: "ses_1".into(),
            outcome: ExecutionOutcome::Succeeded,
        }));
        assert!(!store.is_busy());
        assert_eq!(store.error(), None);
    }

    #[test]
    fn failed_turns_surface_their_message() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        store.apply_stream(StreamEvent::ExecutionStarted {
            session_id: "ses_1".into(),
        });
        store.apply_stream(StreamEvent::ExecutionFinished {
            session_id: "ses_1".into(),
            outcome: ExecutionOutcome::Failed {
                message: "No credits".into(),
            },
        });
        assert!(!store.is_busy());
        assert_eq!(store.error(), Some("No credits"));
        store.clear_error();
        assert_eq!(store.error(), None);
    }

    #[test]
    fn events_for_other_sessions_are_ignored() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        assert!(!store.apply_stream(StreamEvent::ExecutionStarted {
            session_id: "ses_2".into()
        }));
        assert!(!store.is_busy());
        assert!(store.entries().is_empty());
    }

    #[test]
    fn accept_prompt_upserts_the_user_message() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        assert!(store.accept_prompt("ses_1", "msg_1", "hello", 10));
        assert!(store.accept_prompt("ses_1", "msg_1", "hello again", 11));
        assert_eq!(store.entries().len(), 1);
        assert!(matches!(
            &store.entries()[0],
            Entry::User { text, created, .. } if text == "hello again" && *created == 11
        ));
        assert!(!store.accept_prompt("ses_2", "msg_9", "no", 1));
    }

    fn permission(id: &str, session: &str) -> PermissionRequest {
        PermissionRequest {
            id: id.into(),
            session_id: session.into(),
            action: "shell".into(),
            resources: vec![format!("echo {id}")],
            save: Vec::new(),
            message: None,
            source: None,
        }
    }

    #[test]
    fn permission_queue_is_session_scoped_and_deduped() {
        let mut queue = PermissionQueue::default();
        queue.begin("ses_1");
        assert!(queue.push(permission("per_1", "ses_1")));
        assert!(!queue.push(permission("per_1", "ses_1")));
        assert!(queue.push(permission("per_2", "ses_1")));
        assert!(!queue.push(permission("per_3", "ses_2")));
        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue.front().map(|request| request.id.as_str()),
            Some("per_1")
        );

        assert!(queue.remove("per_1"));
        assert!(!queue.remove("per_1"));
        assert_eq!(
            queue.front().map(|request| request.id.as_str()),
            Some("per_2")
        );

        // A snapshot replaces the queue; one for another session is stale.
        assert!(queue.replace("ses_1", vec![permission("per_4", "ses_1")]));
        assert_eq!(
            queue.front().map(|request| request.id.as_str()),
            Some("per_4")
        );
        assert!(!queue.replace("ses_2", Vec::new()));
        assert_eq!(queue.len(), 1);

        // Changing the selection clears the queue.
        queue.begin("ses_2");
        assert!(queue.is_empty());
        assert!(!queue.push(permission("per_5", "ses_1")));
        assert!(queue.push(permission("per_6", "ses_2")));
    }

    #[test]
    fn declined_tool_failure_keeps_its_message() {
        let mut store = TranscriptStore::default();
        store.begin("ses_1");
        store.apply_stream(StreamEvent::ToolFailed {
            session_id: "ses_1".into(),
            assistant_message_id: "msg_1".into(),
            tool_id: "call_1".into(),
            error: StructuredError {
                kind: "aborted".into(),
                message: "The user declined this tool call".into(),
                status: None,
            },
            content: Vec::new(),
        });

        let tool = tool_of(&store, "call_1");
        assert_eq!(tool.status, ToolStatus::Error);
        assert_eq!(
            tool.error.as_deref(),
            Some("The user declined this tool call")
        );
    }

    #[test]
    fn relative_time_covers_each_unit() {
        let now = 100_000_000_000;
        assert_eq!(relative_time(now, 0), "");
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now, now - 30_000), "just now");
        assert_eq!(relative_time(now, now - 60_000), "1m ago");
        assert_eq!(relative_time(now, now - 3_600_000), "1h ago");
        assert_eq!(relative_time(now, now - 86_400_000), "1d ago");
        assert_eq!(relative_time(now, now - 604_800_000), "1w ago");
        assert_eq!(relative_time(now, now - 2_592_000_000), "1mo ago");
        assert_eq!(relative_time(now, now - 31_536_000_000), "1y ago");
        assert_eq!(relative_time(now, now + 10_000), "just now");
    }
}
