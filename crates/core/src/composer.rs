//! Composer draft, triggers, pickers, and prompt payloads.
//!
//! The textarea owns the text; this module owns the meaning: which token is
//! being completed, which files/agents/skills are attached, and what Enter
//! sends. Mention offsets are recomputed from the text after every edit, never
//! patched in place, so an edit before a mention cannot leave a stale range.

use harness_protocol::{
    AgentAttachment, CommandRequest, FileSource, MentionRange, PromptFile, PromptRequest,
    SkillAttachment,
};

/// The sigil that opened a picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    File,
    Command,
    Skill,
    Snippet,
}

/// The token currently being typed at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// Byte offset of the sigil in the draft text.
    pub start: usize,
    pub query: String,
}

/// The text and cursor after a selection rewrote the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEdit {
    pub text: String,
    pub cursor: usize,
}

/// A file attached by an `@path` mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftFile {
    pub path: String,
    pub uri: String,
    /// UTF-16 range of `@path` in the text, as the prompt API expects.
    range: Option<(usize, usize)>,
}

/// An agent chip. Agents have no inline token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAgent {
    pub name: String,
}

/// A skill chip. Skills have no inline token once selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSkill {
    pub id: String,
    pub name: String,
}

/// A selected slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftCommand {
    pub name: String,
}

/// What Enter should send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftSend {
    Prompt(PromptRequest),
    Command(CommandRequest),
}

/// The composer's semantic state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptDraft {
    text: String,
    cursor: usize,
    files: Vec<DraftFile>,
    agents: Vec<DraftAgent>,
    skills: Vec<DraftSkill>,
    command: Option<DraftCommand>,
    trigger: Option<Trigger>,
}

impl PromptDraft {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn trigger(&self) -> Option<&Trigger> {
        self.trigger.as_ref()
    }

    pub fn files(&self) -> &[DraftFile] {
        &self.files
    }

    pub fn agents(&self) -> &[DraftAgent] {
        &self.agents
    }

    pub fn skills(&self) -> &[DraftSkill] {
        &self.skills
    }

    pub fn command(&self) -> Option<&DraftCommand> {
        self.command.as_ref()
    }

