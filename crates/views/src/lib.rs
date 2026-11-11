//! Application views.

mod notify;
mod workspace;

pub use notify::{NotifyChannels, NotifyStatus};
pub use workspace::{CancelOverlays, FocusComposer, NewSession, ToggleTheme, WorkspaceView};
