//! Small time helpers.

/// Current wall-clock time in epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// A compact "3m ago" label for a session timestamp.
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    if then_ms <= 0 {
        return String::new();
    }
    let seconds = now_ms.saturating_sub(then_ms).max(0) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        86_400..=604_799 => format!("{}d ago", seconds / 86_400),
        604_800..=2_591_999 => format!("{}w ago", seconds / 604_800),
        2_592_000..=31_535_999 => format!("{}mo ago", seconds / 2_592_000),
        _ => format!("{}y ago", seconds / 31_536_000),
    }
}
