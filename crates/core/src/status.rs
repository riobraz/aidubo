//! Sidecar lifecycle as the UI presents it.

/// Lifecycle of the OpenCode sidecar as the UI needs to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    /// Locating the binary and launching the process.
    Starting { port: Option<u16> },
    /// The server answered an authenticated request.
    Ready { version: String, port: u16 },
    /// Startup failed; the message is safe to show to the user.
    Failed { message: String },
    /// The engine was asked to stop, or exited cleanly.
    Stopped,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self::Starting { port: None }
    }
}

impl EngineStatus {
    /// Short label suitable for a status badge.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Starting { .. } => "Connecting",
            Self::Ready { .. } => "Connected",
            Self::Failed { .. } => "Engine failed",
            Self::Stopped => "Stopped",
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}
