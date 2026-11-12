//! The single live goal, persisted in the app-data directory.
//!
//! Only a running or interrupted goal is written. A goal stopped by a cap, a
//! blocker, or the user is cleared, so a relaunch never shows a stale loop
//! bar. Nothing is ever written inside a project directory.

use std::path::{Path, PathBuf};

use harness_core::GoalRecord;
use harness_protocol::ModelRef;

pub const GOALS_ENV: &str = "RUST_HARNESS_GOALS";

/// The test override, then the same app-data directory the project list uses.
pub fn goals_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(GOALS_ENV) {
        return Some(PathBuf::from(path));
    }
    crate::persistence::projects_path().map(|path| path.with_file_name("goals.json"))
}

/// Read the saved goal. A missing or invalid file simply means no goal.
pub fn restore() -> Option<GoalRecord> {
    let path = goals_path()?;
    restore_from(&path)
}

fn restore_from(path: &Path) -> Option<GoalRecord> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            tracing::warn!("could not read the goal state file");
            return None;
        }
    };
    match serde_json::from_str(&raw) {
        Ok(record) => Some(record),
        Err(_) => {
            tracing::warn!("ignoring an invalid goal state file");
            None
        }
    }
}

/// Replace (or remove) the saved goal.
pub fn save(record: Option<&GoalRecord>) -> Result<(), String> {
    let Some(path) = goals_path() else {
        return Ok(());
    };
    save_to(&path, record)
}

fn save_to(path: &Path, record: Option<&GoalRecord>) -> Result<(), String> {
    let Some(record) = record else {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not clear the saved goal.".into()),
        };
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Could not create the app-data directory.".to_string())?;
    }
    let raw = serde_json::to_string(record)
        .map_err(|_| "Could not encode the goal state.".to_string())?;
    std::fs::write(path, raw).map_err(|_| "Could not save the goal state.".to_string())
}

/// Parse `provider/modelID`, or a bare model id.
pub fn parse_auditor_model(raw: &str) -> Option<ModelRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(match raw.split_once('/') {
        Some((provider, model)) if !provider.is_empty() && !model.is_empty() => ModelRef {
            id: model.to_owned(),
            provider_id: Some(provider.to_owned()),
            variant: None,
        },
        _ => ModelRef {
            id: raw.to_owned(),
            provider_id: None,
            variant: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use harness_core::{Goal, GoalLimits, GoalState};

    use super::*;

    fn record() -> GoalRecord {
        GoalRecord {
            directory: "/tmp/project".into(),
            goal: Goal {
                id: "goal-1".into(),
                session_id: "session-1".into(),
                objective: "finish the milestone".into(),
                state: GoalState::Running,
                continuations: 2,
                tokens_used: 1_500,
                started_at: 1_000,
                auditor_session_id: Some("auditor-1".into()),
            },
            limits: GoalLimits::default(),
        }
    }

    #[test]
    fn goal_record_round_trips_and_can_be_cleared() {
        let path = std::env::temp_dir().join(format!(
            "rust-harness-goal-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        save_to(&path, Some(&record())).expect("save");
        assert_eq!(restore_from(&path), Some(record()));
        save_to(&path, None).expect("clear");
        assert_eq!(restore_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn auditor_model_parses_an_optional_provider() {
        let parsed = parse_auditor_model("mock/mock-model").expect("model");
        assert_eq!(parsed.id, "mock-model");
        assert_eq!(parsed.provider_id.as_deref(), Some("mock"));
        let bare = parse_auditor_model("some-model").expect("model");
        assert_eq!(bare.id, "some-model");
        assert_eq!(bare.provider_id, None);
        assert_eq!(parse_auditor_model("  "), None);
    }
}
