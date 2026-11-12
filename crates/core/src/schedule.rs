//! Scheduled prompts.
//!
//! The store is a view over the server's schedules: the app never runs the
//! clock and never infers a next run locally.

use harness_protocol::{ScheduleInfo, ScheduleWhen};

const FAILURE_REASON_LIMIT: usize = 400;

#[derive(Debug, Default)]
pub struct ScheduleStore {
    schedules: Vec<ScheduleInfo>,
}

impl ScheduleStore {
    /// Replace the whole list from a server snapshot.
    pub fn replace(&mut self, schedules: Vec<ScheduleInfo>) {
        self.schedules = schedules;
        self.sort();
    }

    /// Insert or update one schedule by id.
    pub fn upsert(&mut self, schedule: ScheduleInfo) {
        match self
            .schedules
            .iter_mut()
            .find(|existing| existing.id == schedule.id)
        {
            Some(existing) => *existing = schedule,
            None => self.schedules.push(schedule),
        }
        self.sort();
    }

    fn sort(&mut self) {
        self.schedules.sort_by(|left, right| {
            let left_due = left.next_due_at.unwrap_or(f64::MAX);
            let right_due = right.next_due_at.unwrap_or(f64::MAX);
            left_due
                .partial_cmp(&right_due)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.time
                        .created
                        .partial_cmp(&right.time.created)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
    }

    pub fn schedules(&self) -> &[ScheduleInfo] {
        &self.schedules
    }

    /// Rows for the default view. Finished schedules stay in the store so the
    /// "Show finished" toggle can reveal them without a reload.
    pub fn visible(&self, show_finished: bool) -> impl Iterator<Item = &ScheduleInfo> {
        self.schedules
            .iter()
            .filter(move |schedule| show_finished || !schedule.is_done())
    }

    pub fn has_finished(&self) -> bool {
        self.schedules.iter().any(ScheduleInfo::is_done)
    }

    /// `remove(id)` is idempotent so a replayed delete is harmless.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.schedules.len();
        self.schedules.retain(|schedule| schedule.id != id);
        self.schedules.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty()
    }
}

/// The server accepts durations like `"90 seconds"` or `"2 hours"`. Validate
/// the simple grammar locally so an obvious typo never reaches the wire; the
/// server remains authoritative.
pub fn validate_duration(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("A duration is required.".into());
    }
    let Some((number, unit)) = trimmed.split_once(char::is_whitespace) else {
        return Err("Use a number and a unit, such as \"5 minutes\".".into());
    };
    let number: f64 = number
        .parse()
        .map_err(|_| "The duration must start with a number.".to_string())?;
    if !number.is_finite() || number <= 0.0 {
        return Err("The duration must be greater than zero.".into());
    }
    let unit = unit.trim().to_ascii_lowercase();
    let unit = unit.strip_suffix('s').unwrap_or(&unit);
    if !matches!(unit, "second" | "minute" | "hour" | "day") {
        return Err("Use seconds, minutes, hours, or days.".into());
    }
    Ok(())
}

/// A short human description of a cadence.
pub fn describe_when(when: &ScheduleWhen) -> String {
    match when {
        ScheduleWhen::In { in_, .. } => format!("Once in {in_}"),
        ScheduleWhen::At { at, .. } => format!("At {at}"),
        ScheduleWhen::Every { every, .. } => format!("Every {every}"),
        ScheduleWhen::Cron { cron, .. } => format!("Cron {cron}"),
        ScheduleWhen::Other(value) => value.to_string(),
    }
}

/// Relative label for the server-provided `nextDueAt` (epoch milliseconds).
pub fn next_run_label(next_due_at: Option<f64>, now_ms: i64) -> String {
    let Some(due) = next_due_at else {
        return String::new();
    };
    let seconds = (due as i64 - now_ms).max(0) / 1000;
    match seconds {
        0 => "due now".to_string(),
        1..=59 => format!("in {seconds}s"),
        60..=3599 => format!("in {}m", seconds / 60),
        3600..=86_399 => format!("in {}h", seconds / 3600),
        _ => format!("in {}d", seconds / 86_400),
    }
}

