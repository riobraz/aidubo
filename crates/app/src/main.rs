//! Rust Harness — M1 composition root.

mod diagnostics;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{Theme, TitleBar};
use gpui_kit::*;
use harness_engine::{Engine, EngineConfig};
use harness_views::{
    CancelOverlays, FocusComposer, NewSession, NotifyChannels, ToggleProjectSwitcher, ToggleTheme,
    WorkspaceView,
};
use rust_harness::{notify, persistence};

fn main() {
    if std::env::args().any(|arg| arg == "--diagnostics") {
        print_diagnostics();
        return;
    }

    init_tracing();
    rust_harness::update::spawn_launch_check();

    let restored = persistence::restore();
    let project_dir = resolve_project_dir(&restored);
    if !project_dir.is_dir() {
        tracing::warn!(
            path = %project_dir.display(),
            "project directory does not exist; the engine will report an error"
        );
    }
    let saved_projects = restored.projects;
    let saved_worktrees = restored.worktrees;
    let saved_notice = restored.notice;

    let (projects_sender, projects_receiver) = async_channel::unbounded();
    if let Err(error) = std::thread::Builder::new()
        .name("project-list".into())
        .spawn(move || {
            while let Ok(snapshot) = projects_receiver.recv_blocking() {
                if let Err(error) = persistence::save(&snapshot) {
                    tracing::warn!(%error, "could not persist the project list");
                }
            }
        })
    {
        tracing::warn!(%error, "could not start the project list thread");
    }

    let notify_settings = notify::load_settings();
    let notifier = notify::from_env();
    let (notifications_sender, notifications_receiver) = async_channel::unbounded();
    let (settings_sender, settings_receiver) = async_channel::unbounded();
    let (status_sender, status_receiver) = async_channel::unbounded();
    notify::spawn_delivery(
        notifier,
        notifications_receiver,
        settings_receiver,
        status_sender,
    );
    let notify_channels = NotifyChannels {
        notifications: notifications_sender,
        settings: settings_sender,
        status: status_receiver,
    };

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        let engine = Engine::spawn(EngineConfig::new(project_dir.clone()));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1120.), px(720.)), cx)),
            ..TitleBar::window_options()
        };

        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            // Follow the OS appearance on startup; the sidebar toggle can
            // override it for the rest of the session.
            Theme::sync_system_appearance(Some(window), cx);
            window.activate_window();
            window.set_window_title("Rust Harness");
            cx.bind_keys([
                KeyBinding::new("cmd-n", NewSession, Some("Workspace")),
                KeyBinding::new("ctrl-n", NewSession, Some("Workspace")),
                KeyBinding::new("cmd-l", FocusComposer, Some("Workspace")),
                KeyBinding::new("ctrl-l", FocusComposer, Some("Workspace")),
                KeyBinding::new("cmd-shift-l", ToggleTheme, Some("Workspace")),
                KeyBinding::new("ctrl-shift-l", ToggleTheme, Some("Workspace")),
                KeyBinding::new("cmd-shift-p", ToggleProjectSwitcher, Some("Workspace")),
                KeyBinding::new("ctrl-shift-p", ToggleProjectSwitcher, Some("Workspace")),
                KeyBinding::new("escape", CancelOverlays, Some("Workspace")),
            ]);
            cx.new(|cx| {
                WorkspaceView::new(
                    window,
                    engine,
                    project_dir,
                    notify_settings,
                    notify_channels,
                    cx,
                )
                .restore_saved_projects(saved_projects, saved_notice)
                .restore_saved_worktrees(saved_worktrees)
                .persist_projects_with(Arc::new(move |snapshot| {
                    if projects_sender.try_send(snapshot).is_err() {
                        tracing::warn!("could not queue the project list for saving");
                    }
                }))
            })
        });

        if let Err(error) = opened {
            tracing::error!(%error, "failed to open the main window");
            cx.quit();
            return;
        }

        // Development aid: let a smoke test drive the full boot → connect →
        // clean-shutdown path without a human closing the window.
        if let Some(millis) = autoquit_ms() {
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(Duration::from_millis(millis)).await;
                tracing::info!(millis, "auto-quitting");
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });

    tracing::info!("Rust Harness exited");
}

/// Startup directory: an explicit positional argument or `RUST_HARNESS_PROJECT`
/// wins, then the active project from the last run, then any restored project,
/// then the current working directory.
fn resolve_project_dir(restored: &persistence::RestoredProjects) -> PathBuf {
    explicit_project_dir()
        .or_else(|| restored.active.as_deref().map(PathBuf::from))
        .or_else(|| restored.projects.first().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn explicit_project_dir() -> Option<PathBuf> {
    std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("RUST_HARNESS_PROJECT").map(PathBuf::from))
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harness_engine=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn autoquit_ms() -> Option<u64> {
    std::env::var("RUST_HARNESS_AUTOQUIT_MS").ok()?.parse().ok()
}

fn print_diagnostics() {
    let opencode = harness_engine::locate_binary(None)
        .map(|path| {
            let version = std::process::Command::new(&path)
                .arg("--version")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".into());
            format!("{} ({version})", path.display())
        })
        .unwrap_or_else(|_| "not found".into());
    let home = std::env::var("HOME").ok();
    let data_dir = home
        .as_ref()
        .map(|home| format!("{home}/Library/Application Support/Rust Harness"))
        .unwrap_or_default();
    print!(
        "{}",
        diagnostics::render(&diagnostics::ReportInput {
            app_version: env!("CARGO_PKG_VERSION"),
            target: &format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            os: std::env::consts::OS,
            opencode: &opencode,
            engine: "not started (diagnostics mode)",
            data_dir: &data_dir,
            errors: &[],
            home: home.as_deref(),
        })
    );
}
