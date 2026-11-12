//! Project list and active selection persisted in the app-data directory.
//!
//! The file holds canonical paths and the active path. A folder that is gone
//! at launch is dropped with a notice; nothing is ever written inside a
//! project directory.

use std::path::{Path, PathBuf};

use harness_views::ProjectSnapshot;
use serde::{Deserialize, Serialize};

pub const PROJECTS_ENV: &str = "RUST_HARNESS_PROJECTS";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredProjects {
    projects: Vec<String>,
    active: Option<String>,
}

/// What the workspace should start with after dropping unavailable folders.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RestoredProjects {
    pub projects: Vec<String>,
    pub active: Option<String>,
    pub notice: Option<String>,
}

/// The test override, then the same per-platform locations notifications use.
pub fn projects_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(PROJECTS_ENV) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        home.map(|home| home.join("Library/Application Support/Rust Harness/projects.json"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(config).join("rust-harness/projects.json"));
        }
        home.map(|home| home.join(".config/rust-harness/projects.json"))
    }
}

/// Read the app-data file and keep only folders that still exist. A missing or
/// invalid file yields an empty list rather than blocking startup.
pub fn restore() -> RestoredProjects {
    let Some(path) = projects_path() else {
        return RestoredProjects::default();
    };
    restore_from(&path)
}

fn restore_from(path: &Path) -> RestoredProjects {
    let stored = load_from(path);
    let saved_active = stored.active;
    let mut restored = RestoredProjects::default();
    let mut dropped = 0usize;

    for saved in stored.projects {
        match std::fs::canonicalize(&saved) {
            Ok(canonical) if canonical.is_dir() => {
                let canonical = canonical.to_string_lossy().into_owned();
                if !restored.projects.contains(&canonical) {
                    restored.projects.push(canonical.clone());
                }
                if saved_active.as_deref() == Some(saved.as_str()) {
                    restored.active = Some(canonical);
                }
            }
            _ => dropped += 1,
        }
    }

    if restored.active.is_none() {
        restored.active = restored.projects.first().cloned();
    }
    restored.notice = match dropped {
        0 => None,
        1 => Some("One saved project folder could not be found and was removed.".into()),
        count => Some(format!(
            "{count} saved project folders could not be found and were removed."
        )),
    };
    restored
}

fn load_from(path: &Path) -> StoredProjects {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| {
            tracing::warn!("ignoring an invalid project list file");
            StoredProjects::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredProjects::default(),
        Err(_) => {
            tracing::warn!("could not read the project list file");
            StoredProjects::default()
        }
    }
}

/// Replace the app-data file with the current list and active path.
pub fn save(snapshot: &ProjectSnapshot) -> Result<(), String> {
    let Some(path) = projects_path() else {
        return Err("no app-data directory is available".to_string());
    };
    save_to(&path, snapshot)
}

fn save_to(path: &Path, snapshot: &ProjectSnapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "could not create the app-data directory".to_string())?;
    }
    let stored = StoredProjects {
        projects: snapshot.projects.clone(),
        active: snapshot.active.clone(),
    };
    let raw = serde_json::to_vec_pretty(&stored)
        .map_err(|_| "could not encode the project list".to_string())?;
    std::fs::write(path, raw).map_err(|_| "could not write the project list".to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rust-harness-persistence-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn canonical(&self) -> String {
            std::fs::canonicalize(&self.path)
                .expect("canonical temp dir")
                .to_string_lossy()
                .into_owned()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn project_list_round_trips_with_active() {
        let data = TempDir::new("round-trip");
        let first = TempDir::new("first");
        let second = TempDir::new("second");
        let file = data.path.join("projects.json");
        let snapshot = ProjectSnapshot {
            projects: vec![first.canonical(), second.canonical()],
            active: Some(second.canonical()),
        };

        save_to(&file, &snapshot).expect("save project list");
        let restored = restore_from(&file);

        assert_eq!(restored.projects, snapshot.projects);
        assert_eq!(restored.active, snapshot.active);
        assert_eq!(restored.notice, None);
    }

    #[test]
    fn missing_project_is_dropped_with_a_notice() {
        let data = TempDir::new("missing");
        let existing = TempDir::new("existing");
        let file = data.path.join("projects.json");
        let missing = data.path.join("gone").to_string_lossy().into_owned();
        let snapshot = ProjectSnapshot {
            projects: vec![existing.canonical(), missing],
            active: None,
        };

        save_to(&file, &snapshot).expect("save project list");
        let restored = restore_from(&file);

        assert_eq!(restored.projects, vec![existing.canonical()]);
        assert_eq!(restored.active, Some(existing.canonical()));
        assert!(
            restored
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("could not be found")),
            "{:?}",
            restored.notice
        );
    }

    #[test]
    fn active_falls_back_when_it_is_missing() {
        let data = TempDir::new("active");
        let first = TempDir::new("active-first");
        let file = data.path.join("projects.json");
        let snapshot = ProjectSnapshot {
            projects: vec![first.canonical()],
            active: Some(data.path.join("gone").to_string_lossy().into_owned()),
        };

        save_to(&file, &snapshot).expect("save project list");
        let restored = restore_from(&file);

        assert_eq!(restored.active, Some(first.canonical()));
    }

    #[test]
    fn missing_file_starts_empty() {
        let data = TempDir::new("absent");
        let restored = restore_from(&data.path.join("projects.json"));

        assert_eq!(restored, RestoredProjects::default());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_saved_path_restores_as_canonical() {
        let data = TempDir::new("symlink");
        let project = TempDir::new("symlink-target");
        let link = data.path.join("link");
        std::os::unix::fs::symlink(&project.path, &link).expect("create symlink");
        let file = data.path.join("projects.json");
        let snapshot = ProjectSnapshot {
            projects: vec![link.to_string_lossy().into_owned()],
            active: None,
        };

        save_to(&file, &snapshot).expect("save project list");
        let restored = restore_from(&file);

        assert_eq!(restored.projects, vec![project.canonical()]);
        assert_eq!(restored.active, Some(project.canonical()));
    }
}
