//! The M1 workspace: project-scoped sessions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, Theme as KitTheme, ThemeMode, TitleBar,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    text::TextView,
};
use gpui_kit::*;
use harness_core::{
    AuthFlow, Content, ContextStore, DiffLineKind, DiffRow, DraftSend, EngineStatus, Entry,
    FormStore, InboxStore, McpStore, Notification, NotifyContext, NotifyInput, NotifyKind,
    NotifyPolicy, NotifySettings, ParsedDiff, PermissionQueue, PickerEffect, PickerItem,
    PickerKind, PickerState, PluginStore, ProjectDrafts, ProjectStore, PromptDraft, ProviderStore,
    RevertStore, SNIPPETS, SavedPermissionStore, ScheduleStore, SessionStore, ToolCall, ToolStatus,
    TranscriptStore, TransferStore, UsageStore, VcsStore, describe_when, failure_reason, flatten,
    import_error, next_run_label, now_ms, parse_patch, path_to_file_uri, relative_time, url_host,
    validate_duration, validate_import, visible_range,
};
use harness_engine::{CatalogData, CatalogKind, Engine, EngineEvent, PermissionReply, StreamEvent};
use harness_protocol::{
    AgentInfo, AuthStatus, CommandAttempt, CommandInfo, CreateScheduleRequest, FileDiff,
    InboxDelivery, IntegrationMethod, McpStatus, ModelInfo, OAuthAttempt, PermissionRequest,
    Project, ProjectId, ScheduleStatus, ScheduleWhen, Session, SkillInfo,
};
use harness_ui::{Theme, attachment_chip, status_dot};

use crate::notify::NotifyChannels;
use crate::project_switcher::ProjectSwitcher;
use crate::transfer::{
    ImportFile, StatsPreset, TransferState, compact_count, money, token_summary,
};

/// Server-backed lists the composer pickers filter locally.
#[derive(Debug, Default)]
struct CatalogCache {
    commands: Vec<CommandInfo>,
    skills: Vec<SkillInfo>,
    agents: Vec<AgentInfo>,
    models: Vec<ModelInfo>,
}

/// The connect flow currently open for one integration.
#[derive(Debug, Clone)]
struct ProviderDialog {
    integration_id: String,
    integration_name: String,
    method: Option<IntegrationMethod>,
}

impl ProviderDialog {
    fn new(integration_id: String, integration_name: String) -> Self {
        Self {
            integration_id,
            integration_name,
            method: None,
        }
    }
}

/// Owned snapshot of where an event happened, so the policy can be borrowed
/// mutably while its context still points at stable local strings.
struct NotifyTarget {
    session_id: String,
    session_title: String,
    focused: bool,
    active_session: bool,
}

impl NotifyTarget {
    fn context(&self) -> NotifyContext<'_> {
        NotifyContext {
            session_id: &self.session_id,
            session_title: &self.session_title,
            focused: self.focused,
            active_session: self.active_session,
        }
    }
}

impl CatalogCache {
    fn items(&self, kind: PickerKind) -> Vec<PickerItem> {
        match kind {
            PickerKind::Command => self
                .commands
                .iter()
                .map(|command| PickerItem::Command {
                    name: command.name.clone(),
                    description: command.description.clone(),
                })
                .collect(),
            PickerKind::Skill => self
                .skills
                .iter()
                .map(|skill| PickerItem::Skill {
                    id: skill.id.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                })
                .collect(),
            PickerKind::File | PickerKind::Agent => self.agents_items(),
            PickerKind::Model => self
                .models
                .iter()
                .filter(|model| model.enabled)
                .map(|model| PickerItem::Model {
                    model: model.model_ref(),
                    name: model.name.clone(),
                    provider_id: model.provider_id.clone(),
                })
                .collect(),
            PickerKind::Snippet => SNIPPETS
                .iter()
                .map(|snippet| PickerItem::Snippet {
                    name: snippet.name.to_string(),
                    description: snippet.description.to_string(),
                    expansion: snippet.expansion.to_string(),
                })
                .collect(),
        }
    }

    fn agents_items(&self) -> Vec<PickerItem> {
        self.agents
            .iter()
            .filter(|agent| !agent.hidden)
            .map(|agent| PickerItem::Agent {
                id: agent.id.clone(),
                name: agent.name.clone(),
                description: agent.description.clone(),
            })
            .collect()
    }
}

gpui_kit::actions!(
    harness_views,
    [
        NewSession,
        ToggleTheme,
        FocusComposer,
        CancelOverlays,
        ToggleProjectSwitcher
    ]
);

/// The project list in the form the app persists: canonical paths plus the
/// active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSnapshot {
    pub projects: Vec<String>,
    pub active: Option<String>,
}

/// Where the app sends snapshots; the view never touches the disk itself.
pub type ProjectSink = Arc<dyn Fn(ProjectSnapshot) + Send + Sync>;

/// Root view. Owns the engine handle so the child is stopped when the view is
/// dropped at app exit.
pub struct WorkspaceView {
    status: EngineStatus,
    sessions: SessionStore,
    projects: ProjectStore,
    switcher: ProjectSwitcher,
    drafts: ProjectDrafts<PromptDraft>,
    project_dir: PathBuf,
    projects_sink: Option<ProjectSink>,
    /// Startup notice about saved projects that could not be reopened.
    projects_notice: Option<String>,
    /// Last command error, shown until dismissed or superseded.
    error: Option<String>,
    /// Open title editor, if any.
    renaming: Option<Entity<InputState>>,
    /// Session awaiting delete confirmation.
    pending_delete: Option<String>,
    transcript: TranscriptStore,
    permissions: PermissionQueue,
    inbox: InboxStore,
    forms: FormStore,
    revert: RevertStore,
    context: ContextStore,
    usage: UsageStore,
    plugins: PluginStore,
    pending_plugin_update: Option<String>,
    mcp: McpStore,
    saved_permissions: SavedPermissionStore,
    providers: ProviderStore,
    provider_dialog: Option<ProviderDialog>,
    provider_key: Entity<InputState>,
    transfer: TransferState,
    transfer_store: TransferStore,
    stats_request: u64,
    schedules: ScheduleStore,
    schedules_session: Option<String>,
    show_finished_schedules: bool,
    schedule_form_open: bool,
    schedule_kind: usize,
    schedule_prompt: Entity<InputState>,
    schedule_when: Entity<InputState>,
    schedule_ends: Entity<InputState>,
    pending_schedule_delete: Option<String>,
    schedule_request: u64,
    pending_saved_permission: Option<String>,
    pending_mcp_remove: Option<String>,
    mcp_add_open: bool,
    mcp_name: Entity<InputState>,
    mcp_config: Entity<InputState>,
    /// Coalesces `mcp.*` stream events; the list is reloaded once they settle.
    mcp_reload_due: Option<Instant>,
    form_input: Entity<InputState>,
    _form_subscription: Subscription,
    form_key: Option<String>,
    /// The request whose reply is in flight; its buttons stay disabled.
    permission_in_flight: Option<String>,
    /// Whether the live event stream is currently open.
    stream_connected: bool,
    composer: Entity<TextareaState>,
    /// Sends the composer text when Enter is pressed.
    _composer_subscription: Subscription,
    draft: PromptDraft,
    /// The picker above the composer, for triggers or the model/agent badges.
    picker: Option<PickerState>,
    catalog: CatalogCache,
    /// Search field used by the model and agent pickers.
    picker_input: Option<Entity<InputState>>,
    _picker_input_subscription: Option<Subscription>,
    /// Debounces `@` file searches; dropping it cancels the pending query.
    file_debounce: Option<Task<()>>,
    engine: Engine,
    vcs: VcsStore,
    diff_files: Vec<FileDiff>,
    diff: ParsedDiff,
    diff_rows: Vec<DiffRow>,
    selected_file: Option<usize>,
    diff_scroll: ScrollHandle,
    vcs_request: u64,
    notify: NotifyPolicy,
    notify_channels: NotifyChannels,
    /// Whether the window had OS focus when the current event arrived.
    notify_focused: bool,
    /// The OS refused permission; the switch stays off until the user turns it
    /// back on.
    notify_denied: bool,
    _notify_status_task: Task<()>,
    _notify_flush_task: Task<()>,
    _events_task: Task<()>,
}

