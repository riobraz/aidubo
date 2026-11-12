//! Worktree list, app-created registry, and session grouping.
//!
//! The store is pure: the caller supplies the server snapshot and the local
//! filesystem predicate. A worktree the app created but the server no longer
//! lists, yet which still exists on disk, is an orphan and must be removable
//! from the UI instead of silently leaking disk.

use harness_protocol::{Session, Worktree};

/// One row shown in the worktree picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub directory: String,
    pub strategy: Option<String>,
    /// The app created this worktree, so it owns the cleanup responsibility.
    pub created_by_app: bool,
    /// On disk but absent from the server's list.
    pub orphan: bool,
}

impl WorktreeEntry {
    /// The last path component, for a compact row label.
    pub fn label(&self) -> &str {
        std::path::Path::new(&self.directory)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.directory)
    }
}

/// Worktrees for the active project plus the paths this app created.
#[derive(Debug, Default)]
pub struct WorktreeStore {
    directory: Option<String>,
    entries: Vec<WorktreeEntry>,
    created_by_app: Vec<String>,
    loading: bool,
}

impl WorktreeStore {
    /// Point the store at a project directory. Switching projects clears the
    /// rows but keeps the app-owned registry, which is global to the app-data
    /// file.
    pub fn begin(&mut self, directory: impl Into<String>) -> bool {
        let directory = directory.into();
        if self.directory.as_deref() == Some(directory.as_str()) {
            return false;
        }
        self.directory = Some(directory);
        self.entries.clear();
        self.loading = true;
        true
    }

    /// Replace the app-owned registry from persisted state.
    pub fn set_registry(&mut self, registry: Vec<String>) {
        self.created_by_app = dedupe(registry);
    }

    pub fn created_by_app(&self) -> &[String] {
        &self.created_by_app
    }

    /// Apply the server snapshot for `directory`.
    ///
    /// Registry paths that exist on disk but are missing from the snapshot are
    /// appended as orphans; registry paths that no longer exist are dropped so
    /// the file cannot grow forever. Returns false for a snapshot belonging to
    /// a project the user has since left.
    pub fn apply_snapshot(
        &mut self,
        directory: &str,
        server: Vec<Worktree>,
        exists: impl Fn(&str) -> bool,
    ) -> bool {
        if self
            .directory
            .as_deref()
            .is_some_and(|current| current != directory)
        {
            return false;
        }
        self.directory = Some(directory.to_owned());
        self.loading = false;

        let project = directory.trim_end_matches('/');
        let mut entries: Vec<WorktreeEntry> = server
            .into_iter()
            // The server lists the project itself alongside its worktrees.
            .filter(|worktree| worktree.directory.trim_end_matches('/') != project)
            .map(|worktree| WorktreeEntry {
                created_by_app: self.created_by_app.contains(&worktree.directory),
                orphan: false,
                directory: worktree.directory,
                strategy: worktree.strategy,
            })
            .collect();

        let registry = std::mem::take(&mut self.created_by_app);
        let mut kept = Vec::with_capacity(registry.len());
        for path in registry {
            if entries.iter().any(|entry| entry.directory == path) {
                kept.push(path);
            } else if path.trim_end_matches('/') != project && exists(&path) {
                entries.push(WorktreeEntry {
                    directory: path.clone(),
                    strategy: None,
                    created_by_app: true,
                    orphan: true,
                });
                kept.push(path);
            }
        }
        self.created_by_app = kept;
        self.entries = entries;
        true
    }

    /// Record a worktree the server just created for this app.
    pub fn track_created(&mut self, worktree: Worktree) -> bool {
        if !self.created_by_app.contains(&worktree.directory) {
            self.created_by_app.push(worktree.directory.clone());
        }
        self.upsert(worktree);
        true
    }

    /// Insert or replace one row.
    pub fn upsert(&mut self, worktree: Worktree) {
        let created_by_app = self.created_by_app.contains(&worktree.directory);
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.directory == worktree.directory)
        {
            Some(entry) => {
                entry.strategy = worktree.strategy;
                entry.orphan = false;
                entry.created_by_app = created_by_app;
            }
            None => self.entries.push(WorktreeEntry {
                directory: worktree.directory,
                strategy: worktree.strategy,
                created_by_app,
                orphan: false,
            }),
        }
    }

    /// Drop one row and forget the app-owned path.
    pub fn remove(&mut self, directory: &str) {
        self.entries.retain(|entry| entry.directory != directory);
        self.created_by_app.retain(|path| path != directory);
    }

    pub fn entries(&self) -> &[WorktreeEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }
}

/// Sessions whose location is `directory`, as ids for removal and counts.
pub fn sessions_in_directory<'a>(sessions: &'a [Session], directory: &str) -> Vec<&'a str> {
    let directory = directory.trim_end_matches('/');
    sessions
        .iter()
        .filter(|session| {
            session
                .location
                .as_ref()
                .map(|location| location.directory.trim_end_matches('/') == directory)
                .unwrap_or(false)
        })
        .map(|session| session.id.as_str())
        .collect()
}

/// Sessions grouped by the directory they run in, with the project root first
/// and the known worktrees after it. Directories with no sessions are kept so
/// the sidebar can show an empty worktree; unknown directories are appended
/// rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDirectoryGroup {
    pub directory: String,
    pub session_ids: Vec<String>,
}

