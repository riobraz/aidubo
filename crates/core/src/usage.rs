//! Provider budget limits with a freshness rule.

use harness_protocol::UsageLimits;

/// Limits older than this are shown as unknown.
pub const LIMITS_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Default)]
pub struct UsageStore {
    limits: Option<UsageLimits>,
    loaded_at: i64,
    unavailable: bool,
}

impl UsageStore {
    pub fn set(&mut self, limits: Option<UsageLimits>, now: i64) {
        self.limits = limits;
        self.loaded_at = now;
        self.unavailable = self.limits.is_none();
    }

    pub fn mark_unavailable(&mut self, now: i64) {
        self.limits = None;
        self.loaded_at = now;
        self.unavailable = true;
    }

    pub fn limits(&self) -> Option<&UsageLimits> {
        self.limits.as_ref()
    }

    pub fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    pub fn is_fresh(&self, now: i64) -> bool {
        self.loaded_at > 0 && now.saturating_sub(self.loaded_at) < LIMITS_TTL_MS
    }

    pub fn budget_count(&self) -> usize {
        self.limits
            .as_ref()
            .map(|limits| limits.budgets.len() + usize::from(limits.workspace.is_some()))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::Budget;

    fn limits() -> UsageLimits {
        UsageLimits {
            budgets: vec![Budget {
                provider: "anthropic".into(),
                used: Some(3.0),
                limit: Some(12.0),
                ..Budget::default()
            }],
            workspace: None,
        }
    }

    #[test]
    fn usage_limits_503_is_unavailable_not_error() {
        let mut store = UsageStore::default();
        store.mark_unavailable(1_000);
        assert!(store.is_unavailable());
        assert!(store.limits().is_none());
        assert_eq!(store.budget_count(), 0);
    }

    #[test]
    fn budget_percentage_needs_both_values() {
        let budget = Budget::default();
        assert_eq!(budget.share(), None);
        let budget = Budget {
            used: Some(2.0),
            limit: Some(8.0),
            ..Budget::default()
        };
        assert_eq!(budget.share(), Some(0.25));
    }

    #[test]
    fn stale_limits_expire_after_ten_minutes() {
        let mut store = UsageStore::default();
        store.set(Some(limits()), 1_000);
        assert!(store.is_fresh(2_000));
        assert!(!store.is_fresh(1_000 + LIMITS_TTL_MS + 1));
        assert_eq!(store.budget_count(), 1);
    }
}