impl WorkspaceView {
    pub fn new(
        window: &mut Window,
        engine: Engine,
        project_dir: PathBuf,
        notify_settings: NotifySettings,
        notify_channels: NotifyChannels,
        cx: &mut Context<Self>,
    ) -> Self {
        let project_dir = std::fs::canonicalize(&project_dir).unwrap_or(project_dir);
        let mut projects = ProjectStore::default();
        let id = projects.add(project_dir.to_string_lossy().into_owned());
        projects.select(&id);
        let notify = NotifyPolicy::new(notify_settings);
        let events = engine.events();
        let events_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                // Focus is only needed for events the notification policy maps;
                // streaming deltas must not pay for a window lookup.
                let relevant = matches!(
                    &event,
                    EngineEvent::Stream(
                        StreamEvent::ExecutionFinished { .. }
                            | StreamEvent::PermissionAsked { .. }
                            | StreamEvent::FormCreated { .. }
                    ) | EngineEvent::FormCreated { .. }
                );
                let focused = relevant
                    && cx
                        .update(|window, _| window.is_window_active())
                        .unwrap_or(false);
                let updated = this.update(cx, |this, cx| {
                    this.notify_focused = focused;
                    this.apply(event);
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        // Throttled notifications wait for their window to close.
        let flush_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let updated = this.update(cx, |this, _| {
                    let now = Instant::now();
                    for notification in this.notify.flush(now) {
                        this.deliver(notification);
                    }
                    if this.mcp_reload_due.is_some_and(|due| now >= due) {
                        this.mcp_reload_due = None;
                        this.engine.load_mcp(this.directory(), 3);
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let status = notify_channels.status.clone();
        let status_task = cx.spawn(async move |this, cx| {
            while status.recv().await.is_ok() {
                let updated = this.update(cx, |this, cx| this.on_notify_denied(cx));
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
        let mcp_name = cx.new(|cx| InputState::new(window, cx).placeholder("server name"));
        let mcp_config = cx.new(|cx| {
            InputState::new(window, cx).placeholder("{\"command\": \"npx\", \"args\": []}")
        });
        // Masked at construction: the key is never rendered in plain text.
        let provider_key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API key (never stored)")
        });
        let form_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type the answer…"));
        let schedule_prompt =
            cx.new(|cx| InputState::new(window, cx).placeholder("Prompt to send…"));
        let schedule_when = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 5 minutes"));
        let schedule_ends =
            cx.new(|cx| InputState::new(window, cx).placeholder("Run limit (optional)"));
        let form_subscription =
            cx.subscribe_in(&form_input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.form_answer_changed(cx);
                }
            });

        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.composer_changed(cx),
                InputEvent::PressEnter { shift: false, .. } => {
                    if this
                        .picker
                        .as_ref()
                        .is_some_and(|picker| !is_session_picker(picker.kind()))
                    {
                        if this
                            .picker
                            .as_ref()
                            .is_some_and(|picker| picker.visible_len() > 0)
                        {
                            this.choose_selected(window, cx);
                        }
                    } else {
                        this.send_prompt(window, cx);
                    }
                }
                _ => {}
            },
        );

        Self {
            status: EngineStatus::default(),
            sessions: SessionStore::default(),
            projects,
            switcher: ProjectSwitcher::default(),
            drafts: ProjectDrafts::default(),
            project_dir,
            projects_sink: None,
            projects_notice: None,
            error: None,
            renaming: None,
            pending_delete: None,
            transcript: TranscriptStore::default(),
            permissions: PermissionQueue::default(),
            inbox: InboxStore::default(),
            forms: FormStore::default(),
            revert: RevertStore::default(),
            context: ContextStore::default(),
            usage: UsageStore::default(),
            plugins: PluginStore::default(),
            pending_plugin_update: None,
            mcp: McpStore::default(),
            saved_permissions: SavedPermissionStore::default(),
            providers: ProviderStore::default(),
            provider_dialog: None,
            provider_key,
            transfer: TransferState::default(),
            transfer_store: TransferStore::default(),
            stats_request: 0,
            schedules: ScheduleStore::default(),
            schedules_session: None,
            show_finished_schedules: false,
            schedule_form_open: false,
            schedule_kind: 0,
            schedule_prompt,
            schedule_when,
            schedule_ends,
            pending_schedule_delete: None,
            schedule_request: 0,
            pending_saved_permission: None,
            pending_mcp_remove: None,
            mcp_add_open: false,
            mcp_name,
            mcp_config,
            mcp_reload_due: None,
            form_input,
            _form_subscription: form_subscription,
            form_key: None,
            permission_in_flight: None,
            stream_connected: false,
            composer,
            _composer_subscription: composer_subscription,
            draft: PromptDraft::default(),
            picker: None,
            catalog: CatalogCache::default(),
            picker_input: None,
            _picker_input_subscription: None,
            file_debounce: None,
            engine,
            vcs: VcsStore::default(),
            diff_files: Vec::new(),
            diff: ParsedDiff::default(),
            diff_rows: Vec::new(),
            selected_file: None,
            diff_scroll: ScrollHandle::new(),
            vcs_request: 0,
            notify,
            notify_channels,
            notify_focused: false,
            notify_denied: false,
            _notify_status_task: status_task,
            _notify_flush_task: flush_task,
            _events_task: events_task,
        }
    }

    /// Merge the projects remembered from the last run. The startup project is
    /// already selected, so adding the rest must not change it.
    pub fn restore_saved_projects(mut self, projects: Vec<String>, notice: Option<String>) -> Self {
        for canonical in projects {
            self.projects.add(canonical);
        }
        self.projects_notice = notice;
        self.persist_projects();
        self
    }

    /// Attach the app-owned snapshot sink and record the current state once.
    pub fn persist_projects_with(mut self, sink: ProjectSink) -> Self {
        self.projects_sink = Some(sink);
        self.persist_projects();
        self
    }

    fn persist_projects(&self) {
        let Some(sink) = &self.projects_sink else {
            return;
        };
        let snapshot = ProjectSnapshot {
            projects: self
                .projects
                .projects()
                .iter()
                .map(|project| project.canonical.clone())
                .collect(),
            active: self
                .projects
                .active()
                .map(|project| project.canonical.clone()),
        };
        sink(snapshot);
    }

    fn apply(&mut self, event: EngineEvent) {
        self.notify_from_event(&event);
        match event {
            EngineEvent::Starting { port } => {
                self.status = EngineStatus::Starting { port: Some(port) };
            }
            EngineEvent::Ready { version, port } => {
                self.request_vcs_status();
                self.engine.load_usage_limits(self.directory(), 1);
                self.engine.load_plugins(self.directory(), 1);
                self.engine.load_mcp(self.directory(), 1);
                self.engine.load_providers(self.directory(), 1);
                self.engine.load_integrations(self.directory(), 1);
                self.status = EngineStatus::Ready { version, port };
                for (request_id, kind) in [
                    (1, CatalogKind::Commands),
                    (2, CatalogKind::Skills),
                    (3, CatalogKind::Agents),
                    (4, CatalogKind::Models),
                ] {
                    self.engine.list_catalog(self.directory(), request_id, kind);
                }
            }
            EngineEvent::Failed { message } => {
                self.status = EngineStatus::Failed { message };
            }
            EngineEvent::Stopped => {
                self.status = EngineStatus::Stopped;
            }
            EngineEvent::Log(_) => {}
            EngineEvent::ProjectsLoaded { result } => {
                if let Err(message) = result {
                    self.error = Some(message);
                }
            }
            EngineEvent::ProjectSelected { id, canonical } => {
                if !self.projects.select(&id) {
                    let mut project = Project::local(canonical.clone());
                    project.id = id.clone();
                    self.projects.upsert(project);
                    self.projects.select(&id);
                }
                self.project_dir = PathBuf::from(&canonical);
                self.reset_project_scoped();
                self.switcher.close();
                self.persist_projects();
                self.reload_project_scoped();
            }
            EngineEvent::SessionsLoaded { sessions } => {
                let previous = self.sessions.selected_id().map(str::to_owned);
                self.sessions.replace(sessions);
                if let Some(session) = self.sessions.selected() {
                    self.apply_session_usage(&session.clone());
                }
                let current = self.sessions.selected_id().map(str::to_owned);
                self.error = None;
                if previous != current {
                    // First load (or the selected session disappeared): open
                    // the newly selected session's timeline.
                    self.follow_selection();
                }
            }
            EngineEvent::SchedulesLoaded {
                session_id, result, ..
            } => {
                if self.schedules_session.as_deref() == Some(session_id.as_str()) {
                    match result {
                        Ok(schedules) => {
                            self.schedules.replace(schedules);
                            self.error = None;
                        }
                        Err(message) => self.error = Some(message),
                    }
                }
            }
            EngineEvent::ScheduleCreated { .. } | EngineEvent::ScheduleChanged { .. } => {}
            EngineEvent::ScheduleDeleted { id } => {
                if self.pending_schedule_delete.as_deref() == Some(id.as_str()) {
                    self.pending_schedule_delete = None;
                }
            }
            EngineEvent::ProvidersLoaded { result, .. } => {
                if let Ok(providers) = result {
                    self.providers.set_providers(providers);
                }
            }
            EngineEvent::IntegrationsLoaded { result, .. } => {
                if let Ok(integrations) = result {
                    self.providers.set_integrations(integrations);
                }
            }
            EngineEvent::AuthAttemptStarted {
                integration_id,
                kind,
                method,
                attempt_id,
                url,
                command,
                instructions,
            } => match kind.as_str() {
                "oauth" => {
                    let attempt = OAuthAttempt {
                        attempt_id,
                        url: url.unwrap_or_default(),
                        instructions,
                        ..OAuthAttempt::default()
                    };
                    self.providers
                        .begin_oauth(&integration_id, &method, &attempt);
                }
                "command" => {
                    let attempt = CommandAttempt { attempt_id };
                    self.providers.begin_command(
                        &integration_id,
                        &method,
                        &attempt,
                        command.unwrap_or_default(),
                    );
                }
                _ => {}
            },
            EngineEvent::AuthAttemptUpdated {
                integration_id,
                status,
                ..
            } => {
                if status == AuthStatus::Complete {
                    self.providers.mark_connected(&integration_id);
                }
            }
            EngineEvent::AuthAttemptFailed {
                integration_id,
                message,
            } => {
                self.providers.fail(&integration_id, &message);
                // Only the redacted form reaches the banner.
                self.error = self
                    .providers
                    .last_error(&integration_id)
                    .map(str::to_string);
                self.close_provider_dialog(&integration_id);
            }
            EngineEvent::AuthAttemptCancelled { integration_id } => {
                self.providers.cancel(&integration_id);
                self.close_provider_dialog(&integration_id);
            }
            EngineEvent::ProviderConnected { integration_id } => {
                self.providers.mark_connected(&integration_id);
                self.error = None;
                self.close_provider_dialog(&integration_id);
                self.engine.load_integrations(self.directory(), 2);
            }
            EngineEvent::SavedPermissionsLoaded { result, .. } => {
                if let Ok(rules) = result {
                    self.saved_permissions.replace(rules);
                }
            }
            EngineEvent::SavedPermissionDeleted { id, result } => {
                self.pending_saved_permission = None;
                if result.is_ok() {
                    self.saved_permissions.remove(&id);
                }
            }
            EngineEvent::McpLoaded { result, .. } => {
                if let Ok(servers) = result {
                    self.mcp.replace(servers);
                }
            }
            EngineEvent::McpChanged { .. } => {
                self.engine.load_mcp(self.directory(), 2);
            }
            EngineEvent::McpError { message, .. } => {
                self.pending_mcp_remove = None;
                self.error = Some(message);
            }
            EngineEvent::PluginsLoaded { result, .. } => {
                if let Ok(plugins) = result {
                    self.plugins.replace(plugins);
                }
            }
            EngineEvent::PluginsChecked { .. } => {}
            EngineEvent::PluginUpdated { .. } => {
                self.pending_plugin_update = None;
                self.engine.load_plugins(self.directory(), 2);
            }
            EngineEvent::UsageLimitsLoaded { result, .. } => match result {
                Ok(limits) => self.usage.set(limits, now_ms()),
                Err(_) => self.usage.mark_unavailable(now_ms()),
            },
            EngineEvent::CompactionRequested { .. } | EngineEvent::ContextLoaded { .. } => {}
            EngineEvent::RevertStaged { session_id, revert } => {
                self.revert.stage(&session_id, revert);
            }
            EngineEvent::RevertCommitted { session_id, .. } => {
                self.revert.acknowledge();
                self.transcript.begin(&session_id);
                self.engine.load_transcript(self.directory(), session_id);
            }
            EngineEvent::RevertCleared { .. } => self.revert.clear(),
            EngineEvent::FormCreated { form } => {
                self.forms.upsert(form);
            }
            EngineEvent::FormsLoaded { forms, .. } => {
                self.forms.replace(forms);
                self.form_key = None;
            }
            EngineEvent::FormSettled { form_id } => self.forms.remove(&form_id),
            EngineEvent::FormError { message } => self.forms.set_error(message),
            EngineEvent::InboxLoaded { session_id, items } => {
                self.inbox.replace(&session_id, items);
            }
            EngineEvent::SessionForked { session } => {
                self.apply_session_usage(&session);
                let id = session.id.clone();
                self.sessions.upsert(session);
                self.sessions.select(&id);
                self.error = None;
                self.transcript.begin(&id);
                self.engine.load_transcript(self.directory(), &id);
                self.engine.load_permissions(self.directory(), &id);
                self.reset_schedules(&id);
            }
            EngineEvent::SessionCreated { session } => {
                self.apply_session_usage(&session);
                let id = session.id.clone();
                self.sessions.upsert(session);
                self.sessions.select(&id);
                self.renaming = None;
                self.pending_delete = None;
                self.error = None;
                self.transcript.begin(&id);
                self.engine.load_transcript(self.directory(), id.clone());
                self.permissions.begin(&id);
                self.engine.load_permissions(self.directory(), id.clone());
                self.inbox.begin(&id);
                self.engine.load_inbox(self.directory(), id.clone());
                self.engine.load_forms(self.directory(), id.clone());
                self.reset_schedules(&id);
                if let Some(project_id) = self
                    .sessions
                    .selected()
                    .and_then(|session| session.project_id.clone())
                {
                    self.engine
                        .load_saved_permissions(self.directory(), 1, project_id);
                }
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
            EngineEvent::SessionExported {
                session_id,
                path,
                bytes,
            } => {
                self.transfer.exporting = false;
                self.transfer.export_notice =
                    Some(format!("Saved {bytes} bytes to {}.", path.display()));
                self.transfer_store.record_export(session_id, path, bytes);
                self.error = None;
            }
            EngineEvent::SessionExportError { message } => {
                self.transfer.exporting = false;
                self.error = Some(message);
            }
            EngineEvent::ImportPreviewed { path, transfer } => match validate_import(&transfer) {
                Ok(preview) => {
                    self.error = None;
                    self.transfer.import_preview = Some(ImportFile { path, preview });
                }
                Err(message) => {
                    self.transfer.importing = false;
                    self.error = Some(message);
                }
            },
            EngineEvent::SessionImported { session } => {
                self.transfer.importing = false;
                if let Some(file) = self.transfer.import_preview.take() {
                    self.transfer_store.record_import(file.preview);
                }
                self.apply_session_usage(&session);
                let id = session.id.clone();
                self.sessions.upsert(session);
                self.sessions.select(&id);
                self.error = None;
                self.transcript.begin(&id);
                self.engine.load_transcript(self.directory(), id.clone());
                self.permissions.begin(&id);
                self.engine.load_permissions(self.directory(), id.clone());
                self.inbox.begin(&id);
                self.engine.load_inbox(self.directory(), id.clone());
                self.engine.load_forms(self.directory(), id.clone());
                self.reset_schedules(&id);
                self.engine.refresh_sessions(self.directory());
            }
            EngineEvent::SessionImportError { message } => {
                self.transfer.importing = false;
                self.error = Some(match &self.transfer.import_preview {
                    Some(file) => import_error(&message, &file.preview),
                    None => message,
                });
            }
            EngineEvent::StatsLoaded { request_id, result } => {
                if request_id != self.stats_request {
                    return;
                }
                self.transfer.stats_loading = false;
                match result {
                    Ok(stats) => {
                        self.transfer_store.cache_stats(stats);
                        self.error = None;
                    }
                    Err(message) => {
                        self.error = Some(format!("Could not load statistics: {message}"));
                    }
                }
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
                self.engine.load_inbox(self.directory(), session_id.clone());
                self.transcript.accept_prompt(
                    &session_id,
                    &message.id,
                    &message.payload.text,
                    message.time.created,
                );
            }
            EngineEvent::CommandAccepted { session_id } => {
                if self.sessions.selected_id() == Some(session_id.as_str()) {
                    self.follow_selection();
                }
            }
            EngineEvent::SessionModelSet { session_id, model } => {
                if self.sessions.set_model(&session_id, model) {
                    self.error = None;
                }
            }
            EngineEvent::SessionAgentSet { session_id, agent } => {
                if self.sessions.set_agent(&session_id, agent) {
                    self.error = None;
                }
            }
            EngineEvent::FilesFound { request_id, result } => {
                if let Some(picker) = self.picker.as_mut()
                    && picker.request_id() == request_id
                {
                    match result {
                        Ok(entries) => {
                            let items = entries
                                .into_iter()
                                .filter(|entry| entry.kind.is_empty() || entry.kind == "file")
                                .map(|entry| PickerItem::File { path: entry.path });
                            picker.apply_file_results(request_id, items);
                        }
                        Err(message) => picker.set_error(message),
                    }
                }
            }
            EngineEvent::CatalogLoaded { kind, result, .. } => match result {
                Ok(data) => {
                    match data {
                        CatalogData::Commands(commands) => self.catalog.commands = commands,
                        CatalogData::Skills(skills) => self.catalog.skills = skills,
                        CatalogData::Agents(agents) => self.catalog.agents = agents,
                        CatalogData::Models(models) => self.catalog.models = models,
                    }
                    if let Some(picker) = self.picker.as_mut()
                        && catalog_kind(picker.kind()) == Some(kind)
                    {
                        picker.add_items(self.catalog.items(picker.kind()));
                    }
                }
                Err(message) => {
                    if let Some(picker) = self.picker.as_mut()
                        && catalog_kind(picker.kind()) == Some(kind)
                    {
                        picker.set_error(message);
                    }
                }
            },
            EngineEvent::Stream(event) => match event {
                StreamEvent::FilesystemChanged => self.on_filesystem_changed(),
                StreamEvent::McpChanged => {
                    self.mcp_reload_due = Some(Instant::now() + Duration::from_millis(250));
                }
                StreamEvent::UsageUpdated { tokens, cost, .. } => {
                    self.context.set_usage(tokens, cost);
                }
                StreamEvent::CompactionStarted { .. } => self.context.begin_compaction(),
                StreamEvent::CompactionEnded { .. } => self.context.finish_compaction(),
                StreamEvent::CompactionFailed { message, .. } => {
                    self.context.fail_compaction(message)
                }
                StreamEvent::PermissionAsked { request } => {
                    self.permissions.push(request);
                }
                StreamEvent::PermissionReplied { request_id, .. } => {
                    self.permissions.remove(&request_id);
                    if self.permission_in_flight.as_deref() == Some(request_id.as_str()) {
                        self.permission_in_flight = None;
                    }
                }
                StreamEvent::FormCreated { form } => self.forms.upsert(form),
                event => {
                    let finished = matches!(event, StreamEvent::ExecutionFinished { .. });
                    if self.transcript.apply_stream(event) && finished {
                        // The final message (finish reason, error, full text)
                        // is only authoritative on the server.
                        self.follow_selection();
                    }
                }
            },
            EngineEvent::VcsStatusLoaded { result, .. } => match result {
                Ok(files) => {
                    let empty = files.is_empty();
                    self.vcs.replace(files);
                    self.vcs.finish_refresh();
                    if !empty && self.diff_files.is_empty() {
                        self.request_vcs_diff();
                    }
                }
                Err(message) => self.vcs.set_error(message),
            },
            EngineEvent::VcsBaseLoaded { result, .. } => {
                if let Ok(Some(base)) = result {
                    self.vcs.set_branch(base.name.clone());
                    self.vcs.set_base(Some(base));
                }
            }
            EngineEvent::RepoInitialized { result, .. } => {
                if result.is_ok() {
                    self.request_vcs_status();
                }
            }
            EngineEvent::VcsDiffLoaded { result, .. }
            | EngineEvent::SessionDiffLoaded { result, .. } => {
                if let Ok(files) = result {
                    self.diff_files = files;
                    self.rebuild_diff();
                }
            }
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
                self.engine.load_mcp(self.directory(), 3);
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
                self.engine.load_permissions(self.directory(), session_id);
            }
            EngineEvent::Interrupted { .. } => {}
            // There is no terminal surface yet; M14-02 consumes these.
            EngineEvent::PtyCreated { .. }
            | EngineEvent::PtysLoaded { .. }
            | EngineEvent::PtyOutput { .. }
            | EngineEvent::PtyClosed { .. }
            | EngineEvent::PtyError { .. } => {}
            EngineEvent::CommandFailed { message } => {
                self.permission_in_flight = None;
                self.error = Some(message);
                // A failed model/agent change may have left an optimistic badge;
                // the server snapshot is authoritative.
                self.engine.refresh_sessions(self.directory());
            }
        }
    }

    /// Feed an event to the notification policy. Runs before the event is
    /// consumed, so the payload is still borrowable.
    fn notify_from_event(&mut self, event: &EngineEvent) {
        let now = Instant::now();
        match event {
            EngineEvent::Stream(stream) => {
                let session_id = stream.session_id();
                if session_id.is_empty() {
                    return;
                }
                let target = self.notify_target(session_id);
                if let Some(notification) =
                    self.notify.on_stream_event(stream, target.context(), now)
                {
                    self.deliver(notification);
                }
            }
            EngineEvent::FormCreated { form } => {
                let target = self.notify_target(&form.session_id);
                let notification = self.notify.on_event(
                    NotifyInput {
                        kind: NotifyKind::QuestionAsked,
                        event_key: &form.id,
                        preview: Some(form.title.as_str()).filter(|title| !title.is_empty()),
                        context: target.context(),
                    },
                    now,
                );
                if let Some(notification) = notification {
                    self.deliver(notification);
                }
            }
            _ => {}
        }
    }

    fn notify_target(&self, session_id: &str) -> NotifyTarget {
        let session_title = self
            .sessions
            .sessions()
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| session.title.clone())
            .unwrap_or_default();
        NotifyTarget {
            session_id: session_id.to_string(),
            session_title,
            focused: self.notify_focused,
            active_session: self.sessions.selected_id() == Some(session_id),
        }
    }

    fn deliver(&mut self, notification: Notification) {
        match self.notify_channels.notifications.try_send(notification) {
            Ok(()) => {}
            Err(_) => {
                // The app stopped draining at shutdown; delivery is best effort.
            }
        }
    }

    fn on_notify_denied(&mut self, cx: &mut Context<Self>) {
        if self.notify_denied {
            return;
        }
        self.notify_denied = true;
        let mut settings = self.notify.settings().clone();
        settings.enabled = false;
        self.notify.set_settings(settings.clone());
        self.persist_notify_settings(settings);
        cx.notify();
    }

    fn persist_notify_settings(&self, settings: NotifySettings) {
        match self.notify_channels.settings.try_send(settings) {
            Ok(()) => {}
            Err(_) => {
                // The app is shutting down; the next launch keeps the old file.
            }
        }
    }

    fn set_notify_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let mut settings = self.notify.settings().clone();
        settings.enabled = enabled;
        if enabled {
            self.notify_denied = false;
        }
        self.update_notify_settings(settings, cx);
    }

