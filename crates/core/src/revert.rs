//! A staged revert, before it is committed.

use harness_protocol::{FileDiff, SessionRevert};

/// The revert the user is previewing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRevert {
    pub session_id: String,
    pub message_id: String,
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Default)]
pub struct RevertStore {
    staged: Option<StagedRevert>,
    committed: Option<String>,
}

impl RevertStore {
    pub fn stage(&mut self, session_id: &str, revert: SessionRevert) {
        self.staged = Some(StagedRevert {
            session_id: session_id.to_string(),
            message_id: revert.message_id,
            files: revert.files,
        });
    }

    pub fn staged(&self) -> Option<&StagedRevert> {
        self.staged.as_ref()
    }

    pub fn is_staged(&self) -> bool {
        self.staged.is_some()
    }

    pub fn clear(&mut self) {
        self.staged = None;
    }

    /// Mark the revert committed and return the boundary message.
    pub fn commit(&mut self) -> Option<String> {
        let staged = self.staged.take()?;
        self.committed = Some(staged.message_id.clone());
        Some(staged.message_id)
    }

    pub fn committed(&self) -> Option<&str> {
        self.committed.as_deref()
    }

    pub fn acknowledge(&mut self) {
        self.committed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revert(message_id: &str) -> SessionRevert {
        SessionRevert {
            message_id: message_id.into(),
            part_id: None,
            snapshot: None,
            files: Vec::new(),
        }
    }

    #[test]
    fn revert_stage_replaces_previous() {
        let mut store = RevertStore::default();
        store.stage("s1", revert("m1"));
        store.stage("s1", revert("m2"));
        assert_eq!(
            store.staged().map(|staged| staged.message_id.as_str()),
            Some("m2")
        );
    }

    #[test]
    fn revert_commit_without_stage_is_ignored() {
        let mut store = RevertStore::default();
        assert!(store.commit().is_none());
        assert!(store.committed().is_none());
    }

    #[test]
    fn revert_commit_returns_boundary_and_clears_stage() {
        let mut store = RevertStore::default();
        store.stage("s1", revert("m1"));
        assert_eq!(store.commit().as_deref(), Some("m1"));
        assert!(!store.is_staged());
        assert_eq!(store.committed(), Some("m1"));
        store.acknowledge();
        assert!(store.committed().is_none());
    }
}