/// The failure text to render for a schedule whose last run failed.
/// Plain text, truncated by characters (not bytes) to fit the panel.
pub fn failure_reason(schedule: &ScheduleInfo) -> Option<String> {
    if !schedule
        .last_outcome
        .as_ref()
        .is_some_and(|outcome| outcome.is_failed())
    {
        return None;
    }
    let output = schedule.last_output.as_deref().unwrap_or("").trim();
    if output.is_empty() {
        return Some("The last run failed.".into());
    }
    Some(output.chars().take(FAILURE_REASON_LIMIT).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::{ScheduleOutcome, ScheduleStatus, ScheduleTime};

    fn schedule(id: &str, due: Option<f64>, created: f64, status: ScheduleStatus) -> ScheduleInfo {
        ScheduleInfo {
            id: id.into(),
            status,
            next_due_at: due,
            time: ScheduleTime {
                created,
                updated: created,
            },
            ..ScheduleInfo::default()
        }
    }

    #[test]
    fn schedule_snapshot_replaces_and_sorts() {
        let mut store = ScheduleStore::default();
        store.replace(vec![
            schedule("b", Some(20.0), 1.0, ScheduleStatus::Active),
            schedule("a", Some(10.0), 1.0, ScheduleStatus::Active),
            schedule("c", None, 5.0, ScheduleStatus::Active),
        ]);
        let ids: Vec<&str> = store
            .schedules()
            .iter()
            .map(|schedule| schedule.id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);

        // Earlier creation wins a next-due tie.
        store.replace(vec![
            schedule("late", Some(10.0), 9.0, ScheduleStatus::Active),
            schedule("early", Some(10.0), 2.0, ScheduleStatus::Active),
        ]);
        assert_eq!(store.schedules()[0].id, "early");

        // A snapshot replaces rather than appends.
        store.replace(vec![schedule(
            "only",
            Some(1.0),
            1.0,
            ScheduleStatus::Active,
        )]);
        assert_eq!(store.schedules().len(), 1);
    }

    #[test]
    fn schedule_upsert_replaces_by_id() {
        let mut store = ScheduleStore::default();
        store.replace(vec![schedule("a", Some(1.0), 1.0, ScheduleStatus::Active)]);
        store.upsert(schedule("a", Some(2.0), 1.0, ScheduleStatus::Paused));
        assert_eq!(store.schedules().len(), 1);
        assert_eq!(store.schedules()[0].status, ScheduleStatus::Paused);
    }

    #[test]
    fn schedule_finished_rows_are_hidden_by_default() {
        let mut store = ScheduleStore::default();
        store.replace(vec![
            schedule("a", Some(1.0), 1.0, ScheduleStatus::Done),
            schedule("b", Some(2.0), 1.0, ScheduleStatus::Active),
        ]);
        assert!(store.has_finished());
        let visible: Vec<&str> = store
            .visible(false)
            .map(|schedule| schedule.id.as_str())
            .collect();
        assert_eq!(visible, ["b"]);
        assert_eq!(store.visible(true).count(), 2);
    }

    #[test]
    fn schedule_duration_validation() {
        for valid in ["2 seconds", "1 minute", "90 minutes", "1.5 hours", "3 days"] {
            assert!(validate_duration(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "5", "0 seconds", "-2 minutes", "5 weeks", "abc days"] {
            assert!(validate_duration(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn schedule_failed_outcome_is_truncated() {
        let mut schedule = schedule("a", Some(1.0), 1.0, ScheduleStatus::Active);
        schedule.last_outcome = Some(ScheduleOutcome::Failed);
        schedule.last_output = Some("é".repeat(FAILURE_REASON_LIMIT + 50));
        let reason = failure_reason(&schedule).expect("reason");
        assert_eq!(reason.chars().count(), FAILURE_REASON_LIMIT);
        assert!(reason.is_char_boundary(reason.len()));

        schedule.last_output = Some(String::new());
        assert_eq!(
            failure_reason(&schedule).as_deref(),
            Some("The last run failed.")
        );

        schedule.last_outcome = Some(ScheduleOutcome::Fired);
        assert!(failure_reason(&schedule).is_none());
    }

    #[test]
    fn schedule_next_run_label_handles_future_and_past() {
        assert_eq!(next_run_label(Some(0.0), 0), "due now");
        assert_eq!(next_run_label(Some(30_000.0), 0), "in 30s");
        assert_eq!(next_run_label(Some(120_000.0), 0), "in 2m");
        assert_eq!(next_run_label(None, 0), "");
    }
}
