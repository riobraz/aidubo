//! Notification policy: dedupe, throttle, focus and preview rules.
//!
//! Delivery is the app's job; this module only decides whether an event
//! becomes a notification and what safe text it carries.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use harness_protocol::{ExecutionOutcome, StreamEvent};
use serde::{Deserialize, Serialize};

/// At most one notification per session in this window. Events inside the
/// window replace the pending one instead of queueing.
pub const NOTIFY_THROTTLE: Duration = Duration::from_secs(2);

/// How many event ids are remembered for reconnect replay suppression.
pub const NOTIFY_SEEN_LIMIT: usize = 200;

const PREVIEW_LIMIT: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    TurnDone,
    TurnFailed,
    PermissionAsked,
    QuestionAsked,
}

impl NotifyKind {
    pub const ALL: [Self; 4] = [
        Self::TurnDone,
        Self::TurnFailed,
        Self::PermissionAsked,
        Self::QuestionAsked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TurnDone => "turn_done",
            Self::TurnFailed => "turn_failed",
            Self::PermissionAsked => "permission_asked",
            Self::QuestionAsked => "question_asked",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TurnDone => "Turn finished",
            Self::TurnFailed => "Turn failed",
            Self::PermissionAsked => "Needs permission",
            Self::QuestionAsked => "Has a question",
        }
    }

    fn default_body(self) -> &'static str {
        self.label()
    }
}

/// One notification ready for a backend. It never contains message content
/// unless the user opted into previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub id: String,
    pub kind: NotifyKind,
    pub session_title: String,
    pub body: String,
}

/// Per-kind switches plus the preview opt-in. Stored app-locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    pub enabled: bool,
    pub turn_done: bool,
    pub turn_failed: bool,
    pub permission_asked: bool,
    pub question_asked: bool,
    pub include_preview: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            turn_done: true,
            turn_failed: true,
            permission_asked: true,
            question_asked: true,
            include_preview: false,
        }
    }
}

impl NotifySettings {
    pub fn allows(&self, kind: NotifyKind) -> bool {
        self.enabled && self.is_on(kind)
    }

    pub fn is_on(&self, kind: NotifyKind) -> bool {
        match kind {
            NotifyKind::TurnDone => self.turn_done,
            NotifyKind::TurnFailed => self.turn_failed,
            NotifyKind::PermissionAsked => self.permission_asked,
            NotifyKind::QuestionAsked => self.question_asked,
        }
    }

    pub fn set(&mut self, kind: NotifyKind, on: bool) {
        match kind {
            NotifyKind::TurnDone => self.turn_done = on,
            NotifyKind::TurnFailed => self.turn_failed = on,
            NotifyKind::PermissionAsked => self.permission_asked = on,
            NotifyKind::QuestionAsked => self.question_asked = on,
        }
    }
}

/// Who and where the event belongs to.
#[derive(Debug, Clone, Copy)]
pub struct NotifyContext<'a> {
    pub session_id: &'a str,
    pub session_title: &'a str,
    /// The window has OS focus.
    pub focused: bool,
    /// The event's session is the one open in the window.
    pub active_session: bool,
}

/// The parts of an event the policy needs. `event_key` must be stable across
/// a reconnect replay.
#[derive(Debug, Clone, Copy)]
pub struct NotifyInput<'a> {
    pub kind: NotifyKind,
    pub event_key: &'a str,
    pub preview: Option<&'a str>,
    pub context: NotifyContext<'a>,
}

#[derive(Debug, Default)]
pub struct NotifyPolicy {
    settings: NotifySettings,
    seen_order: VecDeque<String>,
    seen: HashSet<String>,
    last_sent: BTreeMap<String, Instant>,
    pending: BTreeMap<String, Notification>,
}

impl NotifyPolicy {
    pub fn new(settings: NotifySettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub fn settings(&self) -> &NotifySettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: NotifySettings) {
        if !settings.enabled {
            self.pending.clear();
        }
        self.settings = settings;
    }

