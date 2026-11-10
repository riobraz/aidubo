//! Diff wire types shared by the VCS and session diff endpoints.

use serde::Deserialize;

/// One file's patch as the server reports it.
///
/// `patch` is untrusted text; `more` means the server truncated it. Binary
/// files may carry an empty patch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FileDiff {
    pub file: String,
    #[serde(default)]
    pub patch: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub more: bool,
}

impl FileDiff {
    /// Total changed lines, as the server counts them.
    pub fn changed_lines(&self) -> u64 {
        self.additions + self.deletions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_diff_parses_more_and_counts() {
        let json = r#"{
            "file": "src/main.rs",
            "patch": "@@ -1 +1 @@\n-a\n+b\n",
            "additions": 1,
            "deletions": 1,
            "status": "modified",
            "more": true
        }"#;
        let diff: FileDiff = serde_json::from_str(json).expect("diff");
        assert_eq!(diff.file, "src/main.rs");
        assert_eq!(diff.changed_lines(), 2);
        assert!(diff.more);
    }

    #[test]
    fn file_diff_defaults_missing_patch_and_more() {
        let diff: FileDiff = serde_json::from_str(r#"{"file":"a.png"}"#).expect("minimal diff");
        assert!(diff.patch.is_empty());
        assert!(!diff.more);
        assert_eq!(diff.additions, 0);
    }
}
