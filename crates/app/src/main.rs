//! Rust Harness — M1 composition root.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::Theme;
use gpui_kit::*;
use harness_engine::{Engine, EngineConfig};
use harness_views::{CancelOverlays, FocusComposer, NewSession, ToggleTheme, WorkspaceView};

fn main() {
    init_tracing();

    let project_dir = resolve_project_dir();
    if !project_dir.is_dir() {
        tracing::warn!(
            path = %project_dir.display(),
            "project directory does not exist; the engine will report an error"
        );
    }

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        let engine = Engine::spawn(EngineConfig::new(project_dir.clone()));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1120.), px(720.)), cx)),
            ..Default::default()
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
                KeyBinding::new("escape", CancelOverlays, Some("Workspace")),
            ]);
            cx.new(|cx| WorkspaceView::new(window, engine, project_dir, cx))
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

/// Project directory: positional argument, then `RUST_HARNESS_PROJECT`, then
/// the current working directory.
fn resolve_project_dir() -> PathBuf {
    std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("RUST_HARNESS_PROJECT").map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harness_engine=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn autoquit_ms() -> Option<u64> {
    std::env::var("RUST_HARNESS_AUTOQUIT_MS").ok()?.parse().ok()
}