    /// Record a text/cursor change and recompute the active trigger.
    ///
    /// `collapsed` is false while text is selected: completing inside a
    /// selection would replace text the user did not point at.
    pub fn edit(&mut self, text: impl Into<String>, cursor: usize, collapsed: bool) {
        self.text = text.into();
        self.cursor = clamp_to_char_boundary(&self.text, cursor);
        self.trigger = if collapsed {
            detect_trigger(&self.text, self.cursor)
        } else {
            None
        };
        self.refresh_file_ranges();
        self.validate_command();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn clear_command(&mut self) {
        self.command = None;
    }

    /// Insert an `@path` mention for the active file trigger.
    pub fn select_file(&mut self, path: impl Into<String>, uri: String) -> Option<DraftEdit> {
        let path = path.into();
        let trigger = self.active(TriggerKind::File)?;
        let start = trigger.start;
        let mention = format!("@{path}");
        self.text.replace_range(start..self.cursor, &mention);
        self.cursor = start + mention.len();
        self.trigger = None;
        if let Some(file) = self.files.iter_mut().find(|file| file.path == path) {
            file.uri = uri;
        } else {
            self.files.push(DraftFile {
                path: path.to_string(),
                uri,
                range: None,
            });
        }
        self.refresh_file_ranges();
        self.edit_snapshot()
    }

    /// Attach an agent and remove the `@` token that opened the picker.
    pub fn select_agent(&mut self, name: impl Into<String>) -> Option<DraftEdit> {
        let name = name.into();
        let trigger = self.active(TriggerKind::File)?;
        self.remove_range(trigger.start, self.cursor);
        if !self.agents.iter().any(|agent| agent.name == name) {
            self.agents.push(DraftAgent { name: name.clone() });
        }
        self.edit_snapshot()
    }

    /// Attach a skill and remove the `$` token that opened the picker.
    pub fn select_skill(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Option<DraftEdit> {
        let id = id.into();
        let name = name.into();
        let trigger = self.active(TriggerKind::Skill)?;
        self.remove_range(trigger.start, self.cursor);
        if !self.skills.iter().any(|skill| skill.id == id) {
            self.skills.push(DraftSkill {
                id: id.clone(),
                name: name.clone(),
            });
        }
        self.edit_snapshot()
    }

    /// Replace the `/` token with a command and a trailing space.
    pub fn select_command(&mut self, name: impl Into<String>) -> Option<DraftEdit> {
        let name = name.into();
        let trigger = self.active(TriggerKind::Command)?;
        let start = trigger.start;
        let replacement = format!("/{name} ");
        self.text.replace_range(start..self.cursor, &replacement);
        self.cursor = start + replacement.len();
        self.command = Some(DraftCommand { name });
        self.trigger = None;
        self.refresh_file_ranges();
        self.edit_snapshot()
    }

    /// Expand the `#` token into plain text.
    pub fn select_snippet(&mut self, expansion: impl Into<String>) -> Option<DraftEdit> {
        let expansion = expansion.into();
        let trigger = self.active(TriggerKind::Snippet)?;
        let start = trigger.start;
        self.text.replace_range(start..self.cursor, &expansion);
        self.cursor = start + expansion.len();
        self.trigger = None;
        self.refresh_file_ranges();
        self.edit_snapshot()
    }

    /// Detach a file and remove its mention from the text.
    pub fn remove_file(&mut self, index: usize) -> Option<DraftEdit> {
        let (start, end) = self.files.get(index)?.range?;
        let start = byte_from_utf16(&self.text, start);
        let end = byte_from_utf16(&self.text, end);
        self.files.remove(index);
        self.remove_range(start, end);
        self.edit_snapshot()
    }

    pub fn remove_agent(&mut self, index: usize) -> bool {
        if index >= self.agents.len() {
            return false;
        }
        self.agents.remove(index);
        true
    }

    pub fn remove_skill(&mut self, index: usize) -> bool {
        if index >= self.skills.len() {
            return false;
        }
        self.skills.remove(index);
        true
    }

    /// Turn the active command back into a plain prompt, keeping its arguments.
    pub fn remove_command(&mut self) -> Option<DraftEdit> {
        let command = self.command.take()?;
        let prefix = format!("/{}", command.name);
        if let Some(rest) = self.text.strip_prefix(&prefix) {
            let rest = rest.trim_start().to_string();
            self.text = rest;
            self.cursor = self.text.len();
            self.trigger = None;
            self.refresh_file_ranges();
            return self.edit_snapshot();
        }
        None
    }

    pub fn is_sendable(&self) -> bool {
        self.command.is_some()
            || !self.text.trim().is_empty()
            || !self.files.is_empty()
            || !self.agents.is_empty()
            || !self.skills.is_empty()
    }

    /// The payload Enter should send, when there is one.
    pub fn to_send(&self) -> Option<DraftSend> {
        if !self.is_sendable() {
            return None;
        }
        if let Some(command) = &self.command {
            let prefix = format!("/{}", command.name);
            let text = self
                .text
                .strip_prefix(&prefix)
                .unwrap_or(&self.text)
                .trim_start()
                .to_string();
            return Some(DraftSend::Command(CommandRequest {
                name: command.name.clone(),
                text,
                ..CommandRequest::default()
            }));
        }
        Some(DraftSend::Prompt(PromptRequest {
            text: self.text.clone(),
            files: self
                .files
                .iter()
                .filter_map(|file| {
                    let (start, end) = file.range?;
                    Some(PromptFile {
                        uri: file.uri.clone(),
                        source: Some(FileSource::Uri {
                            uri: file.uri.clone(),
                        }),
                        name: file.path.rsplit('/').next().map(str::to_string),
                        mention: Some(MentionRange {
                            start,
                            end,
                            text: file.path.clone(),
                        }),
                    })
                })
                .collect(),
            agents: self
                .agents
                .iter()
                .map(|agent| AgentAttachment {
                    name: agent.name.clone(),
                    mention: None,
                })
                .collect(),
            skills: self
                .skills
                .iter()
                .map(|skill| SkillAttachment {
                    id: skill.id.clone(),
                    mention: None,
                })
                .collect(),
        }))
    }

    fn active(&self, kind: TriggerKind) -> Option<Trigger> {
        let trigger = self
            .trigger
            .clone()
            .filter(|trigger| trigger.kind == kind)?;
        Some(trigger)
    }

    fn remove_range(&mut self, start: usize, end: usize) {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len());
        if start >= end {
            return;
        }
        self.text.replace_range(start..end, "");
        self.cursor = if self.cursor >= end {
            self.cursor - (end - start)
        } else {
            self.cursor.min(start)
        };
        self.trigger = None;
        self.refresh_file_ranges();
        self.validate_command();
    }

    fn edit_snapshot(&mut self) -> Option<DraftEdit> {
        self.validate_command();
        Some(DraftEdit {
            text: self.text.clone(),
            cursor: self.cursor,
        })
    }

    fn refresh_file_ranges(&mut self) {
        self.files
            .retain(|file| find_mention(&self.text, &file.path).is_some());
        for file in &mut self.files {
            file.range = find_mention(&self.text, &file.path);
        }
    }

    fn validate_command(&mut self) {
        let Some(command) = &self.command else {
            return;
        };
        let prefix = format!("/{}", command.name);
        let valid = match self.text.strip_prefix(&prefix) {
            Some(rest) => rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n'),
            None => false,
        };
        if !valid {
            self.command = None;
        }
    }
}

/// Find `@path` as a standalone token and return its UTF-16 range.
fn find_mention(text: &str, path: &str) -> Option<(usize, usize)> {
    let needle = format!("@{path}");
    let mut from = 0;
    while let Some(index) = text[from..].find(&needle) {
        let start = from + index;
        let end = start + needle.len();
        let before_ok = start == 0
            || text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let after_ok =
            end == text.len() || text[end..].chars().next().is_some_and(char::is_whitespace);
        if before_ok && after_ok {
            return Some((utf16_offset(text, start), utf16_offset(text, end)));
        }
        from = end;
    }
    None
}

fn detect_trigger(text: &str, cursor: usize) -> Option<Trigger> {
    if cursor == 0 {
        return None;
    }
    let cursor = clamp_to_char_boundary(text, cursor);
    let mut start = cursor;
    for (index, ch) in text[..cursor].char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        start = index;
    }
    let sigil = text[start..cursor].chars().next()?;
    let kind = match sigil {
        '@' => TriggerKind::File,
        '/' => TriggerKind::Command,
        '$' => TriggerKind::Skill,
        '#' => TriggerKind::Snippet,
        _ => return None,
    };
    if kind == TriggerKind::Command && !text[..start].trim().is_empty() {
        return None;
    }
    Some(Trigger {
        kind,
        start,
        query: text[start + sigil.len_utf8()..cursor].to_string(),
    })
}