    fn set_notify_kind(&mut self, kind: NotifyKind, on: bool, cx: &mut Context<Self>) {
        let mut settings = self.notify.settings().clone();
        settings.set(kind, on);
        self.update_notify_settings(settings, cx);
    }

    fn set_notify_preview(&mut self, on: bool, cx: &mut Context<Self>) {
        let mut settings = self.notify.settings().clone();
        settings.include_preview = on;
        self.update_notify_settings(settings, cx);
    }

    fn update_notify_settings(&mut self, settings: NotifySettings, cx: &mut Context<Self>) {
        self.notify.set_settings(settings.clone());
        self.persist_notify_settings(settings);
        cx.notify();
    }

    /// Clear everything that belongs to the project that was just left.
    fn reset_project_scoped(&mut self) {
        self.sessions = SessionStore::default();
        self.transcript = TranscriptStore::default();
        self.permissions = PermissionQueue::default();
        self.inbox = InboxStore::default();
        self.forms = FormStore::default();
        self.revert = RevertStore::default();
        self.context = ContextStore::default();
        self.usage = UsageStore::default();
        self.schedules = ScheduleStore::default();
        self.schedules_session = None;
        self.show_finished_schedules = false;
        self.schedule_form_open = false;
        self.pending_schedule_delete = None;
        self.renaming = None;
        self.pending_delete = None;
        self.permission_in_flight = None;
        self.vcs = VcsStore::default();
        self.diff_files.clear();
        self.diff = ParsedDiff::default();
        self.diff_rows.clear();
        self.selected_file = None;
        self.error = None;
    }

    /// Reload the server-backed lists that are scoped to a directory.
    fn reload_project_scoped(&mut self) {
        self.request_vcs_status();
        self.engine.load_usage_limits(self.directory(), 1);
        self.engine.load_plugins(self.directory(), 1);
        self.engine.load_mcp(self.directory(), 1);
        self.engine.load_providers(self.directory(), 1);
        self.engine.load_integrations(self.directory(), 1);
        for (request_id, kind) in [
            (1, CatalogKind::Commands),
            (2, CatalogKind::Skills),
            (3, CatalogKind::Agents),
            (4, CatalogKind::Models),
        ] {
            self.engine.list_catalog(self.directory(), request_id, kind);
        }
    }

    /// Point the transcript at the selected session and reload it.
    fn follow_selection(&mut self) {
        match self.sessions.selected_id().map(str::to_owned) {
            Some(id) => {
                self.transcript.begin(&id);
                self.engine.load_transcript(self.directory(), id.clone());
                self.permissions.begin(&id);
                self.engine.load_permissions(self.directory(), id.clone());
                if self.schedules_session.as_deref() == Some(id.as_str()) {
                    // The stream may have dropped schedule events; resnapshot.
                    self.load_schedules();
                } else {
                    self.reset_schedules(&id);
                }
            }
            None => {
                self.transcript = TranscriptStore::default();
                self.permissions = PermissionQueue::default();
                self.inbox = InboxStore::default();
                self.permission_in_flight = None;
                self.schedules = ScheduleStore::default();
                self.schedules_session = None;
                self.pending_schedule_delete = None;
                self.schedule_form_open = false;
            }
        }
    }

