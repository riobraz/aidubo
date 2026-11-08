//! The M1 workspace: project-scoped sessions.

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, Theme as KitTheme, ThemeMode, TitleBar,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    text::TextView,
};
use gpui_kit::*;
use harness_core::{
    Content, EngineStatus, Entry, PermissionQueue, SessionStore, ToolCall, ToolStatus,
    TranscriptStore, now_ms, relative_time,
};
use harness_engine::{Engine, EngineEvent, PermissionReply, StreamEvent};
use harness_protocol::{PermissionRequest, Session};
use harness_ui::{Theme, status_dot};

gpui_kit::actions!(
    harness_views,
    [NewSession, ToggleTheme, FocusComposer, CancelOverlays]
);

/// Root view. Owns the engine handle so the child is stopped when the view is
/// dropped at app exit.
pub struct WorkspaceView {
    status: EngineStatus,
    sessions: SessionStore,
    project_dir: PathBuf,
    /// Last command error, shown until dismissed or superseded.
    error: Option<String>,
    /// Open title editor, if any.
    renaming: Option<Entity<InputState>>,
    /// Session awaiting delete confirmation.
    pending_delete: Option<String>,
    transcript: TranscriptStore,
    permissions: PermissionQueue,
    /// The request whose reply is in flight; its buttons stay disabled.
    permission_in_flight: Option<String>,
    /// Whether the live event stream is currently open.
    stream_connected: bool,
    composer: Entity<TextareaState>,
    /// Sends the composer text when Enter is pressed.
    _composer_subscription: Subscription,
    engine: Engine,
    _events_task: Task<()>,
}