    /// Map a live stream event to a notification decision.
    pub fn on_stream_event(
        &mut self,
        event: &StreamEvent,
        context: NotifyContext<'_>,
        now: Instant,
    ) -> Option<Notification> {
        let (kind, event_key, preview) = match event {
            StreamEvent::ExecutionFinished {
                event_id, outcome, ..
            } => match outcome {
                ExecutionOutcome::Succeeded => (NotifyKind::TurnDone, event_id.as_str(), None),
                ExecutionOutcome::Failed { message } => (
                    NotifyKind::TurnFailed,
                    event_id.as_str(),
                    Some(message.as_str()),
                ),
                ExecutionOutcome::Interrupted => return None,
            },
            StreamEvent::PermissionAsked { request } => (
                NotifyKind::PermissionAsked,
                request.id.as_str(),
                request.message.as_deref(),
            ),
            StreamEvent::FormCreated { form } => (
                NotifyKind::QuestionAsked,
                form.id.as_str(),
                Some(form.title.as_str()).filter(|title| !title.is_empty()),
            ),
            _ => return None,
        };
        self.on_event(
            NotifyInput {
                kind,
                event_key,
                preview,
                context,
            },
            now,
        )
    }

    pub fn on_event(&mut self, input: NotifyInput<'_>, now: Instant) -> Option<Notification> {
        let id = format!(
            "{}:{}:{}",
            input.context.session_id,
            input.kind.as_str(),
            input.event_key
        );
        if !self.remember(&id) {
            return None;
        }

        let context = input.context;
        if !self.settings.allows(input.kind) {
            return None;
        }
        if context.focused && context.active_session {
            self.pending.remove(context.session_id);
            return None;
        }

        let notification = Notification {
            id,
            kind: input.kind,
            session_title: if context.session_title.is_empty() {
                "Session".to_string()
            } else {
                context.session_title.to_string()
            },
            body: self.body(input),
        };

        let session = context.session_id;
        let throttled = self.pending.contains_key(session)
            || self
                .last_sent
                .get(session)
                .is_some_and(|last| now.duration_since(*last) < NOTIFY_THROTTLE);
        if throttled {
            self.pending.insert(session.to_string(), notification);
            None
        } else {
            self.last_sent.insert(session.to_string(), now);
            Some(notification)
        }
    }

    /// Deliver the pending notification of every session whose window elapsed.
    pub fn flush(&mut self, now: Instant) -> Vec<Notification> {
        if !self.settings.enabled {
            self.pending.clear();
            return Vec::new();
        }
        let due: Vec<String> = self
            .pending
            .keys()
            .filter(|session| {
                self.last_sent
                    .get(*session)
                    .is_none_or(|last| now.duration_since(*last) >= NOTIFY_THROTTLE)
            })
            .cloned()
            .collect();
        let mut delivered = Vec::with_capacity(due.len());
        for session in due {
            if let Some(notification) = self.pending.remove(&session) {
                self.last_sent.insert(session, now);
                delivered.push(notification);
            }
        }
        delivered
    }

    fn body(&self, input: NotifyInput<'_>) -> String {
        match input.preview {
            Some(preview) if self.settings.include_preview => {
                let preview = preview.trim();
                if preview.is_empty() {
                    input.kind.default_body().to_string()
                } else {
                    truncate(preview)
                }
            }
            _ => input.kind.default_body().to_string(),
        }
    }

