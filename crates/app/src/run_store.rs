//! Run groups, persisted in the app-data directory per project.
//!
//! Groups are app-local: the server has sessions and worktrees, not groups.
//! The file keeps the last known state of every project so a relaunch can show
//! what was abandoned instead of silently resuming it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harness_core::RunGroup;
use serde::{Deserialize, Serialize};

pub const RUNS_ENV: &str = "RUST_HARNESS_RUNS";

/// The test override, then the same app-data directory the project list uses.
pub fn runs_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(RUNS_ENV) {
        return Some(PathBuf::from(path));
    }
    crate::persistence::projects_path().map(|path| path.with_file_name("runs.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SavedRuns {
    /// Groups per canonical project directory.
    #[serde(default)]
    projects: BTreeMap<String, Vec<RunGroup>>,
}

/// The startup project's saved groups. A missing or invalid file means none.
pub fn restore_for(directory: &str) -> Vec<RunGroup> {
    let Some(path) = runs_path() else {
        return Vec::new();
    };
    restore_from(&path, directory)
}

fn restore_from(path: &Path, directory: &str) -> Vec<RunGroup> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => {
            tracing::warn!("could not read the run state file");
            return Vec::new();
        }
    };
    let saved: SavedRuns = match serde_json::from_str(&raw) {
        Ok(saved) => saved,
        Err(_) => {
            tracing::warn!("ignoring an invalid run state file");
            return Vec::new();
        }
    };
    saved.projects.get(directory).cloned().unwrap_or_default()
}

/// Replace one project's groups. An empty list removes the entry.
pub fn save(directory: &str, groups: &[RunGroup]) -> Result<(), String> {
    let Some(path) = runs_path() else {
        return Ok(());
    };
    save_to(&path, directory, groups)
}

fn save_to(path: &Path, directory: &str, groups: &[RunGroup]) -> Result<(), String> {
    let mut saved = match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<SavedRuns>(&raw).unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SavedRuns::default(),
        Err(_) => SavedRuns::default(),
    };
    if groups.is_empty() {
        saved.projects.remove(directory);
    } else {
        saved
            .projects
            .insert(directory.to_string(), groups.to_vec());
    }
    if saved.projects.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not clear the saved runs.".into()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Could not create the app-data directory.".to_string())?;
    }
    let raw =
        serde_json::to_string(&saved).map_err(|_| "Could not encode the run state.".to_string())?;
    std::fs::write(path, raw).map_err(|_| "Could not save the run state.".to_string())
}

#[cfg(test)]
mod tests {
    use harness_core::RunState;
    use harness_protocol::ModelRef;

    use super::*;

    fn group(id: &str) -> RunGroup {
        RunGroup {
            id: id.to_string(),
            prompt: "Refactor the parser".into(),
            created: 1_700_000_000_000,
            runs: vec![harness_core::Run {
                id: "run-1".into(),
                model: ModelRef {
                    id: "model-a".into(),
                    provider_id: Some("mock".into()),
                    variant: None,
                },
                session_id: Some("ses-1".into()),
                worktree: Some("/tmp/wt-1".into()),
                state: RunState::Done,
            }],
            kept: Some("run-1".into()),
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rust-harness-runs-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn run_groups_round_trip_per_project() {
        let path = temp_path("roundtrip");
        save_to(&path, "/tmp/a", &[group("group-a")]).expect("save a");
        save_to(&path, "/tmp/b", &[group("group-b")]).expect("save b");

        assert_eq!(restore_from(&path, "/tmp/a"), vec![group("group-a")]);
        assert_eq!(restore_from(&path, "/tmp/b"), vec![group("group-b")]);
        assert!(restore_from(&path, "/tmp/c").is_empty());

        save_to(&path, "/tmp/a", &[]).expect("clear a");
        assert!(restore_from(&path, "/tmp/a").is_empty());
        assert_eq!(restore_from(&path, "/tmp/b"), vec![group("group-b")]);

        save_to(&path, "/tmp/b", &[]).expect("clear b");
        assert!(!path.exists(), "an empty store removes the file");
    }

    #[test]
    fn invalid_files_restore_as_empty() {
        let path = temp_path("invalid");
        std::fs::write(&path, "not json").expect("write");
        assert!(restore_from(&path, "/tmp/a").is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