    /// Close the connect dialog when it belongs to `integration_id`.
    fn close_provider_dialog(&mut self, integration_id: &str) {
        if self
            .provider_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.integration_id == integration_id)
        {
            self.provider_dialog = None;
        }
    }

    /// Pick a method inside the open dialog. OAuth starts immediately; the
    /// other flows wait for an explicit action.
    fn choose_provider_method(&mut self, method: IntegrationMethod) {
        let Some(dialog) = self.provider_dialog.as_mut() else {
            return;
        };
        let integration_id = dialog.integration_id.clone();
        let is_oauth = matches!(method, IntegrationMethod::OAuth { .. });
        let method_id = method.method_id().map(str::to_string);
        dialog.method = Some(method);
        if is_oauth && let Some(method_id) = method_id {
            self.engine
                .start_oauth(self.directory(), integration_id, method_id);
        }
    }

    /// Render the open connect dialog for one integration.
    fn render_provider_dialog(
        &self,
        dialog: &ProviderDialog,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let integration_id = dialog.integration_id.clone();
        let flow = self.providers.flow(&integration_id);
        let integration = self
            .providers
            .integrations()
            .iter()
            .find(|integration| integration.id == integration_id)
            .cloned();
        let mut card = div()
            .mx_3()
            .mb_2()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().child(format!("Connect {}", dialog.integration_name)))
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "provider-close-{integration_id}"
                            )))
                            .text_color(theme.muted)
                            .child("Close")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.provider_dialog = None;
                                cx.notify();
                            })),
                    ),
            );

        if let AuthFlow::Failed { message } = &flow {
            card = card.child(div().text_color(theme.danger).child(message.clone()));
        }

        match &dialog.method {
            None => {
                card = card.child(
                    div()
                        .text_color(theme.muted)
                        .child("How do you want to connect?"),
                );
                let methods = integration
                    .as_ref()
                    .map(|integration| integration.methods.clone())
                    .unwrap_or_default();
                if methods.is_empty() {
                    card = card.child(
                        div()
                            .text_color(theme.muted)
                            .child("This integration exposes no connect method."),
                    );
                }
                for (index, method) in methods.into_iter().enumerate() {
                    let enabled = method.is_connectable();
                    let label = method.label();
                    let detail = if method.kind() == "env" {
                        format!(" — set {}", method.env_names().join(", "))
                    } else {
                        String::new()
                    };
                    let chosen = method.clone();
                    card =
                        card.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_color(theme.muted)
                                        .child(format!("{label}{detail}")),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "provider-method-{integration_id}-{index}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label("Use")
                                    .disabled(!enabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.choose_provider_method(chosen.clone());
                                        cx.notify();
                                    })),
                                ),
                        );
                }
            }
            Some(IntegrationMethod::OAuth { .. }) => {
                let attempt = flow.attempt().cloned();
                let link = attempt
                    .as_ref()
                    .and_then(|attempt| attempt.url.as_deref())
                    .filter(|url| url.starts_with("https://"))
                    .and_then(|url| url_host(url).map(|host| (url.to_string(), host)));
                if let Some(attempt) = &attempt {
                    if !attempt.instructions.is_empty() {
                        card = card.child(
                            div()
                                .text_color(theme.muted)
                                .child(attempt.instructions.clone()),
                        );
                    }
                    if let Some((_, host)) = &link {
                        card = card.child(
                            div()
                                .text_color(theme.muted)
                                .child(format!("Sign-in page: {host}")),
                        );
                    }
                } else {
                    card = card.child(
                        div()
                            .text_color(theme.muted)
                            .child("Waiting for the server to start the sign-in…"),
                    );
                }
                let mut buttons = div().flex().gap_2();
                if let Some((url, host)) = link {
                    buttons = buttons.child(
                        Button::new(SharedString::from(format!(
                            "provider-open-{integration_id}"
                        )))
                        .small()
                        .primary()
                        .label(format!("Open browser ({host})"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.open_url(&url);
                        })),
                    );
                }
                if let Some(attempt) = &attempt {
                    let attempt_id = attempt.attempt_id.clone();
                    let id = integration_id.clone();
                    buttons = buttons.child(
                        Button::new(SharedString::from(format!(
                            "provider-check-{integration_id}"
                        )))
                        .small()
                        .ghost()
                        .label("Check now")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.engine.poll_oauth(
                                this.directory(),
                                id.clone(),
                                attempt_id.clone(),
                            );
                            cx.notify();
                        })),
                    );
                    let attempt_id = attempt.attempt_id.clone();
                    let id = integration_id.clone();
                    buttons = buttons.child(
                        Button::new(SharedString::from(format!(
                            "provider-cancel-{integration_id}"
                        )))
                        .small()
                        .danger()
                        .label("Cancel")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.engine.cancel_oauth(
                                this.directory(),
                                id.clone(),
                                attempt_id.clone(),
                            );
                            cx.notify();
                        })),
                    );
                }
                card = card.child(buttons);
            }
            Some(IntegrationMethod::Command {
                id: method_id,
                command,
                ..
            }) => {
                let line = command.join(" ");
                card = card.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.surface)
                        .child(line.clone()),
                );
                card = card.child(
                    div()
                        .text_color(theme.muted)
                        .child("Run this command yourself. Rust Harness never executes it."),
                );
                if flow.is_active() {
                    let attempt_id = flow
                        .attempt()
                        .map(|attempt| attempt.attempt_id.clone())
                        .unwrap_or_default();
                    let id = integration_id.clone();
                    card =
                        card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_color(theme.muted)
                                        .child("Waiting for the command to finish…"),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "provider-cancel-command-{integration_id}"
                                    )))
                                    .small()
                                    .danger()
                                    .label("Cancel")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.engine.cancel_command_auth(
                                            this.directory(),
                                            id.clone(),
                                            attempt_id.clone(),
                                        );
                                        cx.notify();
                                    })),
                                ),
                        );
                } else {
                    let copy_line = line.clone();
                    let id = integration_id.clone();
                    let method_id = method_id.clone();
                    let start_line = line.clone();
                    card =
                        card.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "provider-copy-command-{integration_id}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label("Copy command")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_line.clone(),
                                        ));
                                    })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "provider-start-command-{integration_id}"
                                    )))
                                    .small()
                                    .primary()
                                    .label("I ran this command")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.engine.start_command_auth(
                                            this.directory(),
                                            id.clone(),
                                            method_id.clone(),
                                            start_line.clone(),
                                        );
                                        cx.notify();
                                    })),
                                ),
                        );
                }
            }
            Some(IntegrationMethod::Key { .. }) => {
                card = card.child(Input::new(&self.provider_key).w_full());
                let has_key = !self.provider_key.read(cx).value().trim().is_empty();
                let id = integration_id.clone();
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!(
                                "provider-send-key-{integration_id}"
                            )))
                            .small()
                            .primary()
                            .label("Send key")
                            .disabled(!has_key)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let mut key =
                                    this.provider_key.read(cx).value().trim().to_string();
                                if key.is_empty() {
                                    return;
                                }
                                this.engine.send_key(this.directory(), id.clone(), key.as_str());
                                key.clear();
                                this.provider_key.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                                cx.notify();
                            })),
                        )
                        .child(div().text_color(theme.muted).child(
                            "The key goes only to the local OpenCode server and is cleared after sending.",
                        )),
                );
            }
            Some(IntegrationMethod::External { id: method_id, .. }) => {
                card = card.child(div().text_color(theme.muted).child(
                    "Credentials are managed outside Rust Harness. The server opens the provider page when it needs one.",
                ));
                let id = integration_id.clone();
                let method_id = method_id.clone();
                card = card.child(
                    Button::new(SharedString::from(format!(
                        "provider-external-complete-{integration_id}"
                    )))
                    .small()
                    .primary()
                    .label("Complete connection")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.engine.connect_external(
                            this.directory(),
                            id.clone(),
                            method_id.clone(),
                        );
                        cx.notify();
                    })),
                );
            }
            Some(IntegrationMethod::Env { names }) => {
                card = card.child(div().text_color(theme.muted).child(format!(
                    "Set {} in the environment of the sidecar, then restart it.",
                    names.join(", ")
                )));
            }
            Some(IntegrationMethod::Unknown { kind, .. }) => {
                card = card.child(
                    div()
                        .text_color(theme.muted)
                        .child(format!("This app cannot perform the {kind} method.")),
                );
            }
        }

        card
    }

    /// Point the schedules panel at a session and load its server list.
    fn reset_schedules(&mut self, session_id: &str) {
        self.schedules = ScheduleStore::default();
        self.schedules_session = Some(session_id.to_string());
        self.pending_schedule_delete = None;
        self.schedule_form_open = false;
        self.load_schedules();
    }

    fn load_schedules(&mut self) {
        let Some(session_id) = self.schedules_session.clone() else {
            return;
        };
        self.schedule_request += 1;
        self.engine
            .load_schedules(self.directory(), self.schedule_request, session_id);
    }

    /// Validate the form locally, then let the server create the schedule.
    fn save_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.schedules_session.clone() else {
            self.error = Some("Open a session before adding a schedule.".into());
            cx.notify();
            return;
        };
        let prompt = self.schedule_prompt.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            self.error = Some("A scheduled prompt cannot be empty.".into());
            cx.notify();
            return;
        }
        let value = self.schedule_when.read(cx).value().trim().to_string();
        let when = match self.schedule_kind {
            0 => {
                if let Err(message) = validate_duration(&value) {
                    self.error = Some(message);
                    cx.notify();
                    return;
                }
                ScheduleWhen::in_(value)
            }
            1 => {
                if value.is_empty() {
                    self.error = Some("A date and time are required.".into());
                    cx.notify();
                    return;
                }
                ScheduleWhen::at(value)
            }
            2 => {
                if let Err(message) = validate_duration(&value) {
                    self.error = Some(message);
                    cx.notify();
                    return;
                }
                ScheduleWhen::every(value)
            }
            _ => {
                if value.is_empty() {
                    self.error = Some("A cron expression is required.".into());
                    cx.notify();
                    return;
                }
                ScheduleWhen::cron(value)
            }
        };
        let raw_ends = self.schedule_ends.read(cx).value().trim().to_string();
        let ends = if raw_ends.is_empty() {
            None
        } else {
            match raw_ends.parse::<u32>() {
                Ok(times) if times > 0 => Some(serde_json::json!({ "times": times })),
                _ => {
                    self.error = Some("The run limit must be a positive whole number.".into());
                    cx.notify();
                    return;
                }
            }
        };
        let name = prompt.chars().take(40).collect::<String>();
        self.schedule_request += 1;
        self.engine.create_schedule(
            self.directory(),
            self.schedule_request,
            CreateScheduleRequest {
                session_id,
                name: Some(name),
                when,
                if_condition: None,
                until: None,
                ends,
                prompt,
            },
        );
        self.schedule_form_open = false;
        self.schedule_prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.schedule_when
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.schedule_ends
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.error = None;
        cx.notify();
    }

    fn schedule_kind_label(&self) -> &'static str {
        match self.schedule_kind {
            0 => "In",
            1 => "At",
            2 => "Every",
            _ => "Cron",
        }
    }

    fn schedule_kind_hint(&self) -> &'static str {
        match self.schedule_kind {
            0 => "duration, e.g. 5 minutes",
            1 => "date, e.g. 2030-01-01T09:00:00Z",
            2 => "interval, e.g. 1 hour",
            _ => "expression, e.g. 0 * * * *",
        }
    }

    fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_current_draft();
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        self.clear_composer(window, cx);
        if !self.engine.create_session(self.directory()) {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn select_session(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.sessions.selected_id() == Some(id.as_str()) {
            return;
        }
        self.save_current_draft();
        self.sessions.select(&id);
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        match self.active_project_key() {
            Some(project) => self.restore_draft(&project, &id, window, cx),
            None => self.clear_composer(window, cx),
        }
        self.follow_selection();
        cx.notify();
    }

    fn active_project_key(&self) -> Option<String> {
        self.projects.active_id().map(|id| id.as_str().to_owned())
    }

    /// Remember the composer under the active project and session before the
    /// selection changes.
    fn save_current_draft(&mut self) {
        let Some(project) = self.active_project_key() else {
            return;
        };
        let session = self.sessions.selected_id().unwrap_or_default().to_owned();
        self.drafts.set(&project, &session, self.draft.clone());
        self.drafts.set(&project, "", self.draft.clone());
    }

    /// Load a draft when one was saved for this project/session, falling back
    /// to the project's last used draft.
    fn restore_draft(
        &mut self,
        project: &str,
        session: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.draft = self
            .drafts
            .get(project, session)
            .or_else(|| self.drafts.get(project, ""))
            .cloned()
            .unwrap_or_default();
        let text = self.draft.text().to_owned();
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
    }

    /// The canonical directory every engine command must name.
    fn directory(&self) -> String {
        self.project_dir.to_string_lossy().into_owned()
    }

    fn switch_project(&mut self, id: ProjectId, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.active_id() == Some(&id) {
            self.switcher.close();
            cx.notify();
            return;
        }
        self.save_current_draft();
        if !self.projects.select(&id) {
            self.error = Some("That project is no longer open.".into());
            cx.notify();
            return;
        }
        let Some(canonical) = self.projects.active_canonical().map(str::to_owned) else {
            return;
        };
        self.project_dir = PathBuf::from(canonical);
        self.reset_project_scoped();
        self.switcher.close();
        self.persist_projects();
        self.restore_draft(id.as_str(), "", window, cx);
        if !self.engine.select_project(self.directory(), id) {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn add_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(SharedString::from("Add project folder")),
        });
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let picked = receiver.await;
            if this
                .update(cx, |this, cx| {
                    match picked {
                        Ok(Ok(Some(paths))) => {
                            for path in paths {
                                this.add_project_path(path);
                            }
                            this.switcher.open();
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            this.error = Some(format!("Could not open the folder picker: {error}"));
                        }
                        Err(_) => {
                            this.error = Some("The folder picker closed unexpectedly.".into());
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                // The window closed while the picker was open.
            }
        })
        .detach();
    }

    fn add_project_path(&mut self, path: PathBuf) {
        match std::fs::canonicalize(&path) {
            Ok(canonical) if canonical.is_dir() => {
                self.projects.add(canonical.to_string_lossy().into_owned());
                self.error = None;
                self.persist_projects();
            }
            Ok(_) => {
                self.error = Some(format!("{} is not a directory.", path.display()));
            }
            Err(error) => {
                self.error = Some(format!("Could not open {}: {error}", path.display()));
            }
        }
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
        if !self.engine.rename_session(self.directory(), id, title) {
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
        if !self.engine.delete_session(self.directory(), &id) {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn begin_export(&mut self, cx: &mut Context<Self>) {
        if self.sessions.selected().is_none() {
            return;
        }
        self.transfer.export_open = true;
        self.transfer.export_notice = None;
        self.transfer.exporting = false;
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        cx.notify();
    }

    fn cancel_export(&mut self, cx: &mut Context<Self>) {
        self.transfer.export_open = false;
        self.transfer.export_notice = None;
        cx.notify();
    }

    fn toggle_sanitize(&mut self, cx: &mut Context<Self>) {
        self.transfer.sanitize = !self.transfer.sanitize;
        cx.notify();
    }

    fn choose_export_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            return;
        };
        let suggested = format!("session-{id}.json");
        let receiver = cx.prompt_for_new_path(&self.project_dir, Some(&suggested));
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let picked = receiver.await;
            if this
                .update(cx, |this, cx| {
                    match picked {
                        Ok(Ok(Some(path))) => {
                            if this.engine.export_session(
                                this.directory(),
                                id,
                                this.transfer.sanitize,
                                path,
                            ) {
                                this.transfer.exporting = true;
                            } else {
                                this.error = Some("The engine is not running.".into());
                            }
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            this.error = Some(format!("Could not open the save dialog: {error}"));
                        }
                        Err(_) => {
                            this.error = Some("The save dialog closed unexpectedly.".into());
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                // The window closed while the save dialog was open.
            }
        })
        .detach();
    }

    fn begin_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Import session export")),
        });
        self.transfer.import_preview = None;
        self.transfer.import_as_copy = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let picked = receiver.await;
            if this
                .update(cx, |this, cx| {
                    match picked {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.into_iter().next()
                                && !this.engine.preview_import(this.directory(), path)
                            {
                                this.error = Some("The engine is not running.".into());
                            }
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            this.error = Some(format!("Could not open the file dialog: {error}"));
                        }
                        Err(_) => {
                            this.error = Some("The file dialog closed unexpectedly.".into());
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                // The window closed while the file dialog was open.
            }
        })
        .detach();
    }

    fn confirm_import(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.transfer.import_preview.as_ref() else {
            return;
        };
        let path = file.path.clone();
        self.transfer.importing = true;
        self.error = None;
        if !self
            .engine
            .import_session(self.directory(), path, self.transfer.import_as_copy)
        {
            self.transfer.importing = false;
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn toggle_import_copy(&mut self, cx: &mut Context<Self>) {
        self.transfer.import_as_copy = !self.transfer.import_as_copy;
        cx.notify();
    }

    fn cancel_import(&mut self, cx: &mut Context<Self>) {
        self.transfer.import_preview = None;
        self.transfer.importing = false;
        cx.notify();
    }

    fn open_stats(&mut self, cx: &mut Context<Self>) {
        self.transfer.stats_open = true;
        self.load_stats(cx);
    }

    fn close_stats(&mut self, cx: &mut Context<Self>) {
        self.transfer.stats_open = false;
        self.transfer.stats_loading = false;
        cx.notify();
    }

    fn choose_stats_preset(&mut self, preset: StatsPreset, cx: &mut Context<Self>) {
        self.transfer.stats_preset = preset;
        self.load_stats(cx);
    }

    fn load_stats(&mut self, cx: &mut Context<Self>) {
        let range = self.transfer.range(now_ms());
        self.stats_request = self.stats_request.wrapping_add(1);
        self.transfer.stats_loading = true;
        self.error = None;
        if !self
            .engine
            .load_stats(self.directory(), self.stats_request, range)
        {
            self.transfer.stats_loading = false;
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
    fn apply_session_usage(&mut self, session: &Session) {
        if let Some(tokens) = session.tokens {
            self.context.set_usage(tokens, session.cost);
        }
    }

    fn revert_from(&mut self, message_id: String) {
        let Some(session_id) = self.sessions.selected_id().map(str::to_owned) else {
            self.error = Some("Select a session first.".into());
            return;
        };
        if !self
            .engine
            .stage_revert(self.directory(), &session_id, message_id, Some(false))
        {
            self.error = Some("The engine is not running.".into());
        }
    }

    fn fork_from(&mut self, message_id: String) {
        let Some(session_id) = self.sessions.selected_id().map(str::to_owned) else {
            self.error = Some("Select a session first.".into());
            return;
        };
        if !self
            .engine
            .fork_session(self.directory(), &session_id, Some(message_id))
        {
            self.error = Some("The engine is not running.".into());
        }
    }

    fn request_vcs_status(&mut self) {
        self.vcs.begin_refresh(now_ms());
        self.vcs_request += 1;
        self.engine
            .load_vcs_status(self.directory(), self.vcs_request);
        self.engine
            .load_vcs_base(self.directory(), self.vcs_request);
    }

    fn on_filesystem_changed(&mut self) {
        let now = now_ms();
        if now.saturating_sub(self.vcs.refreshed_at()) < 250 {
            return;
        }
        self.request_vcs_status();
    }

    fn request_vcs_diff(&mut self) {
        self.vcs_request += 1;
        self.engine.load_vcs_diff(
            self.directory(),
            self.vcs_request,
            "working",
            None,
            None,
            "files",
        );
    }

    fn rebuild_diff(&mut self) {
        let mut all = ParsedDiff::default();
        for (index, diff) in self.diff_files.iter().enumerate() {
            if self.selected_file.is_some_and(|selected| selected != index) {
                continue;
            }
            let one = parse_patch(diff);
            all.truncated |= one.truncated;
            all.binary |= one.binary;
            all.files.extend(one.files);
        }
        self.diff_rows = flatten(&all, &[]);
        self.diff = all;
    }

    fn select_file(&mut self, name: &str) {
        let index = self.diff_files.iter().position(|diff| diff.file == name);
        self.selected_file = match (index, self.selected_file) {
            (Some(index), Some(selected)) if index == selected => None,
            (Some(index), _) => Some(index),
            (None, _) => None,
        };
        self.diff_scroll = ScrollHandle::new();
        self.rebuild_diff();
    }

    fn cancel_overlays(&mut self, cx: &mut Context<Self>) {
        let had_overlay = self.renaming.is_some()
            || self.pending_delete.is_some()
            || self.error.is_some()
            || self.picker.is_some()
            || self.switcher.is_open()
            || self.transfer.export_open
            || self.transfer.import_preview.is_some()
            || self.transfer.stats_open;
        self.renaming = None;
        self.pending_delete = None;
        self.error = None;
        self.switcher.close();
        self.transfer.export_open = false;
        self.transfer.export_notice = None;
        self.transfer.import_preview = None;
        self.transfer.stats_open = false;
        self.close_picker();
        if had_overlay {
            cx.notify();
        }
    }

    fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let (text, cursor, collapsed) = {
            let input = self.composer.read(cx);
            let range = input.selected_range();
            (input.value().to_string(), range.end, range.is_empty())
        };
        self.draft.edit(text, cursor, collapsed);
        self.sync_trigger_picker(cx);
    }

    /// Open, update, or close the picker for the draft's current trigger.
    fn sync_trigger_picker(&mut self, cx: &mut Context<Self>) {
        let Some(trigger) = self.draft.trigger().cloned() else {
            self.close_picker();
            return;
        };
        let kind = PickerKind::for_trigger(trigger.kind);
        self.ensure_catalog(kind);
        let items = self.catalog.items(kind);
        let effect = match self.picker.as_mut() {
            Some(picker) if picker.kind() == kind => picker.update_query(trigger.query),
            _ => {
                let (picker, effect) = PickerState::open(kind, trigger.query, items);
                self.picker = Some(picker);
                effect
            }
        };
        self.schedule_picker_effect(effect, cx);
        cx.notify();
    }

    fn schedule_picker_effect(&mut self, effect: PickerEffect, cx: &mut Context<Self>) {
        let PickerEffect::FetchFiles { request_id, query } = effect else {
            return;
        };
        self.file_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, _| {
                this.engine
                    .find_files(this.directory(), request_id, query, 24);
            });
        }));
    }

    fn ensure_catalog(&self, kind: PickerKind) {
        let (request_id, catalog) = match kind {
            PickerKind::Command => (1, CatalogKind::Commands),
            PickerKind::Skill => (2, CatalogKind::Skills),
            PickerKind::File | PickerKind::Agent => (3, CatalogKind::Agents),
            PickerKind::Model => (4, CatalogKind::Models),
            PickerKind::Snippet => return,
        };
        let empty = match kind {
            PickerKind::Command => self.catalog.commands.is_empty(),
            PickerKind::Skill => self.catalog.skills.is_empty(),
            PickerKind::File | PickerKind::Agent => self.catalog.agents.is_empty(),
            PickerKind::Model => self.catalog.models.is_empty(),
            PickerKind::Snippet => false,
        };
        if empty {
            self.engine
                .list_catalog(self.directory(), request_id, catalog);
        }
    }

    fn choose_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .picker
            .as_ref()
            .and_then(PickerState::selected)
            .cloned()
        else {
            return;
        };
        self.choose_item(item, window, cx);
    }

    fn choose_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.visible().nth(index))
            .cloned()
        else {
            return;
        };
        self.choose_item(item, window, cx);
    }

    fn choose_item(&mut self, item: PickerItem, window: &mut Window, cx: &mut Context<Self>) {
        let picker_kind = self.picker.as_ref().map(PickerState::kind);
        match item {
            PickerItem::File { path } => {
                let uri = path_to_file_uri(&self.project_dir.join(&path).to_string_lossy());
                if let Some(edit) = self.draft.select_file(path, uri) {
                    self.apply_draft_edit(edit, window, cx);
                }
            }
            PickerItem::Agent { id, .. } if picker_kind == Some(PickerKind::Agent) => {
                let Some(session_id) = self.sessions.selected_id().map(str::to_owned) else {
                    self.error = Some("Select a session first.".into());
                    self.close_picker();
                    cx.notify();
                    return;
                };
                self.sessions.set_agent(&session_id, id.clone());
                self.error = None;
                self.close_picker();
                if !self
                    .engine
                    .set_session_agent(self.directory(), session_id, id)
                {
                    self.error = Some("The engine is not running.".into());
                }
                self.focus_composer(window, cx);
            }
            PickerItem::Agent { name, .. } => {
                if let Some(edit) = self.draft.select_agent(name) {
                    self.apply_draft_edit(edit, window, cx);
                }
            }
            PickerItem::Command { name, .. } => {
                if let Some(edit) = self.draft.select_command(name) {
                    self.apply_draft_edit(edit, window, cx);
                }
            }
            PickerItem::Skill { id, name, .. } => {
                if let Some(edit) = self.draft.select_skill(id, name) {
                    self.apply_draft_edit(edit, window, cx);
                }
            }
            PickerItem::Snippet { expansion, .. } => {
                if let Some(edit) = self.draft.select_snippet(&expansion) {
                    self.apply_draft_edit(edit, window, cx);
                }
            }
            PickerItem::Model { model, .. } => {
                let Some(session_id) = self.sessions.selected_id().map(str::to_owned) else {
                    self.error = Some("Select a session first.".into());
                    self.close_picker();
                    cx.notify();
                    return;
                };
                self.sessions.set_model(&session_id, model.clone());
                self.error = None;
                self.close_picker();
                if !self
                    .engine
                    .set_session_model(self.directory(), session_id, model)
                {
                    self.error = Some("The engine is not running.".into());
                }
                self.focus_composer(window, cx);
            }
        }
        cx.notify();
    }

    fn apply_draft_edit(
        &mut self,
        edit: harness_core::DraftEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |input, cx| {
            input.set_value(edit.text.clone(), window, cx);
            input.set_selected_range(edit.cursor..edit.cursor, cx);
            input.focus(window, cx);
        });
        // `set_value` does not emit Change; resync from the same edit.
        self.draft.edit(edit.text, edit.cursor, true);
        self.close_picker();
    }

    fn remove_file_chip(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.draft.remove_file(index) {
            self.apply_draft_edit(edit, window, cx);
        }
        cx.notify();
    }

    fn close_picker(&mut self) {
        self.picker = None;
        self.picker_input = None;
        self._picker_input_subscription = None;
        self.file_debounce = None;
    }

    /// Handle the keys the composer intercepts while a picker is open.
    ///
    /// Attached in the capture phase on the whole composer so it also fires
    /// when the model/agent search input holds focus. Enter is left to the
    /// textarea and search-input subscriptions.
    fn picker_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.picker.is_none() {
            return false;
        }
        match key {
            "escape" => {
                self.close_picker();
                self.focus_composer(window, cx);
            }
            "up" | "down" => {
                let Some(picker) = self.picker.as_mut() else {
                    return false;
                };
                picker.move_highlight(if key == "up" { -1 } else { 1 });
            }
            "tab" => {
                if !self
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.visible_len() > 0)
                {
                    return false;
                }
                self.choose_selected(window, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn open_session_picker(
        &mut self,
        kind: PickerKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_catalog(kind);
        let (picker, _) = PickerState::open(kind, "", self.catalog.items(kind));
        self.picker = Some(picker);
        self.file_debounce = None;

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search…"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    if let Some(picker) = this.picker.as_mut() {
                        picker.update_query(query);
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => {
                    this.choose_selected(window, cx);
                }
                _ => {}
            },
        );
        self.picker_input = Some(input);
        self._picker_input_subscription = Some(subscription);
        cx.notify();
    }

    /// Focus the composer, cancelling any open picker first.
    fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.draft.clear();
        self.close_picker();
    }

    fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            self.error = Some("Select a session before sending a message.".into());
            cx.notify();
            return;
        };
        let Some(send) = self.draft.to_send() else {
            return;
        };

        self.error = None;
        let accepted = match &send {
            DraftSend::Prompt(request) => {
                let mut request = request.clone();
                if self.transcript.is_busy() {
                    request.delivery = Some(InboxDelivery::Queue);
                }
                self.engine.send_prompt(self.directory(), id, request)
            }
            DraftSend::Command(request) => {
                self.engine
                    .run_command(self.directory(), id, request.clone())
            }
        };
        if accepted {
            self.clear_composer(window, cx);
        } else {
            self.error = Some("The engine is not running.".into());
        }
        cx.notify();
    }

    fn stop_turn(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sessions.selected_id().map(str::to_owned) else {
            return;
        };
        if !self.engine.interrupt(self.directory(), id) {
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
            .reply_permission(self.directory(), session_id, request_id.clone(), reply)
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

    fn form_answer_changed(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.front() else {
            return;
        };
        let form_id = form.id.clone();
        let Some(key) = self.form_key.clone() else {
            return;
        };
        let Some(kind) = form
            .fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.kind.clone())
        else {
            return;
        };
        let value = self.form_input.read(cx).value().to_string();
        let json = match kind.as_str() {
            "number" | "integer" => value
                .trim()
                .parse::<f64>()
                .map(|number| serde_json::json!(number))
                .unwrap_or(serde_json::Value::Null),
            _ => serde_json::Value::String(value),
        };
        self.forms.set_answer(&form_id, &key, json);
    }

    fn render_form(
        &self,
        theme: Theme,
        form: &harness_protocol::FormInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let form_id = form.id.clone();
        let ready = self.forms.can_reply(&form_id);
        let mut card = div()
            .id("form-card")
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(form.title.clone()),
            );

        for field in &form.fields {
            if field.hidden {
                continue;
            }
            let key = field.key.clone();
            let label = field.title.clone().unwrap_or_else(|| field.key.clone());
            let current = self
                .forms
                .value(&form_id, &key)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match field.kind.as_str() {
                "boolean" => {
                    let on = current.as_bool().unwrap_or(false);
                    let form = form_id.clone();
                    let key_bool = key.clone();
                    card = card.child(
                        div()
                            .id(SharedString::from(format!("form-bool-{key}")))
                            .text_xs()
                            .child(format!("{label}: {}", if on { "yes" } else { "no" }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.forms
                                    .set_answer(&form, &key_bool, serde_json::json!(!on));
                                cx.notify();
                            })),
                    );
                }
                "multiselect" => {
                    let chosen: Vec<String> = current
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut row = div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .text_xs()
                        .child(format!("{label}:"));
                    for option in &field.options {
                        let Some(value) = option.as_str() else {
                            continue;
                        };
                        let selected = chosen.iter().any(|chosen| chosen == value);
                        let mut next = chosen.clone();
                        if selected {
                            next.retain(|chosen| chosen != value);
                        } else {
                            next.push(value.to_string());
                        }
                        let form = form_id.clone();
                        let value = value.to_string();
                        let option_key = key.clone();
                        row = row.child(
                            div()
                                .id(SharedString::from(format!("form-opt-{key}-{value}")))
                                .px_2()
                                .bg(if selected { theme.accent } else { theme.border })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let json = serde_json::json!(next.clone());
                                    this.forms.set_answer(&form, &option_key, json);
                                    cx.notify();
                                }))
                                .child(value),
                        );
                    }
                    card = card.child(row);
                }
                "external" => {
                    card = card.child(div().text_xs().text_color(theme.muted).child(label));
                }
                _ => {
                    let value = current.as_str().unwrap_or("").to_string();
                    card = card.child(
                        div()
                            .id(SharedString::from(format!("form-text-{key}")))
                            .text_xs()
                            .child(format!(
                                "{label}: {}",
                                if value.is_empty() { "—" } else { &value }
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.form_key = Some(key.clone());
                                let current = value.clone();
                                this.form_input.update(cx, |state, cx| {
                                    state.set_value(current, window, cx);
                                });
                                cx.notify();
                            })),
                    );
                }
            }
        }

        if self.form_key.is_some() {
            card = card.child(Input::new(&self.form_input).w_full());
        }

        let form_reply = form_id.clone();
        let form_dismiss = form_id.clone();
        card.child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .id("form-reply")
                        .px_2()
                        .py_1()
                        .text_xs()
                        .bg(if ready { theme.accent } else { theme.border })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.forms.can_reply(&form_reply) {
                                return;
                            }
                            let Some(answer) = this.forms.answer(&form_reply) else {
                                return;
                            };
                            let Some(session) = this.sessions.selected_id().map(str::to_owned)
                            else {
                                return;
                            };
                            this.engine.reply_form(
                                this.directory(),
                                session,
                                form_reply.clone(),
                                answer,
                            );
                            cx.notify();
                        }))
                        .child("Reply"),
                )
                .child(
                    div()
                        .id("form-dismiss")
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let Some(session) = this.sessions.selected_id().map(str::to_owned)
                            else {
                                return;
                            };
                            this.engine.dismiss_form(
                                this.directory(),
                                session,
                                form_dismiss.clone(),
                            );
                            cx.notify();
                        }))
                        .child("Dismiss"),
                ),
        )
    }

    fn render_context_meter(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let used = self.context.usage().used();
        let cost = self.context.usage().cost.unwrap_or(0.0);
        let compacting = self.context.compaction() != harness_core::CompactionState::Idle;
        div()
            .id("context-meter")
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(theme.muted)
            .child(if used > 0.0 {
                format!("{used:.0} tokens · ${cost:.2}")
            } else {
                "context unknown".to_string()
            })
            .child(match self.usage.budget_count() {
                0 => String::new(),
                count => format!("· {count} limits"),
            })
            .child(
                div()
                    .id("compact-now")
                    .px_2()
                    .py_1()
                    .bg(if compacting {
                        theme.border
                    } else {
                        theme.surface
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.context.compaction() != harness_core::CompactionState::Idle {
                            return;
                        }
                        if let Some(id) = this.sessions.selected_id().map(str::to_owned) {
                            this.context.begin_compaction();
                            this.engine.compact(this.directory(), id);
                        }
                        cx.notify();
                    }))
                    .child(if compacting {
                        "Compacting…"
                    } else {
                        "Compact"
                    }),
            )
    }

    fn render_inbox(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = div()
            .id("inbox-strip")
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(format!("{} queued", self.inbox.len())),
            );
        for item in self.inbox.items() {
            let inbox_id = item.id.clone();
            strip = strip.child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(div().text_xs().child(if item.text.is_empty() {
                        item.kind.clone()
                    } else {
                        item.text.clone()
                    }))
                    .child(
                        div()
                            .id(SharedString::from(format!("inbox-cancel-{inbox_id}")))
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.danger))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(session) =
                                    this.sessions.selected_id().map(str::to_owned)
                                {
                                    this.engine.cancel_inbox(
                                        this.directory(),
                                        session,
                                        inbox_id.clone(),
                                    );
                                }
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            );
        }
        strip
    }

    fn render_changes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let branch = self.vcs.branch().unwrap_or("no branch").to_string();
        let mut panel = div()
            .debug_selector(|| "changes-panel".into())
            .w(px(340.0))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child("Changes"))
                    .child(div().text_xs().text_color(theme.muted).child(branch)),
            );

        if let Some(error) = self.vcs.error() {
            panel = panel.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(error.to_string()),
            );
        }

        let mut list = div()
            .id("changes-list")
            .flex()
            .flex_col()
            .max_h(px(220.0))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(theme.border);
        if self.vcs.files().is_empty() {
            let label = if self.vcs.is_loading() {
                "Loading…"
            } else {
                "No changes"
            };
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(label),
            );
        } else {
            for file in self.vcs.files() {
                let name = file.file.clone();
                let selected = self
                    .selected_file
                    .and_then(|index| self.diff_files.get(index))
                    .is_some_and(|diff| diff.file == name);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("changes-file-{name}")))
                        .px_3()
                        .py_1()
                        .flex()
                        .justify_between()
                        .bg(if selected {
                            theme.border
                        } else {
                            theme.surface
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_file(&name);
                            cx.notify();
                        }))
                        .child(div().text_xs().child(file.file.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(format!("+{} -{}", file.additions, file.deletions)),
                        ),
                );
            }
        }
        panel = panel.child(list);

        if self.schedules_session.is_some() {
            let mut section = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_between()
                        .child(div().text_sm().child("Schedules"))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(if self.schedules.has_finished() {
                                    div()
                                        .id("schedule-show-finished")
                                        .text_xs()
                                        .text_color(theme.muted)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_finished_schedules =
                                                !this.show_finished_schedules;
                                            cx.notify();
                                        }))
                                        .child(if self.show_finished_schedules {
                                            "Hide finished"
                                        } else {
                                            "Show finished"
                                        })
                                } else {
                                    div().id("schedule-show-finished-placeholder")
                                })
                                .child(
                                    div()
                                        .id("schedule-add-toggle")
                                        .text_xs()
                                        .text_color(theme.accent)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.schedule_form_open = !this.schedule_form_open;
                                            cx.notify();
                                        }))
                                        .child(if self.schedule_form_open {
                                            "Cancel"
                                        } else {
                                            "Add"
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(theme.warning)
                        .child("Schedules only run while Rust Harness is open."),
                );

            if self.schedule_form_open {
                section = section
                    .child(Input::new(&self.schedule_prompt).w_full())
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .child(
                                div()
                                    .id("schedule-kind")
                                    .text_color(theme.accent)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.schedule_kind = (this.schedule_kind + 1) % 4;
                                        cx.notify();
                                    }))
                                    .child(format!("When: {}", self.schedule_kind_label())),
                            )
                            .child(
                                div()
                                    .text_color(theme.muted)
                                    .child(self.schedule_kind_hint()),
                            ),
                    )
                    .child(Input::new(&self.schedule_when).w_full())
                    .child(Input::new(&self.schedule_ends).w_full())
                    .child(
                        div()
                            .id("schedule-save")
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(theme.accent)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_schedule(window, cx);
                            }))
                            .child("Save"),
                    );
            }

            let mut listed = false;
            for schedule in self.schedules.visible(self.show_finished_schedules) {
                listed = true;
                let id = schedule.id.clone();
                let cadence = schedule
                    .when
                    .as_ref()
                    .map(describe_when)
                    .unwrap_or_default();
                let next = next_run_label(schedule.next_due_at, now_ms());
                let status = schedule.status.clone();
                let reason = failure_reason(schedule);

                let mut row = div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_col()
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(schedule.name.clone())
                            .child(status.label().to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_color(theme.muted)
                            .child(cadence)
                            .child(next),
                    );
                if let Some(reason) = reason {
                    row = row.child(div().text_color(theme.danger).child(reason));
                }

                let mut actions = div().flex().gap_2().justify_end();
                let session_id = self.schedules_session.clone().unwrap_or_default();
                if status.is_active() {
                    let pause_id = id.clone();
                    actions = actions.child(
                        div()
                            .id(SharedString::from(format!("schedule-pause-{id}")))
                            .text_color(theme.muted)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.schedule_request += 1;
                                this.engine.pause_schedule(
                                    this.directory(),
                                    this.schedule_request,
                                    session_id.clone(),
                                    pause_id.clone(),
                                );
                                cx.notify();
                            }))
                            .child("Pause"),
                    );
                } else if matches!(status, ScheduleStatus::Paused) {
                    let resume_id = id.clone();
                    actions = actions.child(
                        div()
                            .id(SharedString::from(format!("schedule-resume-{id}")))
                            .text_color(theme.muted)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.schedule_request += 1;
                                this.engine.resume_schedule(
                                    this.directory(),
                                    this.schedule_request,
                                    session_id.clone(),
                                    resume_id.clone(),
                                );
                                cx.notify();
                            }))
                            .child("Resume"),
                    );
                }

                let session_id = self.schedules_session.clone().unwrap_or_default();
                let delete_id = id.clone();
                let pending = self.pending_schedule_delete.as_deref() == Some(id.as_str());
                actions = actions.child(
                    div()
                        .id(SharedString::from(format!("schedule-delete-{id}")))
                        .text_color(if pending { theme.danger } else { theme.muted })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.pending_schedule_delete.as_deref() == Some(delete_id.as_str()) {
                                this.schedule_request += 1;
                                this.engine.delete_schedule(
                                    this.directory(),
                                    this.schedule_request,
                                    session_id.clone(),
                                    delete_id.clone(),
                                );
                                this.pending_schedule_delete = None;
                            } else {
                                this.pending_schedule_delete = Some(delete_id.clone());
                            }
                            cx.notify();
                        }))
                        .child(if pending { "Confirm" } else { "Delete" }),
                );
                row = row.child(actions);
                section = section.child(row);
            }
            if !listed && !self.schedule_form_open {
                section = section.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(theme.muted)
                        .child("No schedules for this session."),
                );
            }
            panel = panel.child(section);
        }

        if !self.plugins.is_empty() {
            let mut section = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .child(div().px_3().py_2().text_sm().child("Plugins"));
            for plugin in self.plugins.rows() {
                let id = plugin.id.clone();
                let pending = self.pending_plugin_update.as_deref() == Some(id.as_str());
                let mut row = div()
                    .px_3()
                    .py_1()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .child(plugin.id.clone())
                    .child(plugin.status.clone());
                if plugin.outdated {
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("plugin-update-{id}")))
                            .px_2()
                            .text_color(theme.warning)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.pending_plugin_update.as_deref() == Some(id.as_str()) {
                                    this.engine.update_plugin(this.directory(), id.clone());
                                    this.pending_plugin_update = None;
                                } else {
                                    this.pending_plugin_update = Some(id.clone());
                                }
                                cx.notify();
                            }))
                            .child(if pending { "Confirm" } else { "Update" }),
                    );
                }
                section = section.child(row);
            }
            panel = panel.child(section);
        }

        {
            let mut section = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_between()
                        .child(div().text_sm().child("MCP"))
                        .child(
                            div()
                                .id("mcp-add-toggle")
                                .text_xs()
                                .text_color(theme.muted)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mcp_add_open = !this.mcp_add_open;
                                    cx.notify();
                                }))
                                .child(if self.mcp_add_open { "Cancel" } else { "Add" }),
                        ),
                );
            if self.mcp_add_open {
                section = section
                    .child(Input::new(&self.mcp_name).w_full())
                    .child(Input::new(&self.mcp_config).w_full())
                    .child(
                        div()
                            .id("mcp-add-save")
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(theme.accent)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let name = this.mcp_name.read(cx).value().trim().to_string();
                                let raw = this.mcp_config.read(cx).value().to_string();
                                let config = match serde_json::from_str::<serde_json::Value>(&raw) {
                                    Ok(config) if config.is_object() => config,
                                    _ => {
                                        this.error =
                                            Some("MCP config must be a JSON object.".into());
                                        cx.notify();
                                        return;
                                    }
                                };
                                if name.is_empty() {
                                    this.error = Some("MCP server needs a name.".into());
                                    cx.notify();
                                    return;
                                }
                                this.engine.add_mcp(this.directory(), name, config);
                                this.mcp_add_open = false;
                                cx.notify();
                            }))
                            .child("Save"),
                    );
            }
            section = section.child(
                div()
                    .px_3()
                    .pb_2()
                    .flex()
                    .justify_between()
                    .items_center()
                    .text_xs()
                    .text_color(theme.warning)
                    .child("Runtime only: this may not survive a sidecar restart.")
                    .child(
                        div()
                            .id("mcp-copy-config")
                            .text_color(theme.muted)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let name = this.mcp_name.read(cx).value().trim().to_string();
                                let config = serde_json::from_str::<serde_json::Value>(
                                    &this.mcp_config.read(cx).value(),
                                )
                                .ok()
                                .filter(serde_json::Value::is_object);
                                let (name, config) = match (name.is_empty(), config) {
                                    (false, Some(config)) => (name, config),
                                    _ => (
                                        "server-name".to_string(),
                                        serde_json::json!({
                                            "type": "local",
                                            "command": ["command", "args"]
                                        }),
                                    ),
                                };
                                let mut servers = serde_json::Map::new();
                                servers.insert(name, config);
                                let snippet = serde_json::json!({
                                    "mcp": { "servers": servers }
                                });
                                match serde_json::to_string_pretty(&snippet) {
                                    Ok(snippet) => {
                                        cx.write_to_clipboard(ClipboardItem::new_string(snippet));
                                    }
                                    Err(_) => {
                                        this.error =
                                            Some("Could not build the config snippet.".into());
                                        cx.notify();
                                    }
                                }
                            }))
                            .child("Copy config"),
                    ),
            );
            if self.mcp.is_empty() {
                section = section.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .child("No MCP servers."),
                );
            }
            for server in self.mcp.rows() {
                let name = server.name.clone();
                let pending = self.pending_mcp_remove.as_deref() == Some(name.as_str());
                let color = match &server.status {
                    McpStatus::Connected => theme.success,
                    McpStatus::Pending | McpStatus::NeedsAuth => theme.warning,
                    McpStatus::Failed { .. } => theme.danger,
                    McpStatus::Disabled | McpStatus::Unknown { .. } => theme.muted,
                };

                let mut label = div().flex().flex_col().child(name.clone());
                if let Some(title) = server.title.clone() {
                    label = label.child(div().text_color(theme.muted).child(title));
                }

                let mut actions = div().flex().items_center().gap_2().child(
                    div()
                        .text_color(theme.muted)
                        .child(server.status_label().to_string()),
                );
                if server.can_connect() {
                    let connect_name = name.clone();
                    actions = actions.child(
                        div()
                            .id(SharedString::from(format!("mcp-connect-{name}")))
                            .text_color(theme.accent)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.engine
                                    .connect_mcp(this.directory(), connect_name.clone());
                                cx.notify();
                            }))
                            .child("Connect"),
                    );
                }
                if server.can_disconnect() {
                    let disconnect_name = name.clone();
                    actions = actions.child(
                        div()
                            .id(SharedString::from(format!("mcp-disconnect-{name}")))
                            .text_color(theme.accent)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.engine
                                    .disconnect_mcp(this.directory(), disconnect_name.clone());
                                cx.notify();
                            }))
                            .child("Disconnect"),
                    );
                }
                let remove_name = name.clone();
                actions = actions.child(
                    div()
                        .id(SharedString::from(format!("mcp-remove-{name}")))
                        .text_color(if pending { theme.danger } else { theme.muted })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.pending_mcp_remove.as_deref() == Some(remove_name.as_str()) {
                                this.engine
                                    .remove_mcp(this.directory(), remove_name.clone());
                                this.pending_mcp_remove = None;
                            } else {
                                this.pending_mcp_remove = Some(remove_name.clone());
                            }
                            cx.notify();
                        }))
                        .child(if pending { "Confirm" } else { "Remove" }),
                );

                let mut row = div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(status_dot(color))
                                    .child(label),
                            )
                            .child(actions),
                    );
                if let Some(error) = server.error.clone() {
                    row = row.child(div().text_color(theme.danger).child(error));
                }
                section = section.child(row);
            }
            panel = panel.child(section);
        }

        if !self.saved_permissions.is_empty() {
            let mut section = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .child(div().px_3().py_2().text_sm().child("Permissions"));
            for rule in self.saved_permissions.rules() {
                let id = rule.id.clone();
                let pending = self.pending_saved_permission.as_deref() == Some(id.as_str());
                section = section.child(
                    div()
                        .px_3()
                        .py_1()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .child(format!("{} · {}", rule.action, rule.resource))
                        .child(
                            div()
                                .id(SharedString::from(format!("saved-perm-revoke-{id}")))
                                .text_color(if pending { theme.danger } else { theme.muted })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.pending_saved_permission.as_deref() == Some(id.as_str())
                                    {
                                        this.engine
                                            .delete_saved_permission(this.directory(), id.clone());
                                        this.pending_saved_permission = None;
                                    } else {
                                        this.pending_saved_permission = Some(id.clone());
                                    }
                                    cx.notify();
                                }))
                                .child(if pending { "Confirm" } else { "Revoke" }),
                        ),
                );
            }
            panel = panel.child(section);
        }

        if !self.providers.is_empty() {
            let providers = self.providers.providers().to_vec();
            let integrations = self.providers.integrations().to_vec();
            let mut section = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .child(div().px_3().py_2().text_sm().child("Providers"));
            for provider in providers {
                let package = provider.package.clone().unwrap_or_default();
                let mut row = div()
                    .px_3()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(provider.name.clone())
                            .child(div().text_color(theme.muted).child(package)),
                    );
                if let Some(integration) = provider
                    .integration_id
                    .as_deref()
                    .and_then(|id| integrations.iter().find(|integration| integration.id == id))
                    .cloned()
                {
                    let connected = integration.has_credential();
                    let needs_auth = integration
                        .connections
                        .iter()
                        .any(|connection| connection.needs_auth());
                    let (label, color) = if needs_auth {
                        ("Needs auth", theme.danger)
                    } else if connected {
                        ("Connected", theme.success)
                    } else if integration.has_env() {
                        ("Env only", theme.muted)
                    } else {
                        ("Not connected", theme.muted)
                    };
                    let dialog_id = integration.id.clone();
                    let dialog_name = integration.name.clone();
                    row =
                        row.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_color(color).child(label))
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "provider-connect-{}",
                                        integration.id
                                    )))
                                    .small()
                                    .ghost()
                                    .label(if connected { "Reconnect" } else { "Connect" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.provider_dialog = Some(ProviderDialog::new(
                                            dialog_id.clone(),
                                            dialog_name.clone(),
                                        ));
                                        this.error = None;
                                        cx.notify();
                                    })),
                                ),
                        );
                }
                section = section.child(row);
            }
            if let Some(dialog) = self.provider_dialog.clone() {
                section = section.child(self.render_provider_dialog(&dialog, &theme, cx));
            }
            panel = panel.child(section);
        }

        panel = panel.child(self.render_notify_settings(&theme, cx));

        let offset: f32 = self.diff_scroll.offset().y.into();
        let row_height = 18.0;
        let top = if offset < 0.0 {
            (-offset / row_height) as usize
        } else {
            0
        };
        let visible = visible_range(self.diff_rows.len(), top, 50, 8);

        let mut body = div()
            .id("changes-diff")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.diff_scroll)
            .flex()
            .flex_col()
            .pt_1();
        for row in &self.diff_rows[visible] {
            let element = match row {
                DiffRow::FileHeader { path, .. } => {
                    div().px_2().pt_2().text_xs().child(path.clone())
                }
                DiffRow::HunkHeader { header, .. } => div()
                    .px_2()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(header.clone()),
                DiffRow::Notice { text, .. } => div()
                    .px_2()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(text.clone()),
                DiffRow::Line { file, hunk, line } => {
                    let Some(entry) = self
                        .diff
                        .files
                        .get(*file)
                        .and_then(|file| file.hunks.get(*hunk))
                        .and_then(|hunk| hunk.lines.get(*line))
                    else {
                        continue;
                    };
                    let (prefix, color) = match entry.kind {
                        DiffLineKind::Added => ("+", theme.success),
                        DiffLineKind::Removed => ("-", theme.danger),
                        DiffLineKind::Context => (" ", theme.text),
                        DiffLineKind::Meta => ("\\", theme.muted),
                    };
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(color)
                        .child(format!("{prefix}{}", entry.text))
                }
            };
            body = body.child(element);
        }
        panel.child(body)
    }

    fn render_notify_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.notify.settings();
        let mut section = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .justify_between()
                    .child(div().text_sm().child("Notifications"))
                    .child(
                        div()
                            .id("notify-enabled")
                            .text_xs()
                            .text_color(if settings.enabled {
                                theme.success
                            } else {
                                theme.muted
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                let enabled = !this.notify.settings().enabled;
                                this.set_notify_enabled(enabled, cx);
                            }))
                            .child(if settings.enabled { "On" } else { "Off" }),
                    ),
            );

        if self.notify_denied {
            section = section.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(
                        "Notifications are blocked by the system. \
                         Enable Rust Harness in System Settings.",
                    ),
            );
        }

        for kind in NotifyKind::ALL {
            let on = settings.is_on(kind);
            section = section.child(
                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .child(kind.label())
                    .child(
                        div()
                            .id(SharedString::from(format!("notify-kind-{}", kind.as_str())))
                            .text_color(if on { theme.accent } else { theme.muted })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_notify_kind(kind, !on, cx);
                            }))
                            .child(if on { "On" } else { "Off" }),
                    ),
            );
        }

        let preview = settings.include_preview;
        section = section.child(
            div()
                .px_3()
                .py_1()
                .flex()
                .justify_between()
                .text_xs()
                .child("Include message preview")
                .child(
                    div()
                        .id("notify-preview")
                        .text_color(if preview { theme.accent } else { theme.muted })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_notify_preview(!preview, cx);
                        }))
                        .child(if preview { "On" } else { "Off" }),
                ),
        );

        if !settings.enabled {
            section = section.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("Notifications are off."),
            );
        }

        section
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

    fn render_project_switcher(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active_name = self
            .projects
            .active()
            .map(|project| project.display_name().to_owned())
            .unwrap_or_else(|| "No project".to_owned());
        let canonical = self
            .projects
            .active_canonical()
            .unwrap_or_default()
            .to_owned();

        let mut section = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .pt_3()
            .pb_2()
            .child(
                Button::new("project-switcher-toggle")
                    .small()
                    .ghost()
                    .label(SharedString::from(format!("{active_name} ▾")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.switcher.toggle();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(canonical),
            );

        if let Some(notice) = &self.projects_notice {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(notice.clone()),
            );
        }

        if self.switcher.is_open() {
            let mut menu = div()
                .mt_1()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg)
                .p_1();
            for project in self.projects.projects() {
                let id = project.id.clone();
                let name = project.display_name().to_owned();
                let path = project.canonical.clone();
                let active = self.projects.active_id() == Some(&project.id);
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("project-{path}")))
                        .flex()
                        .flex_col()
                        .cursor_pointer()
                        .rounded_sm()
                        .px_2()
                        .py_1()
                        .bg(if active { theme.border } else { theme.bg })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch_project(id.clone(), window, cx);
                        }))
                        .child(div().text_sm().child(name))
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(path),
                        ),
                );
            }
            menu = menu.child(
                Button::new("project-add")
                    .small()
                    .ghost()
                    .label("Add folder…")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.add_project_folder(window, cx)),
                    ),
            );
            section = section.child(menu);
        }

        section.child(div().mt_2().h(px(1.)).bg(theme.border))
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
            .child(self.render_project_switcher(theme, cx))
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
                        Button::new("import-session")
                            .small()
                            .ghost()
                            .label("Import…")
                            .disabled(!ready)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.begin_import(window, cx)),
                            ),
                    )
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.create_session(window, cx)),
                            ),
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
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_session(id.clone(), window, cx)),
            );

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

        if self.transfer.stats_open {
            main = main.child(self.stats_pane(theme, cx));
        } else if let Some(session) = self.sessions.selected() {
            if let Some(file) = &self.transfer.import_preview {
                main = main.child(self.import_panel(file, theme, cx));
            }
            main = main.child(self.selected_pane(session, theme, cx));
        } else {
            if let Some(file) = &self.transfer.import_preview {
                main = main.child(self.import_panel(file, theme, cx));
            }
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
                    .on_click(cx.listener(|this, _, window, cx| this.create_session(window, cx))),
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
                Button::new("export-session")
                    .small()
                    .ghost()
                    .label("Export…")
                    .on_click(cx.listener(|this, _, _, cx| this.begin_export(cx))),
            )
            .child(
                Button::new("session-stats")
                    .small()
                    .ghost()
                    .label("Stats")
                    .on_click(cx.listener(|this, _, _, cx| this.open_stats(cx))),
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

        if self.transfer.export_open {
            pane = pane.child(self.export_panel(theme, cx));
        }

        let mut pane = pane.child(self.transcript_pane(theme, cx));

        if let Some(request) = self.permissions.front() {
            pane = pane.child(self.permission_sheet(request, theme, cx));
        }

        if let Some(staged) = self.revert.staged() {
            let files = staged.files.len();
            let message_id = staged.message_id.clone();
            let session_id = staged.session_id.clone();
            let mut panel = div()
                .id("revert-panel")
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(theme.surface)
                .border_t_1()
                .border_color(theme.warning)
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(format!("Revert to {message_id} · {files} file(s)")),
                );
            panel = panel.child(
                div()
                    .id("revert-commit")
                    .px_2()
                    .py_1()
                    .text_xs()
                    .bg(theme.warning)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.engine
                            .commit_revert(this.directory(), session_id.clone());
                        cx.notify();
                    }))
                    .child("Revert"),
            );
            let session_id = staged.session_id.clone();
            panel = panel.child(
                div()
                    .id("revert-cancel")
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.engine
                            .clear_revert(this.directory(), session_id.clone());
                        this.revert.clear();
                        cx.notify();
                    }))
                    .child("Cancel"),
            );
            pane = pane.child(panel);
        }
        if let Some(form) = self.forms.front() {
            pane = pane.child(self.render_form(theme, form, cx));
        }
        if !self.inbox.is_empty() {
            pane = pane.child(self.render_inbox(theme, cx));
        }
        pane = pane.child(self.render_context_meter(theme, cx));
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

    fn transcript_pane(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
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
                list = list.child(self.render_entry(entry, theme, cx));
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

    fn render_entry(
        &self,
        entry: &Entry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match entry {
            Entry::User { text, .. } => {
                let message_id = entry.id().to_string();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(theme.muted).child("You"))
                    .child(div().min_w_0().text_sm().child(text.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("fork-{message_id}")))
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.accent))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fork_from(message_id.clone());
                                cx.notify();
                            }))
                            .child("Fork from here"),
                    )
                    .child({
                        let message_id = entry.id().to_string();
                        div()
                            .id(SharedString::from(format!("revert-{message_id}")))
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.warning))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.revert_from(message_id.clone());
                                cx.notify();
                            }))
                            .child("Revert to here")
                    })
            }
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

        let model_label = self
            .sessions
            .selected()
            .and_then(|session| session.model.as_ref())
            .map(|model| model.id.clone())
            .unwrap_or_else(|| "Model".into());
        let agent_label = self
            .sessions
            .selected()
            .and_then(|session| session.agent.clone())
            .unwrap_or_else(|| "Agent".into());

        let mut column = div()
            .border_t_1()
            .border_color(theme.border)
            .p_4()
            .flex()
            .flex_col()
            .gap_2();

        let has_chips = !self.draft.files().is_empty()
            || !self.draft.agents().is_empty()
            || !self.draft.skills().is_empty()
            || self.draft.command().is_some();
        if has_chips {
            let mut chips = div().flex().flex_wrap().gap_2();
            for (index, file) in self.draft.files().iter().enumerate() {
                chips = chips.child(attachment_chip(
                    &theme,
                    file.path.clone(),
                    SharedString::from(format!("file-chip-{index}")),
                    cx.listener(move |this, _, window, cx| {
                        this.remove_file_chip(index, window, cx)
                    }),
                ));
            }
            for (index, agent) in self.draft.agents().iter().enumerate() {
                chips = chips.child(attachment_chip(
                    &theme,
                    format!("@{}", agent.name),
                    SharedString::from(format!("agent-chip-{index}")),
                    cx.listener(move |this, _, _, cx| {
                        this.draft.remove_agent(index);
                        cx.notify();
                    }),
                ));
            }
            for (index, skill) in self.draft.skills().iter().enumerate() {
                chips = chips.child(attachment_chip(
                    &theme,
                    format!("${}", skill.name),
                    SharedString::from(format!("skill-chip-{index}")),
                    cx.listener(move |this, _, _, cx| {
                        this.draft.remove_skill(index);
                        cx.notify();
                    }),
                ));
            }
            if let Some(command) = self.draft.command() {
                chips = chips.child(attachment_chip(
                    &theme,
                    format!("/{}", command.name),
                    "command-chip-remove",
                    cx.listener(|this, _, window, cx| this.remove_command_chip(window, cx)),
                ));
            }
            column = column.child(chips);
        }

        if let Some(picker) = &self.picker {
            let mut panel = div()
                .id("composer-picker-panel")
                .debug_selector(|| "composer-picker".into())
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .max_h(px(280.))
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(picker.kind().title()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(if picker.loading() { "Searching…" } else { "" }),
                        ),
                );

            if let Some(input) = &self.picker_input {
                panel = panel.child(Input::new(input).w_full());
            }

            if let Some(error) = picker.error() {
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(error.to_owned()),
                );
            } else if picker.visible_len() == 0 {
                panel = panel.child(div().text_xs().text_color(theme.muted).child("No matches."));
            } else {
                for (index, item) in picker.visible().take(8).enumerate() {
                    let selected = picker.highlighted() == index;
                    let label = item.label();
                    let detail = item.detail();
                    let mut row = div()
                        .id(SharedString::from(format!("picker-row-{index}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.border))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_index(index, window, cx)
                        }))
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(label));
                    if !detail.is_empty() {
                        row = row.child(
                            div()
                                .max_w(px(320.))
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(detail),
                        );
                    }
                    panel = panel.child(if selected { row.bg(theme.border) } else { row });
                }
            }

            column = column.child(panel);
        }

        let input_row = div().flex().items_end().gap_2().child(
            div()
                .flex_1()
                .min_w_0()
                .child(Textarea::new(&self.composer).h(px(84.))),
        );

        let input_row = if busy {
            input_row.child(
                Button::new("stop-turn")
                    .small()
                    .danger()
                    .label("Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_turn(cx))),
            )
        } else {
            input_row.child(
                Button::new("send-prompt")
                    .small()
                    .primary()
                    .label("Send")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.send_prompt(window, cx))),
            )
        };
        column = column.child(input_row);

        let weak = cx.weak_entity();
        column
            .capture_key_down(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                if !matches!(key, "up" | "down" | "escape" | "tab") {
                    return;
                }
                let Some(this) = weak.upgrade() else {
                    return;
                };
                let handled = this.update(cx, |this, cx| this.picker_key(key, window, cx));
                if handled {
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("model-picker")
                            .small()
                            .ghost()
                            .label(model_label)
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_session_picker(PickerKind::Model, window, cx)
                            })),
                    )
                    .child(
                        Button::new("agent-picker")
                            .small()
                            .ghost()
                            .label(agent_label)
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_session_picker(PickerKind::Agent, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("@ files · / commands · $ skills · # snippets"),
                    ),
            )
    }

    fn remove_command_chip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.draft.remove_command() {
            self.apply_draft_edit(edit, window, cx);
        } else {
            self.draft.clear_command();
        }
        cx.notify();
    }

    fn export_panel(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let sanitize = self.transfer.sanitize;
        let exporting = self.transfer.exporting;
        let notice = self.transfer.export_notice.clone();

        let mut panel = div()
            .mx_6()
            .mt_4()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child("Export session"))
            .child(div().text_xs().text_color(theme.muted).child(
                "The file contains this session's messages. Sanitizing redacts titles, \
                         directories, and secrets before the file is written.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("toggle-sanitize")
                            .small()
                            .ghost()
                            .label(if sanitize {
                                "Sanitize secrets: On"
                            } else {
                                "Sanitize secrets: Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sanitize(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("choose-export-path")
                            .small()
                            .primary()
                            .label(if exporting {
                                "Exporting…"
                            } else {
                                "Choose file…"
                            })
                            .disabled(exporting)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_export_path(window, cx)
                            })),
                    )
                    .child(
                        Button::new("cancel-export")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_export(cx))),
                    ),
            );

        if let Some(notice) = notice {
            panel = panel.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.success)
                    .child(notice),
            );
        }

        panel
    }

    fn import_panel(
        &self,
        file: &ImportFile,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let preview = &file.preview;
        let importing = self.transfer.importing;
        let as_copy = self.transfer.import_as_copy;
        let from_elsewhere = preview.project.as_deref().is_some_and(|project| {
            !project.starts_with("[redacted")
                && project != self.project_dir.to_string_lossy().as_ref()
        });

        let mut panel =
            div()
                .mx_6()
                .mt_4()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme.accent)
                .bg(theme.surface)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().text_sm().child("Import session"))
                        .child(
                            Button::new("cancel-import")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_import(cx))),
                        )
                        .child(
                            Button::new("confirm-import")
                                .small()
                                .primary()
                                .label(if importing { "Importing…" } else { "Import" })
                                .disabled(importing)
                                .on_click(cx.listener(|this, _, _, cx| this.confirm_import(cx))),
                        ),
                )
                .child(div().text_lg().child(preview.title.clone()))
                .child(div().text_xs().text_color(theme.muted).child(format!(
                    "{} messages · {} · {}",
                    preview.message_count,
                    preview.project.as_deref().unwrap_or("unknown project"),
                    preview.session_id
                )))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("toggle-import-copy")
                                .small()
                                .ghost()
                                .label(if as_copy {
                                    "Import as a copy: On"
                                } else {
                                    "Import as a copy: Off"
                                })
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.toggle_import_copy(cx)),
                                ),
                        )
                        .child(div().flex_1().text_xs().text_color(theme.muted).child(
                            if as_copy {
                                "New session and message ids; the original stays untouched."
                            } else {
                                "Keeps the exported ids; refused if the session already exists."
                            },
                        )),
                );

        if from_elsewhere {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child("This export came from a different project."),
            );
        }

        panel
    }

    fn stats_pane(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let range = self.transfer.range(now_ms());
        let stats = self.transfer_store.stats(range).cloned();
        let has_stats = stats.is_some();
        let loading = self.transfer.stats_loading;
        let preset = self.transfer.stats_preset;

        let mut pane = div()
            .id("stats-pane")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xl().child("Statistics"))
                    .child(div().flex_1())
                    .children(StatsPreset::ALL.into_iter().map(|option| {
                        Button::new(SharedString::from(format!(
                            "stats-preset-{}",
                            option.label()
                        )))
                        .small()
                        .ghost()
                        .label(if option == preset {
                            format!("{} ✓", option.label())
                        } else {
                            option.label().to_string()
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.choose_stats_preset(option, cx)),
                        )
                    }))
                    .child(
                        Button::new("close-stats")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.close_stats(cx))),
                    ),
            )
            .child(div().text_xs().text_color(theme.muted).child(
                "Totals cover every session in the server's storage, not only this project.",
            ));

        if loading && stats.is_none() {
            pane = pane.child(div().text_sm().text_color(theme.muted).child("Loading…"));
        }

        if let Some(stats) = stats {
            if stats.is_empty() {
                pane = pane.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted)
                        .child("No activity in this range."),
                );
            } else {
                let tile = |label: &'static str, value: String| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(110.))
                        .child(div().text_xs().text_color(theme.muted).child(label))
                        .child(div().text_lg().child(value))
                };
                pane = pane
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_4()
                            .child(tile("Sessions", compact_count(stats.sessions as f64)))
                            .child(tile("Prompts", compact_count(stats.prompts as f64)))
                            .child(tile("Steps", compact_count(stats.steps as f64)))
                            .child(tile("Active days", compact_count(stats.active_days as f64)))
                            .child(tile("Streak", compact_count(stats.streak as f64)))
                            .child(tile("Cost", money(stats.cost))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(token_summary(stats.tokens)),
                    );

                let mut models = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_xs().text_color(theme.muted).child("Per model"));
                if stats.models.is_empty() {
                    models = models.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted)
                            .child("No model activity in this range."),
                    );
                }
                for model in &stats.models {
                    models = models.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(model.label()),
                            )
                            .child(div().text_xs().text_color(theme.muted).child(format!(
                                "{} steps · {} · {}",
                                compact_count(model.steps as f64),
                                compact_count(
                                    model.tokens.input
                                        + model.tokens.output
                                        + model.tokens.reasoning
                                        + model.tokens.cache.read
                                        + model.tokens.cache.write
                                ),
                                money(model.cost)
                            ))),
                    );
                }
                pane = pane.child(models);
            }
        } else if !loading {
            pane = pane.child(
                div()
                    .text_sm()
                    .text_color(theme.muted)
                    .child("No statistics available."),
            );
        }

        if loading && has_stats {
            pane = pane.child(div().text_xs().text_color(theme.muted).child("Refreshing…"));
        }

        pane
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