    /// Remember an id, returning false when it was already seen.
    fn remember(&mut self, id: &str) -> bool {
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.seen_order.push_back(id.to_string());
        while self.seen_order.len() > NOTIFY_SEEN_LIMIT {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= PREVIEW_LIMIT {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(PREVIEW_LIMIT - 1).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::{FormInfo, PermissionRequest};

    fn context<'a>(session: &'a str, focused: bool, active: bool) -> NotifyContext<'a> {
        NotifyContext {
            session_id: session,
            session_title: "Fix the build",
            focused,
            active_session: active,
        }
    }

    fn done(session: &str, event_id: &str) -> StreamEvent {
        StreamEvent::ExecutionFinished {
            session_id: session.to_string(),
            event_id: event_id.to_string(),
            outcome: ExecutionOutcome::Succeeded,
        }
    }

    fn permission(id: &str, message: Option<&str>) -> PermissionRequest {
        PermissionRequest {
            id: id.to_string(),
            session_id: "s1".to_string(),
            action: "bash".to_string(),
            resources: Vec::new(),
            save: Vec::new(),
            message: message.map(str::to_string),
            source: None,
        }
    }

    #[test]
    fn notify_dedupes_replayed_event() {
        let mut policy = NotifyPolicy::default();
        let now = Instant::now();
        let event = done("s1", "evt_1");

        let first = policy.on_stream_event(&event, context("s1", false, false), now);
        assert_eq!(first.expect("first").kind, NotifyKind::TurnDone);

        // A reconnect replays the frame with the same id.
        let replay = policy.on_stream_event(&event, context("s1", false, false), now);
        assert!(replay.is_none());
    }

    #[test]
    fn notify_throttles_per_session() {
        let mut policy = NotifyPolicy::default();
        let start = Instant::now();
        assert!(
            policy
                .on_stream_event(&done("s1", "evt_1"), context("s1", false, false), start)
                .is_some()
        );

        let later = start + Duration::from_millis(100);
        assert!(
            policy
                .on_stream_event(&done("s1", "evt_2"), context("s1", false, false), later)
                .is_none()
        );

        assert!(policy.flush(start + Duration::from_secs(1)).is_empty());
        let delivered = policy.flush(start + NOTIFY_THROTTLE);
        assert_eq!(delivered.len(), 1);
        // The later event replaced the pending one.
        assert_eq!(delivered[0].id, "s1:turn_done:evt_2");
    }

    #[test]
    fn notify_preview_off_by_default() {
        let mut policy = NotifyPolicy::default();
        let now = Instant::now();
        let event = StreamEvent::PermissionAsked {
            request: permission("perm_1", Some("run the secret migration")),
        };
        let notification = policy
            .on_stream_event(&event, context("s1", false, false), now)
            .expect("notification");
        assert_eq!(notification.body, "Needs permission");
        assert!(!notification.body.contains("secret"));

        policy.set_settings(NotifySettings {
            include_preview: true,
            ..NotifySettings::default()
        });
        let long = "x".repeat(300);
        let event = StreamEvent::PermissionAsked {
            request: permission("perm_2", Some(&long)),
        };
        let notification = policy
            .on_stream_event(&event, context("s1", false, false), now + NOTIFY_THROTTLE)
            .expect("preview notification");
        assert!(notification.body.chars().count() <= PREVIEW_LIMIT);
        assert!(notification.body.ends_with('…'));
    }

    #[test]
    fn notify_form_created_asks_question() {
        let mut policy = NotifyPolicy::default();
        let event = StreamEvent::FormCreated {
            form: FormInfo {
                id: "form_1".into(),
                session_id: "s1".into(),
                title: "Pick one".into(),
                fields: Vec::new(),
            },
        };
        let notification = policy
            .on_stream_event(&event, context("s1", false, false), Instant::now())
            .expect("question notification");
        assert_eq!(notification.kind, NotifyKind::QuestionAsked);
        assert_eq!(notification.body, "Has a question");
    }

    #[test]
    fn notify_suppressed_when_focused() {
        let mut policy = NotifyPolicy::default();
        let now = Instant::now();
        assert!(
            policy
                .on_stream_event(&done("s1", "evt_1"), context("s1", true, true), now)
                .is_none()
        );
        // Focused on another session still notifies.
        assert!(
            policy
                .on_stream_event(&done("s2", "evt_2"), context("s2", true, false), now)
                .is_some()
        );
        // A disabled switch wins over everything.
        policy.set_settings(NotifySettings {
            enabled: false,
            ..NotifySettings::default()
        });
        assert!(
            policy
                .on_stream_event(
                    &done("s3", "evt_3"),
                    context("s3", false, false),
                    now + NOTIFY_THROTTLE
                )
                .is_none()
        );
    }
}
