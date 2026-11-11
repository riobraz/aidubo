//! Conversation mirror and streaming reducers.

use harness_protocol::{
    AssistantPart, ExecutionOutcome, ModelRef, SessionMessage, StreamEvent, StructuredError,
    ToolContent, ToolState,
};

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
            StreamEvent::PermissionAsked { .. }
            | StreamEvent::PermissionReplied { .. }
            | StreamEvent::FilesystemChanged
            | StreamEvent::UsageUpdated { .. }
            | StreamEvent::CompactionStarted { .. }
            | StreamEvent::CompactionEnded { .. }
            | StreamEvent::CompactionFailed { .. } => false,
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