fn is_session_picker(kind: PickerKind) -> bool {
    matches!(kind, PickerKind::Model | PickerKind::Agent)
}

fn catalog_kind(kind: PickerKind) -> Option<CatalogKind> {
    match kind {
        PickerKind::Command => Some(CatalogKind::Commands),
        PickerKind::Skill => Some(CatalogKind::Skills),
        PickerKind::File | PickerKind::Agent => Some(CatalogKind::Agents),
        PickerKind::Model => Some(CatalogKind::Models),
        PickerKind::Snippet => None,
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
            .on_action(
                cx.listener(|this, _: &NewSession, window, cx| this.create_session(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleTheme, window, cx| this.toggle_theme(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusComposer, window, cx| this.focus_composer(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleProjectSwitcher, _, cx| {
                this.switcher.toggle();
                cx.notify();
            }))
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
                    .child(self.render_main(cx))
                    .child(self.render_changes(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui_kit::component::Theme as KitTheme;
    use gpui_kit::component::input::InputEvent;
    use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
    use harness_core::{DraftSend, PickerItem, PickerKind, PickerState};
    use harness_engine::{Engine, EngineEvent, StreamEvent};

    use super::WorkspaceView;

    fn diff_file(name: &str) -> harness_protocol::FileDiff {
        harness_protocol::FileDiff {
            file: name.into(),
            patch: "diff --git a/a b/a\n@@ -1 +1 @@\n-a\n+b\n".into(),
            additions: 1,
            deletions: 1,
            status: "modified".into(),
            more: false,
        }
    }

    #[gpui_kit::test]
    fn revert_stage_and_cancel_update_the_store(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-revert-test");
        let engine = Engine::stub_for_tests();
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        view.update(cx, |view, _| {
            view.apply(harness_engine::EngineEvent::RevertStaged {
                session_id: "ses_1".into(),
                revert: harness_protocol::SessionRevert {
                    message_id: "msg_2".into(),
                    part_id: None,
                    snapshot: None,
                    files: Vec::new(),
                },
            });
            assert!(view.revert.is_staged());
            view.apply(harness_engine::EngineEvent::RevertCleared {
                session_id: "ses_1".into(),
            });
            assert!(!view.revert.is_staged());
        });
    }

    #[gpui_kit::test]
    fn forked_session_is_selected(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-fork-test");
        let engine = Engine::stub_for_tests();
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        view.update(cx, |view, _| {
            view.sessions.replace(vec![harness_protocol::Session {
                id: "ses_1".into(),
                title: None,
                project_id: None,
                agent: None,
                model: None,
                time: harness_protocol::SessionTime {
                    created: 10,
                    updated: 10,
                },
                location: None,
                tokens: None,
                cost: None,
            }]);
            view.apply(harness_engine::EngineEvent::SessionForked {
                session: harness_protocol::Session {
                    id: "ses_2".into(),
                    title: None,
                    project_id: None,
                    agent: None,
                    model: None,
                    time: harness_protocol::SessionTime {
                        created: 20,
                        updated: 20,
                    },
                    location: None,
                    tokens: None,
                    cost: None,
                },
            });
            assert_eq!(view.sessions.selected_id(), Some("ses_2"));
            assert_eq!(view.sessions.len(), 2);
        });
    }

    #[gpui_kit::test]
    fn changes_selection_filters_the_diff_rows(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-diff-panel-test");
        let engine = Engine::stub_for_tests();
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        view.update(cx, |view, _| {
            view.diff_files = vec![diff_file("a.txt"), diff_file("b.txt")];
            view.rebuild_diff();
            assert_eq!(view.diff_rows.len(), 8);
            view.select_file("a.txt");
            assert_eq!(view.diff_rows.len(), 4);
            view.select_file("a.txt");
            assert_eq!(view.diff_rows.len(), 8);
        });
    }

    #[gpui_kit::test]
    fn title_bar_is_rendered_above_the_workspace(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-titlebar-test");
        let engine = Engine::stub_for_tests();

        let (_, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
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

    #[gpui_kit::test]
    fn typing_a_file_trigger_inserts_the_selected_mention(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(KitTheme::default()));

        let project_dir = PathBuf::from("/tmp/rust-harness-composer-test");
        let engine = Engine::stub_for_tests();

        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("@src/ma", window, cx);
                    input.set_selected_range(7..7, cx);
                });
                view.composer_changed(cx);
                assert_eq!(
                    view.picker.as_ref().map(PickerState::kind),
                    Some(PickerKind::File)
                );

                let request_id = view.picker.as_ref().unwrap().request_id();
                view.picker.as_mut().unwrap().apply_file_results(
                    request_id,
                    [PickerItem::File {
                        path: "src/main.rs".into(),
                    }],
                );
                view.choose_selected(window, cx);

                assert!(view.picker.is_none());
                assert_eq!(view.composer.read(cx).value().as_ref(), "@src/main.rs");

                let Some(DraftSend::Prompt(request)) = view.draft.to_send() else {
                    panic!("expected a prompt");
                };
                assert_eq!(request.files.len(), 1);
                let mention = request.files[0].mention.as_ref().unwrap();
                assert_eq!((mention.start, mention.end), (0, 12));
                assert_eq!(
                    request.files[0].uri,
                    "file:///tmp/rust-harness-composer-test/src/main.rs"
                );
            });
        });
    }

    #[gpui_kit::test]
    fn enter_with_the_picker_open_chooses_instead_of_sending(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let project_dir = PathBuf::from("/tmp/rust-harness-composer-enter-test");
        let engine = Engine::stub_for_tests();

        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("@src/ma", window, cx);
                    input.set_selected_range(7..7, cx);
                    input.focus(window, cx);
                });
                view.composer_changed(cx);
                assert!(view.picker.is_some());
            });
            window.draw(cx).clear(cx);
        });

        // With no results yet, Enter must not send the message.
        press_enter(&view, cx);
        cx.update(|window, cx| {
            let _ = window;
            view.update(cx, |view, _| {
                assert!(view.picker.is_some());
                assert!(view.error.is_none());
                assert_eq!(view.draft.text(), "@src/ma");
            });
        });

        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                let request_id = view.picker.as_ref().unwrap().request_id();
                view.picker.as_mut().unwrap().apply_file_results(
                    request_id,
                    [PickerItem::File {
                        path: "src/main.rs".into(),
                    }],
                );
            });
        });

        press_enter(&view, cx);
        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                assert!(view.picker.is_none());
                assert_eq!(view.draft.text(), "@src/main.rs");
                assert_eq!(view.draft.files().len(), 1);
            });
        });
    }

    #[gpui_kit::test]
    fn picker_keys_navigate_choose_and_close(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let project_dir = PathBuf::from("/tmp/rust-harness-composer-keys-test");
        let engine = Engine::stub_for_tests();

        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                crate::notify::NotifyChannels::disabled(),
                cx,
            )
        });

        let items = [
            PickerItem::Snippet {
                name: "review".into(),
                description: String::new(),
                expansion: "Review the diff.".into(),
            },
            PickerItem::Snippet {
                name: "test".into(),
                description: String::new(),
                expansion: "Run the tests.".into(),
            },
        ];

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("#", window, cx);
                    input.set_selected_range(1..1, cx);
                });
                view.draft.edit("#", 1, true);

                let (picker, _) = PickerState::open(PickerKind::Snippet, "", items.to_vec());
                view.picker = Some(picker);

                assert!(view.picker_key("down", window, cx));
                assert_eq!(view.picker.as_ref().unwrap().highlighted(), 1);
                assert!(view.picker_key("up", window, cx));
                assert_eq!(view.picker.as_ref().unwrap().highlighted(), 0);

                assert!(view.picker_key("tab", window, cx));
                assert!(view.picker.is_none());
                assert_eq!(view.draft.text(), "Review the diff.");
            });

            view.update(cx, |view, cx| {
                let (picker, _) = PickerState::open(PickerKind::Snippet, "", items.to_vec());
                view.picker = Some(picker);

                assert!(view.picker_key("escape", window, cx));
                assert!(view.picker.is_none());
            });
        });
    }

    fn notify_session(id: &str, title: &str) -> harness_protocol::Session {
        harness_protocol::Session {
            id: id.into(),
            title: Some(title.into()),
            project_id: None,
            agent: None,
            model: None,
            time: harness_protocol::SessionTime {
                created: 10,
                updated: 10,
            },
            location: None,
            tokens: None,
            cost: None,
        }
    }

    #[gpui_kit::test]
    fn unfocused_turn_done_is_delivered(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let project_dir = PathBuf::from("/tmp/rust-harness-notify-delivery-test");
        let engine = Engine::stub_for_tests();
        let (notifications_sender, notifications) = async_channel::unbounded();
        let (settings_sender, _settings_receiver) = async_channel::unbounded();
        let (_status_sender, status) = async_channel::unbounded();
        let channels = crate::notify::NotifyChannels {
            notifications: notifications_sender,
            settings: settings_sender,
            status,
        };
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                channels,
                cx,
            )
        });

        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                view.sessions
                    .replace(vec![notify_session("ses_1", "Fix the build")]);
                view.sessions.select("ses_1");
                view.notify_focused = false;
                view.apply(EngineEvent::Stream(StreamEvent::ExecutionFinished {
                    session_id: "ses_1".into(),
                    event_id: "evt_1".into(),
                    outcome: harness_protocol::ExecutionOutcome::Succeeded,
                }));

                let notification = notifications.try_recv().expect("notification");
                assert_eq!(notification.kind, harness_core::NotifyKind::TurnDone);
                assert_eq!(notification.session_title, "Fix the build");
                assert_eq!(notification.body, "Turn finished");
            });
        });
    }

    #[gpui_kit::test]
    fn focused_active_session_is_not_notified(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let project_dir = PathBuf::from("/tmp/rust-harness-notify-focus-test");
        let engine = Engine::stub_for_tests();
        let (notifications_sender, notifications) = async_channel::unbounded();
        let (settings_sender, _settings_receiver) = async_channel::unbounded();
        let (_status_sender, status) = async_channel::unbounded();
        let channels = crate::notify::NotifyChannels {
            notifications: notifications_sender,
            settings: settings_sender,
            status,
        };
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                channels,
                cx,
            )
        });

        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                view.sessions
                    .replace(vec![notify_session("ses_1", "Fix the build")]);
                view.sessions.select("ses_1");
                view.notify_focused = true;
                view.apply(EngineEvent::Stream(StreamEvent::ExecutionFinished {
                    session_id: "ses_1".into(),
                    event_id: "evt_1".into(),
                    outcome: harness_protocol::ExecutionOutcome::Succeeded,
                }));
                assert!(notifications.try_recv().is_err());
            });
        });
    }

    #[gpui_kit::test]
    fn denied_permission_disables_and_persists(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);

        let project_dir = PathBuf::from("/tmp/rust-harness-notify-denied-test");
        let engine = Engine::stub_for_tests();
        let (notifications_sender, _notifications) = async_channel::unbounded();
        let (settings_sender, settings) = async_channel::unbounded();
        let (_status_sender, status) = async_channel::unbounded();
        let channels = crate::notify::NotifyChannels {
            notifications: notifications_sender,
            settings: settings_sender,
            status,
        };
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkspaceView::new(
                window,
                engine,
                project_dir.clone(),
                harness_core::NotifySettings::default(),
                channels,
                cx,
            )
        });

        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.on_notify_denied(cx);
                assert!(view.notify_denied);
                assert!(!view.notify.settings().enabled);
                let persisted = settings.try_recv().expect("settings written");
                assert!(!persisted.enabled);

                // A second denial must not write another setting.
                view.on_notify_denied(cx);
                assert!(settings.try_recv().is_err());
            });
        });
    }

    /// Deliver Enter to the composer the way a keypress would, after the
    /// current entity update has finished.
    fn press_enter(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let composer = view.composer.clone();
                cx.defer_in(window, move |_, _, cx| {
                    composer.update(cx, |_, cx| {
                        cx.emit(InputEvent::PressEnter {
                            secondary: false,
                            shift: false,
                        });
                    });
                });
            });
        });
        cx.background_executor.run_until_parked();
    }
}