impl WorkspaceView {
    pub fn new(
        window: &mut Window,
        engine: Engine,
        project_dir: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = engine.events();
        let events_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let updated = this.update(cx, |this, cx| {
                    this.apply(event);
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message the agent…")
                .submit_on_enter(true)
        });
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.send_prompt(window, cx);
                }
            },
        );

        Self {
            status: EngineStatus::default(),
            sessions: SessionStore::default(),
            project_dir,
            error: None,
            renaming: None,
            pending_delete: None,
            transcript: TranscriptStore::default(),
            permissions: PermissionQueue::default(),
            permission_in_flight: None,
            stream_connected: false,
            composer,
            _composer_subscription: composer_subscription,
            engine,
            _events_task: events_task,
        }
    }

    fn apply(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Starting { port } => {
                self.status = EngineStatus::Starting { port: Some(port) };
            }
            EngineEvent::Ready { version, port } => {
                self.status = EngineStatus::Ready { version, port };
            }
            EngineEvent::Failed { message } => {
                self.status = EngineStatus::Failed { message };
            }
            EngineEvent::Stopped => {
                self.status = EngineStatus::Stopped;
            }
            EngineEvent::Log(_) => {}
            EngineEvent::SessionsLoaded { sessions } => {
                let previous = self.sessions.selected_id().map(str::to_owned);
                self.sessions.replace(sessions);
                let current = self.sessions.selected_id().map(str::to_owned);
                self.error = None;
                if previous != current {
                    // First load (or the selected session disappeared): open
                    // the newly selected session's timeline.
                    self.follow_selection();
                }
            }
            EngineEvent::SessionCreated { session } => {
                let id = session.id.clone();
                self.sessions.upsert(session);
                self.sessions.select(&id);
                self.renaming = None;
                self.pending_delete = None;
                self.error = None;
                self.transcript.begin(&id);
                self.engine.load_transcript(id.clone());
                self.permissions.begin(&id);
                self.engine.load_permissions(id);
            }
            EngineEvent::SessionRenamed { id, title } => {
                self.sessions.rename(&id, title);
                self.error = None;
            }
            EngineEvent::SessionDeleted { id } => {
                self.sessions.remove(&id);
                if self.pending_delete.as_deref() == Some(id.as_str()) {
                    self.pending_delete = None;
                }
                self.renaming = None;
                self.error = None;
                self.follow_selection();
            }
            EngineEvent::TranscriptLoaded {
                session_id,
                messages,
            } => {
                if self.transcript.apply_snapshot(&session_id, messages) {
                    self.error = None;
                }
            }
            EngineEvent::PromptAccepted {
                session_id,
                message,
            } => {
                self.transcript.accept_prompt(
                    &session_id,
                    &message.id,
                    &message.payload.text,
                    message.time.created,
                );
            }
            EngineEvent::Stream(event) => match event {
                StreamEvent::PermissionAsked { request } => {
                    self.permissions.push(request);
                }
                StreamEvent::PermissionReplied { request_id, .. } => {
                    self.permissions.remove(&request_id);
                    if self.permission_in_flight.as_deref() == Some(request_id.as_str()) {
                        self.permission_in_flight = None;
                    }
                }
                event => {
                    let finished = matches!(event, StreamEvent::ExecutionFinished { .. });
                    if self.transcript.apply_stream(event) && finished {
                        // The final message (finish reason, error, full text)
                        // is only authoritative on the server.
                        self.follow_selection();
                    }
                }
            },
            EngineEvent::PermissionsLoaded {
                session_id,
                requests,
            } => {
                self.permissions.replace(&session_id, requests);
                if let Some(id) = self.permission_in_flight.clone()
                    && !self.permissions.contains(&id)
                {
                    self.permission_in_flight = None;
                }
            }
            EngineEvent::PermissionReplied { request_id, .. } => {
                self.permissions.remove(&request_id);
                if self.permission_in_flight.as_deref() == Some(request_id.as_str()) {
                    self.permission_in_flight = None;
                }
                self.error = None;
            }
            EngineEvent::StreamConnected => {
                self.stream_connected = true;
                // Events may have been missed while disconnected.
                self.follow_selection();
            }
            EngineEvent::StreamDisconnected => {
                self.stream_connected = false;
            }
            EngineEvent::Interrupted {
                session_id,
                interrupted: true,
            } => {
                // A turn waiting on a permission may cancel its request; the
                // server's pending list is authoritative.
                self.engine.load_permissions(session_id);
            }
            EngineEvent::Interrupted { .. } => {}
            EngineEvent::CommandFailed { message } => {
                self.permission_in_flight = None;
                self.error = Some(message);
            }
        }
    }

    /// Point the transcript at the selected session and reload it.
    fn follow_selection(&mut self) {
        match self.sessions.selected_id().map(str::to_owned) {
            Some(id) => {
                self.transcript.begin(&id);
                self.engine.load_transcript(id.clone());
                self.permissions.begin(&id);
                self.engine.load_permissions(id);
            }
            None => {
                self.transcript = TranscriptStore::default();
                self.permissions = PermissionQueue::default();
                self.permission_in_flight = None;
            }
        }
    }

    fn create_session(&mut self, cx: &mut Context<Self>) {
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        if !self.engine.create_session() {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        if self.sessions.selected_id() == Some(id.as_str()) {
            return;
        }
        self.sessions.select(&id);
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        self.follow_selection();
        cx.notify();
    }

    fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.selected() else {
            return;
        };
        let current = session.title.clone().unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.renaming = Some(input);
        self.pending_delete = None;
        self.error = None;
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            self.renaming = None;
            cx.notify();
            return;
        };
        let Some(input) = self.renaming.take() else {
            return;
        };

        let value = input.read(cx).value().to_string();
        let title = value.trim().to_string();
        if title.is_empty() {
            self.error = Some("Enter a title before saving.".into());
            self.renaming = Some(input);
            cx.notify();
            return;
        }

        self.error = None;
        if !self.engine.rename_session(id, title) {
            self.error = Some("The engine is not running.".into());
            self.renaming = Some(input);
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming = None;
        self.error = None;
        cx.notify();
    }

    fn request_delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            return;
        };
        self.pending_delete = Some(id);
        self.renaming = None;
        self.error = None;
        cx.notify();
    }

    fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.pending_delete = None;
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pending_delete.take() else {
            return;
        };
        self.renaming = None;
        self.error = None;
        if !self.engine.delete_session(&id) {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
    }

    /// Switch between the light and dark themes.
    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().mode.is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        KitTheme::change(mode, Some(window), cx);
        cx.notify();
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Close transient overlays (title editor, delete confirmation, errors).
    fn cancel_overlays(&mut self, cx: &mut Context<Self>) {
        let had_overlay =
            self.renaming.is_some() || self.pending_delete.is_some() || self.error.is_some();
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        if had_overlay {
            cx.notify();
        }
    }

    fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            self.error = Some("Select a session before sending a message.".into());
            cx.notify();
            return;
        };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }

        self.error = None;
        if self.engine.send_prompt(id, text) {
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        } else {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn stop_turn(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            return;
        };
        if !self.engine.interrupt(id) {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    /// Answer the oldest pending permission request.
    fn answer_permission(&mut self, reply: PermissionReply, cx: &mut Context<Self>) {
        let Some(request) = self.permissions.front() else {
            return;
        };
        let session_id = request.session_id.clone();
        let request_id = request.id.clone();

        self.error = None;
        if self
            .engine
            .reply_permission(session_id, request_id.clone(), reply)
        {
            self.permission_in_flight = Some(request_id);
        } else {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn status_badge(&self, theme: Theme) -> (&'static str, Hsla) {
        match &self.status {
            EngineStatus::Starting { .. } => (self.status.label(), theme.warning),
            EngineStatus::Ready { .. } => (self.status.label(), theme.success),
            EngineStatus::Failed { .. } => (self.status.label(), theme.danger),
            EngineStatus::Stopped => (self.status.label(), theme.muted),
        }
    }

    fn detail(&self) -> SharedString {
        match &self.status {
            EngineStatus::Starting { port: Some(port) } => {
                format!("Launching the OpenCode sidecar on 127.0.0.1:{port}…").into()
            }
            EngineStatus::Starting { port: None } => {
                "Locating and launching the OpenCode sidecar…".into()
            }
            EngineStatus::Ready { version, port } => {
                format!("OpenCode {version} · 127.0.0.1:{port}").into()
            }
            EngineStatus::Failed { message } => message.clone().into(),
            EngineStatus::Stopped => "The engine has stopped.".into(),
        }
    }

    /// Project name for the title bar, falling back to the full path when the
    /// directory has no final component.
    fn project_name(&self) -> SharedString {
        self.project_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .map(SharedString::from)
            .unwrap_or_else(|| SharedString::from(self.project_dir.display().to_string()))
    }

    /// Window title content: the app and project on the left, engine status on
    /// the right. The bar itself owns dragging and double-click handling.
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let (label, color) = self.status_badge(theme);

        div()
            .debug_selector(|| "workspace-title-bar".into())
            .flex_shrink_0()
            .child(
                TitleBar::new()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().child("Rust Harness"))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted)
                                    .child(self.project_name()),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pr_3()
                            .child(status_dot(color))
                            .child(div().text_xs().text_color(theme.muted).child(label)),
                    ),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let ready = self.status.is_ready();
        let (label, color) = self.status_badge(theme);
        let detail = self.detail();
        let project = SharedString::from(self.project_dir.display().to_string());
        let count = self.sessions.len();
        let theme_label = if cx.theme().mode.is_dark() {
            "Light"
        } else {
            "Dark"
        };

        let mut list = div()
            .id("session-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2();

        if self.sessions.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(theme.muted)
                    .child(if ready {
                        "No sessions in this project yet."
                    } else {
                        "Waiting for the engine…"
                    }),
            );
        } else {
            for session in self.sessions.sessions() {
                list = list.child(self.session_row(session, theme, cx));
            }
        }

        div()
            .w(px(288.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().child("Sessions"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted)
                                    .child(format!("{count}")),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("theme-toggle")
                            .small()
                            .ghost()
                            .label(theme_label)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_theme(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("sidebar-new-session")
                            .small()
                            .primary()
                            .label("New")
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.create_session(cx))),
                    ),
            )
            .child(div().h(px(1.)).bg(theme.border))
            .child(list)
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(project),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(status_dot(color))
                            .child(div().text_xs().child(label)),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(detail)),
            )
    }

    fn session_row(
        &self,
        session: &Session,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.sessions.selected_id() == Some(session.id.as_str());
        let title = session
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "Untitled session".into());
        let updated = relative_time(now_ms(), session.time.updated);
        let id = session.id.clone();

        let row = div()
            .id(SharedString::from(format!("session-{}", session.id)))
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(theme.border))
            .on_click(cx.listener(move |this, _, _, cx| this.select_session(id.clone(), cx)));

        let row = if selected { row.bg(theme.border) } else { row };

        let row = row.child(div().min_w_0().truncate().text_sm().child(title));
        if updated.is_empty() {
            row
        } else {
            row.child(div().text_xs().text_color(theme.muted).child(updated))
        }
    }

    fn render_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let mut main = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(theme.bg);

        if self.error.is_some() {
            main = main.child(self.error_banner(theme, cx));
        }

        if self.status.is_ready() && !self.stream_connected {
            main = main.child(self.reconnect_banner(theme));
        }

        if let Some(session) = self.sessions.selected() {
            main = main.child(self.selected_pane(session, theme, cx));
        } else {
            main = main.child(self.empty_pane(theme, cx));
        }

        main
    }

    fn error_banner(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let message = self.error.clone().unwrap_or_default();
        div()
            .m_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.danger)
            .bg(theme.surface)
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(message),
            )
            .child(
                Button::new("dismiss-error")
                    .small()
                    .ghost()
                    .label("Dismiss")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_error(cx))),
            )
    }

    /// Shown while the sidecar is up but live updates are paused.
    fn reconnect_banner(&self, theme: Theme) -> impl IntoElement {
        div()
            .m_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.warning)
            .bg(theme.surface)
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.warning)
                    .child("Reconnecting to the agent…"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("Live updates resume automatically."),
            )
    }

    fn empty_pane(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.status.is_ready();
        let (title, hint) = match &self.status {
            EngineStatus::Failed { message } => ("The engine failed to start", message.clone()),
            EngineStatus::Starting { .. } => (
                "Starting OpenCode…",
                "The session list will appear when the engine is ready.".to_string(),
            ),
            EngineStatus::Stopped => (
                "The engine has stopped",
                "Restart the app to reconnect.".to_string(),
            ),
            EngineStatus::Ready { .. } => (
                "No sessions in this project",
                "Create a session to start a conversation.".to_string(),
            ),
        };

        let pane = div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(div().text_lg().child(title))
            .child(
                div()
                    .max_w(px(420.))
                    .text_sm()
                    .text_color(theme.muted)
                    .child(hint),
            );

        if ready {
            pane.child(
                Button::new("empty-new-session")
                    .small()
                    .primary()
                    .label("New session")
                    .on_click(cx.listener(|this, _, _, cx| this.create_session(cx))),
            )
        } else {
            pane
        }
    }

    fn selected_pane(
        &self,
        session: &Session,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = session
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "Untitled session".into());
        let subtitle = session_subtitle(session);

        let header = div()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().min_w_0().truncate().text_xl().child(title))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(subtitle),
                    ),
            )
            .child(
                Button::new("rename-session")
                    .small()
                    .ghost()
                    .label("Rename")
                    .on_click(cx.listener(|this, _, window, cx| this.begin_rename(window, cx))),
            )
            .child(
                Button::new("delete-session")
                    .small()
                    .danger()
                    .label("Delete")
                    .on_click(cx.listener(|this, _, _, cx| this.request_delete(cx))),
            );

        let mut pane = div().flex_1().min_h_0().flex().flex_col().child(header);

        if self.pending_delete.as_deref() == Some(session.id.as_str()) {
            pane = pane.child(self.delete_confirm(session, theme, cx));
        }

        if let Some(input) = &self.renaming {
            pane = pane.child(self.rename_panel(input, theme, cx));
        }

        let mut pane = pane.child(self.transcript_pane(theme));

        if let Some(request) = self.permissions.front() {
            pane = pane.child(self.permission_sheet(request, theme, cx));
        }

        pane.child(self.composer_bar(theme, cx))
    }

    /// The pending permission prompt, shown above the composer while a tool
    /// call is waiting on the user. Replies are explicit; nothing is approved
    /// on the user's behalf.
    fn permission_sheet(
        &self,
        request: &PermissionRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let in_flight = self
            .permission_in_flight
            .as_deref()
            .is_some_and(|id| id == request.id);
        let ready = self.status.is_ready() && !in_flight;

        let action = if request.action.trim().is_empty() {
            "use a tool".to_string()
        } else {
            request.action.clone()
        };
        let mut details = String::new();
        for resource in &request.resources {
            details.push_str(resource);
            details.push('\n');
        }
        let details = details.trim().to_string();

        let mut card = div()
            .mx_4()
            .mb_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.warning)
            .bg(theme.surface)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(format!("Allow the agent to {action}?")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("Permission required"),
                    ),
            );

        if let Some(message) = request.message.as_deref().filter(|text| !text.is_empty()) {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(message.to_owned()),
            );
        }

        if !details.is_empty() {
            card = card.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(theme.bg)
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.text)
                    .child(truncate_text(&details, 900)),
            );
        }

        if let Some(save) = (!request.save.is_empty()).then(|| request.save.join(", ")) {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(format!("Always allow covers: {save}")),
            );
        }

        card.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("permission-once")
                        .small()
                        .primary()
                        .label("Allow once")
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.answer_permission(PermissionReply::Once, cx)
                        })),
                )
                .child(
                    Button::new("permission-always")
                        .small()
                        .ghost()
                        .label("Always allow")
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.answer_permission(PermissionReply::Always, cx)
                        })),
                )
                .child(
                    Button::new("permission-reject")
                        .small()
                        .danger()
                        .label("Deny")
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.answer_permission(PermissionReply::Reject, cx)
                        })),
                ),
        )
    }

    fn transcript_pane(&self, theme: Theme) -> impl IntoElement {
        let mut list = div()
            .id("transcript-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .px_6()
            .py_5();

        if self.transcript.is_loading() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(theme.muted)
                    .child("Loading conversation…"),
            );
        } else if self.transcript.entries().is_empty() {
            list = list.child(
                div()
                    .p_8()
                    .text_sm()
                    .text_color(theme.muted)
                    .child("No messages yet. Say hello below."),
            );
        } else {
            for entry in self.transcript.entries() {
                list = list.child(self.render_entry(entry, theme));
            }
        }

        if let Some(error) = self.transcript.error() {
            list = list.child(
                div()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.danger)
                    .bg(theme.surface)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(error.to_owned()),
                    ),
            );
        }

        list
    }

    fn render_entry(&self, entry: &Entry, theme: Theme) -> impl IntoElement {
        match entry {
            Entry::User { text, .. } => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(theme.muted).child("You"))
                .child(div().min_w_0().text_sm().child(text.clone())),
            Entry::Assistant(assistant) => {
                let mut label = String::from("Assistant");
                if let Some(agent) = &assistant.agent {
                    label.push_str(" · ");
                    label.push_str(agent);
                }
                if let Some(model) = &assistant.model {
                    label.push_str(" · ");
                    label.push_str(model);
                }

                let mut block = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_xs().text_color(theme.muted).child(label));

                for part in &assistant.parts {
                    block = block.child(match part {
                        Content::Text { text, ordinal } => {
                            if text.trim().is_empty() {
                                div()
                                    .text_sm()
                                    .text_color(theme.muted)
                                    .child("…")
                                    .into_any_element()
                            } else {
                                TextView::markdown(
                                    format!("md-{}-{ordinal}", assistant.id),
                                    text.clone(),
                                )
                                .min_w_0()
                                .text_sm()
                                .into_any_element()
                            }
                        }
                        Content::Reasoning { text, .. } => {
                            let body = if text.trim().is_empty() {
                                "Thinking…".to_string()
                            } else {
                                text.clone()
                            };
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_xs().text_color(theme.muted).child("Thinking"))
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_sm()
                                        .italic()
                                        .text_color(theme.muted)
                                        .child(body),
                                )
                                .into_any_element()
                        }
                        Content::Tool(tool) => self.render_tool(tool, theme).into_any_element(),
                    });
                }

                if assistant.parts.is_empty() && assistant.error.is_none() {
                    block = block.child(div().text_sm().text_color(theme.muted).child("…"));
                }
                if let Some(error) = &assistant.error {
                    block = block.child(
                        div()
                            .min_w_0()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.danger)
                            .bg(theme.surface)
                            .text_sm()
                            .text_color(theme.danger)
                            .child(format!("Error: {error}")),
                    );
                }

                block
            }
            Entry::System { text, .. } => {
                let first_line = text.lines().next().unwrap_or_default().to_owned();
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(first_line)
            }
        }
    }

    fn render_tool(&self, tool: &ToolCall, theme: Theme) -> impl IntoElement {
        let status_color = match tool.status {
            ToolStatus::Streaming | ToolStatus::Unknown => theme.muted,
            ToolStatus::Running => theme.warning,
            ToolStatus::Completed => theme.success,
            ToolStatus::Error => theme.danger,
        };
        let border_color = if tool.status == ToolStatus::Error {
            theme.danger
        } else {
            theme.border
        };

        let mut card = div()
            .min_w_0()
            .rounded_md()
            .border_1()
            .border_color(border_color)
            .bg(theme.surface)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .child(format!("⚙ {}", tool.name)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(status_color)
                            .child(tool.status.label()),
                    ),
            );

        if !tool.input.trim().is_empty() {
            card = card.child(
                div()
                    .px_3()
                    .pb_2()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(one_line(&tool.input, 180)),
            );
        }

        if let Some(output) = &tool.output {
            card = card.child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.bg)
                    .px_3()
                    .py_2()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.text)
                    .child(truncate_text(output, 1500)),
            );
        }

        if let Some(error) = &tool.error {
            card = card.child(
                div()
                    .border_t_1()
                    .border_color(theme.danger)
                    .px_3()
                    .py_2()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(truncate_text(error, 1500)),
            );
        }

        card
    }

    fn composer_bar(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.transcript.is_busy();
        let ready = self.status.is_ready() && self.sessions.selected_id().is_some();

        let mut bar = div()
            .border_t_1()
            .border_color(theme.border)
            .p_4()
            .flex()
            .items_end()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Textarea::new(&self.composer).h(px(84.))),
            );

        if busy {
            bar = bar.child(
                Button::new("stop-turn")
                    .small()
                    .danger()
                    .label("Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_turn(cx))),
            );
        } else {
            bar = bar.child(
                Button::new("send-prompt")
                    .small()
                    .primary()
                    .label("Send")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.send_prompt(window, cx))),
            );
        }

        bar
    }

    fn rename_panel(
        &self,
        input: &Entity<InputState>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("Session title"),
            )
            .child(Input::new(input).w_full())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("rename-save")
                            .small()
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| this.commit_rename(cx))),
                    )
                    .child(
                        Button::new("rename-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_rename(cx))),
                    ),
            )
    }

    fn delete_confirm(
        &self,
        session: &Session,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = session
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "this session".into());

        div()
            .mx_6()
            .mt_4()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.danger)
            .bg(theme.surface)
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(format!("Delete “{title}”? This cannot be undone.")),
            )
            .child(
                Button::new("confirm-delete")
                    .small()
                    .danger()
                    .label("Delete")
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
            )
            .child(
                Button::new("cancel-delete")
                    .small()
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx))),
            )
    }
}

