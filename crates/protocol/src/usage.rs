//! Provider budget limits.

use serde::Deserialize;

/// One provider budget.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Budget {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub used: Option<f64>,
    #[serde(default)]
    pub limit: Option<f64>,
    #[serde(default)]
    pub reset_at: Option<i64>,
}

impl Budget {
    /// Share used, only when both values exist.
    pub fn share(&self) -> Option<f64> {
        let limit = self.limit.filter(|limit| *limit > 0.0)?;
        Some((self.used? / limit).clamp(0.0, 1.0))
    }
}

/// The usage limits payload.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct UsageLimits {
    #[serde(default)]
    pub budgets: Vec<Budget>,
    #[serde(default)]
    pub workspace: Option<Budget>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_limits_null_data_parses() {
        let limits: Option<UsageLimits> = serde_json::from_str("null").expect("null");
        assert!(limits.is_none());
    }

    #[test]
    fn budget_missing_fields_default() {
        let budget: Budget = serde_json::from_str(r#"{"provider":"anthropic"}"#).expect("budget");
        assert_eq!(budget.provider, "anthropic");
        assert!(budget.limit.is_none());
        assert_eq!(budget.share(), None);
    }

    #[test]
    fn budget_share_needs_both_values() {
        let partial: Budget = serde_json::from_str(r#"{"provider":"p","used":3}"#).expect("budget");
        assert_eq!(partial.share(), None);
        let full: Budget =
            serde_json::from_str(r#"{"provider":"p","used":3,"limit":12}"#).expect("budget");
        assert_eq!(full.share(), Some(0.25));
    }
}