/// Convert a UTF-8 byte offset into the UTF-16 code-unit offset used by the
/// OpenCode prompt API.
pub fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    let byte_offset = clamp_to_char_boundary(text, byte_offset);
    text[..byte_offset].encode_utf16().count()
}

/// Convert a UTF-16 offset back into a byte offset, clamped to the text.
pub fn byte_from_utf16(text: &str, units_target: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= units_target {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Convert an absolute filesystem path into the `file://` URI the prompt API
/// requires, percent-encoding bytes outside the URI path set.
pub fn path_to_file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        let allowed =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':');
        if allowed {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// A built-in text expansion offered by the `#` picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snippet {
    pub name: &'static str,
    pub description: &'static str,
    pub expansion: &'static str,
}

/// Deliberately small: user-defined snippets need a storage design.
pub const SNIPPETS: &[Snippet] = &[
    Snippet {
        name: "review",
        description: "Ask for a review of the current changes",
        expansion: "Review the changes and list the risks.",
    },
    Snippet {
        name: "explain",
        description: "Explain the selected code",
        expansion: "Explain what this code does and why.",
    },
    Snippet {
        name: "tests",
        description: "Ask for tests",
        expansion: "Add tests for this behavior, including edge cases.",
    },
    Snippet {
        name: "fix",
        description: "Fix a failing test",
        expansion: "Find the cause of the failing test and fix it.",
    },
];

/// What a picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerKind {
    #[default]
    File,
    Command,
    Skill,
    Snippet,
    Agent,
    Model,
}

impl PickerKind {
    pub fn title(&self) -> &'static str {
        match self {
            Self::File => "Files and agents",
            Self::Command => "Commands",
            Self::Skill => "Skills",
            Self::Snippet => "Snippets",
            Self::Agent => "Agents",
            Self::Model => "Models",
        }
    }

    pub fn for_trigger(kind: TriggerKind) -> Self {
        match kind {
            TriggerKind::File => Self::File,
            TriggerKind::Command => Self::Command,
            TriggerKind::Skill => Self::Skill,
            TriggerKind::Snippet => Self::Snippet,
        }
    }
}