/// Collapse whitespace so an argument summary stays on one line.
fn one_line(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text(&flat, max_chars)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        out.push_str("\n… (truncated)");
    }
    out
}

fn session_subtitle(session: &Session) -> SharedString {
    let mut parts = vec![session.id.clone()];

    if let Some(agent) = &session.agent {
        parts.push(format!("agent {agent}"));
    }

    if let Some(model) = &session.model {
        let provider = model.provider_id.as_deref().unwrap_or_default();
        let mut name = if provider.is_empty() {
            model.id.clone()
        } else {
            format!("{provider}/{}", model.id)
        };
        if let Some(variant) = &model.variant
            && !variant.is_empty()
        {
            name = format!("{name} ({variant})");
        }
        parts.push(name);
    }

    let updated = relative_time(now_ms(), session.time.updated);
    if !updated.is_empty() {
        parts.push(updated);
    }

    parts.join(" · ").into()
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        div()
            .key_context("Workspace")
            .on_action(cx.listener(|this, _: &NewSession, _, cx| this.create_session(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleTheme, window, cx| this.toggle_theme(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusComposer, window, cx| this.focus_composer(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CancelOverlays, _, cx| this.cancel_overlays(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .child(self.render_title_bar(cx))
            .child(
                div()
                    .debug_selector(|| "workspace-body".into())
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui_kit::component::Theme as KitTheme;
    use gpui_kit::{Modifiers, TestAppContext, point, px};
    use harness_engine::{Engine, EngineConfig};

    use super::WorkspaceView;

    #[gpui_kit::test]
    fn title_bar_is_rendered_above_the_workspace(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-titlebar-test");
        let engine =
            Engine::spawn(EngineConfig::new(&project_dir).with_binary("/nonexistent/opencode"));

        let (_, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(window, engine, project_dir.clone(), cx)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let title_bar = cx
            .debug_bounds("workspace-title-bar")
            .expect("the custom title bar is rendered");
        let content = cx
            .debug_bounds("workspace-body")
            .expect("the workspace body is rendered below the title bar");

        assert_eq!(title_bar.origin.y, px(0.));
        assert_eq!(title_bar.size.height, px(34.));
        assert!(title_bar.origin.y + title_bar.size.height <= content.origin.y);

        // A click on the bar must go through its mouse handlers without
        // panicking; the test platform does not implement window dragging.
        let bar_center = point(px(400.), px(17.));
        cx.simulate_click(bar_center, Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("workspace-title-bar").is_some());
    }
}
