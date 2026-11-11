//! Application views.

mod notify;
mod transfer;
mod workspace;

pub use notify::{NotifyChannels, NotifyStatus};
pub use workspace::{CancelOverlays, FocusComposer, NewSession, ToggleTheme, WorkspaceView};