pub fn group_sessions_by_directory(
    sessions: &[Session],
    project_dir: &str,
    worktrees: &[String],
) -> Vec<SessionDirectoryGroup> {
    let mut directories = vec![project_dir.trim_end_matches('/').to_owned()];
    for worktree in worktrees {
        let worktree = worktree.trim_end_matches('/').to_owned();
        if !directories.contains(&worktree) {
            directories.push(worktree);
        }
    }
    for session in sessions {
        if let Some(location) = &session.location {
            let directory = location.directory.trim_end_matches('/').to_owned();
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        }
    }

    directories
        .into_iter()
        .map(|directory| SessionDirectoryGroup {
            session_ids: sessions_in_directory(sessions, &directory)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            directory,
        })
        .collect()
}

/// Local guard for the base ref sent to the worktree endpoint. An empty ref is
/// valid and means `HEAD`. The server still validates; this only catches the
/// mistakes that are cheap to explain before a request.
pub fn validate_base_ref(base_ref: &str) -> Result<(), String> {
    let base_ref = base_ref.trim();
    if base_ref.is_empty() {
        return Ok(());
    }
    if base_ref.chars().any(char::is_whitespace) {
        return Err("A branch or ref cannot contain spaces.".into());
    }
    if base_ref.starts_with('-') {
        return Err("A branch or ref cannot start with '-'.".into());
    }
    if base_ref.contains("..") {
        return Err("A branch or ref cannot contain '..'.".into());
    }
    if base_ref.contains(['~', '^', ':', '?', '*', '[', '\\']) {
        return Err("A branch or ref contains a character git does not allow.".into());
    }
    Ok(())
}

fn dedupe(paths: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::{SessionLocation, SessionTime};

    fn session(id: &str, directory: Option<&str>) -> Session {
        Session {
            id: id.into(),
            title: None,
            project_id: None,
            agent: None,
            model: None,
            time: SessionTime::default(),
            location: directory.map(|directory| SessionLocation {
                directory: directory.into(),
            }),
            tokens: None,
            cost: None,
        }
    }

    fn worktree(directory: &str) -> Worktree {
        Worktree {
            directory: directory.into(),
            strategy: None,
        }
    }

    #[test]
    fn worktree_orphan_detected_on_launch() {
        let mut store = WorktreeStore::default();
        store.set_registry(vec![
            "/w/known".into(),
            "/w/orphan".into(),
            "/w/gone".into(),
        ]);
        assert!(
            store.apply_snapshot("/project", vec![worktree("/w/known")], |path| {
                path != "/w/gone"
            })
        );

        let entries = store.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].directory, "/w/known");
        assert!(entries[0].created_by_app);
        assert!(!entries[0].orphan);
        assert_eq!(entries[1].directory, "/w/orphan");
        assert!(entries[1].orphan);
        assert_eq!(store.created_by_app(), ["/w/known", "/w/orphan"]);
    }

    #[test]
    fn worktree_remove_drops_sessions() {
        let sessions = vec![
            session("s1", Some("/project/")),
            session("s2", Some("/w/feature")),
            session("s3", None),
            session("s4", Some("/w/other")),
        ];
        assert_eq!(sessions_in_directory(&sessions, "/w/feature"), ["s2"]);
        assert_eq!(sessions_in_directory(&sessions, "/project"), ["s1"]);

        let mut store = WorktreeStore::default();
        store.set_registry(vec!["/w/feature".into()]);
        store.apply_snapshot("/project", vec![worktree("/w/feature")], |_| true);
        store.remove("/w/feature");
        assert!(store.is_empty());
        assert!(store.created_by_app().is_empty());
    }

    #[test]
    fn base_ref_validation() {
        for valid in ["", "feature", "fix/sse-reconnect", "release-1.2"] {
            assert!(validate_base_ref(valid).is_ok(), "{valid}");
        }
        for invalid in ["two words", "-flag", "a..b", "a~b", "a:b", "a?b"] {
            assert!(validate_base_ref(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sessions_group_by_worktree_directory() {
        let sessions = vec![
            session("root", Some("/project")),
            session("wt", Some("/w/feature")),
            session("unknown", Some("/elsewhere")),
            session("none", None),
        ];
        let groups = group_sessions_by_directory(&sessions, "/project", &["/w/feature".into()]);
        assert_eq!(
            groups
                .iter()
                .map(|group| (group.directory.as_str(), group.session_ids.clone()))
                .collect::<Vec<_>>(),
            [
                ("/project", vec!["root".to_string()]),
                ("/w/feature", vec!["wt".to_string()]),
                ("/elsewhere", vec!["unknown".to_string()]),
            ]
        );
    }

    #[test]
    fn stale_project_snapshot_is_ignored() {
        let mut store = WorktreeStore::default();
        store.begin("/project");
        assert!(!store.apply_snapshot("/other", vec![worktree("/w")], |_| true));
        assert!(store.is_empty());
    }

    #[test]
    fn project_root_is_not_a_worktree_row() {
        let mut store = WorktreeStore::default();
        store.begin("/project");
        assert!(store.apply_snapshot(
            "/project",
            vec![worktree("/project"), worktree("/w/feature")],
            |_| true
        ));
        assert_eq!(store.len(), 1);
        assert_eq!(store.entries()[0].directory, "/w/feature");
    }
}
