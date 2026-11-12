//! Transfer and stats UI state.

use std::path::PathBuf;

use harness_core::ImportPreview;
use harness_protocol::{StatsRange, TokenUsage};

/// Preset windows offered by the stats modal. Free-form dates would need a
/// date picker and server-side ISO parsing; the endpoint only takes epoch ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatsPreset {
    #[default]
    Week,
    Month,
    All,
}

impl StatsPreset {
    pub const ALL: [Self; 3] = [Self::Week, Self::Month, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::Week => "7 days",
            Self::Month => "30 days",
            Self::All => "All time",
        }
    }

    pub fn range(self, now_ms: i64) -> StatsRange {
        let day = 24 * 60 * 60 * 1000;
        match self {
            Self::Week => StatsRange {
                from: now_ms - 7 * day,
                to: now_ms,
            },
            Self::Month => StatsRange {
                from: now_ms - 30 * day,
                to: now_ms,
            },
            Self::All => StatsRange {
                from: 0,
                to: now_ms,
            },
        }
    }
}

/// The file waiting for the user's confirmation.
#[derive(Debug, Clone)]
pub struct ImportFile {
    pub path: PathBuf,
    pub preview: ImportPreview,
}

#[derive(Debug)]
pub struct TransferState {
    pub export_open: bool,
    /// Default on: exports contain prompt content.
    pub sanitize: bool,
    pub exporting: bool,
    pub export_notice: Option<String>,
    pub import_preview: Option<ImportFile>,
    pub importing: bool,
    /// Rewrite the session and message ids so the copy can live beside its
    /// source. Storage is global; keeping the ids is refused.
    pub import_as_copy: bool,
    pub stats_open: bool,
    pub stats_preset: StatsPreset,
    pub stats_loading: bool,
}

impl Default for TransferState {
    fn default() -> Self {
        Self {
            export_open: false,
            sanitize: true,
            exporting: false,
            export_notice: None,
            import_preview: None,
            importing: false,
            import_as_copy: true,
            stats_open: false,
            stats_preset: StatsPreset::Week,
            stats_loading: false,
        }
    }
}

impl TransferState {
    pub fn range(&self, now_ms: i64) -> StatsRange {
        self.stats_preset.range(now_ms)
    }
}

pub fn compact_count(value: f64) -> String {
    if value < 1_000.0 {
        return format!("{value:.0}");
    }
    if value < 1_000_000.0 {
        return format!("{:.1}k", value / 1_000.0);
    }
    format!("{:.1}M", value / 1_000_000.0)
}

pub fn money(value: f64) -> String {
    if value == 0.0 {
        "—".into()
    } else {
        format!("${value:.2}")
    }
}

pub fn token_summary(tokens: TokenUsage) -> String {
    format!(
        "in {} · out {} · reasoning {} · cache {}",
        compact_count(tokens.input),
        compact_count(tokens.output),
        compact_count(tokens.reasoning),
        compact_count(tokens.cache.read + tokens.cache.write)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_cover_their_window() {
        let now = 1_700_000_000_000;
        let week = StatsPreset::Week.range(now);
        assert_eq!(week.to, now);
        assert_eq!(week.from, now - 7 * 24 * 60 * 60 * 1000);
        assert_eq!(StatsPreset::All.range(now).from, 0);
        assert!(TransferState::default().sanitize);
    }

    #[test]
    fn numbers_stay_readable() {
        assert_eq!(compact_count(0.0), "0");
        assert_eq!(compact_count(999.0), "999");
        assert_eq!(compact_count(1_500.0), "1.5k");
        assert_eq!(compact_count(2_500_000.0), "2.5M");
        assert_eq!(money(0.0), "—");
        assert_eq!(money(1.5), "$1.50");
    }
}
