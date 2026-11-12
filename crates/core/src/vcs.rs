//! Working-tree status mirror.

use harness_protocol::{VcsBase, VcsFileStatus};

/// The project's changes as the panel renders them.
#[derive(Debug, Default, Clone)]
pub struct VcsStore {
    files: Vec<VcsFileStatus>,
    base: Option<VcsBase>,
    branch: Option<String>,
    loading: bool,
    error: Option<String>,
    refreshed_at: i64,
}

impl VcsStore {
    /// Replace the snapshot. Removal matters: a cleaned file must disappear.
    pub fn replace(&mut self, files: Vec<VcsFileStatus>) {
        self.files = files;
        self.files.sort_by(|a, b| a.file.cmp(&b.file));
        self.error = None;
    }

    pub fn set_base(&mut self, base: Option<VcsBase>) {
        self.base = base;
    }

    pub fn set_branch(&mut self, branch: impl Into<String>) {
        self.branch = Some(branch.into());
    }

    pub fn begin_refresh(&mut self, now: i64) {
        self.loading = true;
        self.refreshed_at = now;
    }

    pub fn finish_refresh(&mut self) {
        self.loading = false;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.loading = false;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn files(&self) -> &[VcsFileStatus] {
        &self.files
    }

    pub fn base(&self) -> Option<&VcsBase> {
        self.base.as_ref()
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn refreshed_at(&self) -> i64 {
        self.refreshed_at
    }

    pub fn total_changes(&self) -> u64 {
        self.files
            .iter()
            .map(|file| file.additions + file.deletions)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, additions: u64, deletions: u64) -> VcsFileStatus {
        VcsFileStatus {
            file: name.into(),
            additions,
            deletions,
            status: "modified".into(),
        }
    }

    #[test]
    fn vcs_snapshot_replaces_removed_files() {
        let mut store = VcsStore::default();
        store.replace(vec![file("b.rs", 1, 0), file("a.rs", 2, 1)]);
        assert_eq!(store.files()[0].file, "a.rs");
        assert_eq!(store.total_changes(), 4);
        store.replace(vec![file("a.rs", 2, 1)]);
        assert_eq!(store.files().len(), 1);
    }

    #[test]
    fn vcs_branch_event_updates_label_only() {
        let mut store = VcsStore::default();
        store.replace(vec![file("a.rs", 1, 1)]);
        store.set_branch("feature/x");
        assert_eq!(store.branch(), Some("feature/x"));
        assert_eq!(store.files().len(), 1);
    }

    #[test]
    fn vcs_error_clears_loading() {
        let mut store = VcsStore::default();
        store.begin_refresh(10);
        assert!(store.is_loading());
        store.set_error("not a repository");
        assert!(!store.is_loading());
        assert_eq!(store.error(), Some("not a repository"));
    }

    #[test]
    fn vcs_base_is_optional() {
        let mut store = VcsStore::default();
        assert!(store.base().is_none());
        store.set_base(Some(VcsBase {
            name: "main".into(),
            reference: "refs/heads/main".into(),
            source: "default".into(),
        }));
        assert_eq!(store.base().map(|base| base.name.as_str()), Some("main"));
        store.set_base(None);
        assert!(store.base().is_none());
    }
}
