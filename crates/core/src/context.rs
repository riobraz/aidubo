//! Context usage and compaction state.

use harness_protocol::TokenUsage;

/// Where the session sits in its context window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ContextUsage {
    pub tokens: TokenUsage,
    pub cost: Option<f64>,
    pub window: Option<u64>,
    pub stale: bool,
}

impl ContextUsage {
    /// Total tokens counted against the window.
    pub fn used(&self) -> f64 {
        self.tokens.input
            + self.tokens.output
            + self.tokens.reasoning
            + self.tokens.cache.read
            + self.tokens.cache.write
    }

    /// Share of the window, or `None` when the window is unknown.
    pub fn share(&self) -> Option<f64> {
        let window = self.window.filter(|window| *window > 0)?;
        Some((self.used() / window as f64).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompactionState {
    #[default]
    Idle,
    Running,
    Failed,
}

#[derive(Debug, Default)]
pub struct ContextStore {
    usage: ContextUsage,
    compaction: CompactionState,
    error: Option<String>,
}

impl ContextStore {
    pub fn set_usage(&mut self, tokens: TokenUsage, cost: Option<f64>) {
        self.usage.tokens = tokens;
        self.usage.cost = cost;
        self.usage.stale = false;
    }

    pub fn set_window(&mut self, window: Option<u64>) {
        self.usage.window = window;
    }

    pub fn usage(&self) -> &ContextUsage {
        &self.usage
    }

    pub fn share(&self) -> Option<f64> {
        self.usage.share()
    }

    pub fn mark_stale(&mut self) {
        self.usage.stale = true;
    }

    pub fn is_stale(&self) -> bool {
        self.usage.stale
    }

    pub fn compaction(&self) -> CompactionState {
        self.compaction
    }

    pub fn begin_compaction(&mut self) {
        self.compaction = CompactionState::Running;
        self.error = None;
    }

    pub fn finish_compaction(&mut self) {
        self.compaction = CompactionState::Idle;
    }

    pub fn fail_compaction(&mut self, message: impl Into<String>) {
        self.compaction = CompactionState::Failed;
        self.error = Some(message.into());
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: f64) -> TokenUsage {
        TokenUsage {
            input,
            ..TokenUsage::default()
        }
    }

    #[test]
    fn usage_unknown_window_is_not_zero() {
        let mut store = ContextStore::default();
        store.set_usage(tokens(100.0), None);
        assert_eq!(store.share(), None);
        store.set_window(Some(0));
        assert_eq!(store.share(), None);
        store.set_window(Some(1000));
        assert_eq!(store.share(), Some(0.1));
    }

    #[test]
    fn usage_marks_stale_and_clears_on_update() {
        let mut store = ContextStore::default();
        store.mark_stale();
        assert!(store.is_stale());
        store.set_usage(tokens(1.0), Some(0.1));
        assert!(!store.is_stale());
    }

    #[test]
    fn compaction_failure_clears_running() {
        let mut store = ContextStore::default();
        store.begin_compaction();
        assert_eq!(store.compaction(), CompactionState::Running);
        store.fail_compaction("provider error");
        assert_eq!(store.compaction(), CompactionState::Failed);
        assert_eq!(store.error(), Some("provider error"));
        store.begin_compaction();
        store.finish_compaction();
        assert_eq!(store.compaction(), CompactionState::Idle);
    }

    #[test]
    fn share_clamps_to_one() {
        let mut store = ContextStore::default();
        store.set_usage(tokens(5000.0), None);
        store.set_window(Some(1000));
        assert_eq!(store.share(), Some(1.0));
    }
}