/// One picker row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerItem {
    File {
        path: String,
    },
    Command {
        name: String,
        description: String,
    },
    Skill {
        id: String,
        name: String,
        description: String,
    },
    Agent {
        id: String,
        name: String,
        description: String,
    },
    Snippet {
        name: String,
        description: String,
        expansion: String,
    },
    Model {
        model: harness_protocol::ModelRef,
        name: String,
        provider_id: String,
    },
}

impl PickerItem {
    pub fn label(&self) -> String {
        match self {
            Self::File { path } => path.clone(),
            Self::Command { name, .. } => format!("/{name}"),
            Self::Skill { name, .. } => format!("${name}"),
            Self::Agent { name, .. } => name.clone(),
            Self::Snippet { name, .. } => format!("#{name}"),
            Self::Model { name, .. } => name.clone(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::File { .. } | Self::Model { .. } => String::new(),
            Self::Command { description, .. }
            | Self::Skill { description, .. }
            | Self::Agent { description, .. }
            | Self::Snippet { description, .. } => description.clone(),
        }
    }

    fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }
}

/// What the picker needs the view to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerEffect {
    None,
    FetchFiles { request_id: u64, query: String },
}

/// The picker's pure state: query, rows, highlight, and the in-flight request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PickerState {
    kind: PickerKind,
    query: String,
    items: Vec<PickerItem>,
    filtered: Vec<usize>,
    highlighted: usize,
    request_id: u64,
    loading: bool,
    error: Option<String>,
}

impl PickerState {
    pub fn open(
        kind: PickerKind,
        query: impl Into<String>,
        items: Vec<PickerItem>,
    ) -> (Self, PickerEffect) {
        let mut state = Self {
            kind,
            query: query.into(),
            items,
            filtered: Vec::new(),
            highlighted: 0,
            request_id: 0,
            loading: false,
            error: None,
        };
        state.recompute();
        let effect = state.fetch_if_needed();
        (state, effect)
    }

    pub fn kind(&self) -> PickerKind {
        self.kind
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn visible_len(&self) -> usize {
        self.filtered.len()
    }

    pub fn visible(&self) -> impl Iterator<Item = &PickerItem> {
        self.filtered
            .iter()
            .filter_map(|index| self.items.get(*index))
    }

    pub fn selected(&self) -> Option<&PickerItem> {
        self.filtered
            .get(self.highlighted)
            .and_then(|index| self.items.get(*index))
    }

    pub fn move_highlight(&mut self, delta: i32) {
        let len = self.filtered.len();
        if len == 0 {
            self.highlighted = 0;
            return;
        }
        let next = self.highlighted as i32 + delta;
        self.highlighted = next.rem_euclid(len as i32) as usize;
    }

    pub fn update_query(&mut self, query: impl Into<String>) -> PickerEffect {
        self.query = query.into();
        self.error = None;
        self.recompute();
        self.fetch_if_needed()
    }

    /// Merge a file-search result, dropping it when a newer request is in
    /// flight. Non-file rows (agents) stay.
    pub fn apply_file_results(
        &mut self,
        request_id: u64,
        items: impl IntoIterator<Item = PickerItem>,
    ) -> bool {
        if request_id != self.request_id {
            return false;
        }
        let mut merged: Vec<PickerItem> = items.into_iter().filter(PickerItem::is_file).collect();
        merged.extend(self.items.iter().filter(|item| !item.is_file()).cloned());
        self.items = merged;
        self.loading = false;
        self.recompute();
        true
    }

    pub fn add_items(&mut self, items: impl IntoIterator<Item = PickerItem>) {
        self.items.extend(items);
        self.recompute();
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.loading = false;
    }

    fn fetch_if_needed(&mut self) -> PickerEffect {
        if self.kind != PickerKind::File {
            return PickerEffect::None;
        }
        self.request_id += 1;
        self.loading = true;
        PickerEffect::FetchFiles {
            request_id: self.request_id,
            query: self.query.clone(),
        }
    }

    fn recompute(&mut self) {
        let needle = self.query.trim().to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.label().to_lowercase().contains(&needle)
                    || item.detail().to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();
        if self.highlighted >= self.filtered.len() {
            self.highlighted = self.filtered.len().saturating_sub(1);
        }
    }
}
