//! Application views.

mod notify;
mod project_switcher;
mod run_compare;
mod terminal;
mod terminal_keys;
mod transfer;
mod workspace;

pub use notify::{NotifyChannels, NotifyStatus};
pub use project_switcher::ProjectSwitcher;
pub use workspace::{
    CancelOverlays, FocusComposer, NewSession, ProjectSink, ProjectSnapshot, RunSink, RunSnapshot,
    ToggleProjectSwitcher, ToggleTheme, WorkspaceView,
};
