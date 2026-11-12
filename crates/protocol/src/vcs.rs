//! Git status and base wire types.

use serde::Deserialize;

/// One file in the working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct VcsFileStatus {
    pub file: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub status: String,
}

/// The ref the working tree is compared against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct VcsBase {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "ref", default)]
    pub reference: String,
    #[serde(default)]
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_status_parses_counts() {
        let file: VcsFileStatus = serde_json::from_str(
            r#"{"file":"a.rs","additions":2,"deletions":1,"status":"modified"}"#,
        )
        .expect("status");
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 1);
    }

    #[test]
    fn vcs_base_parses_ref_keyword() {
        let base: VcsBase =
            serde_json::from_str(r#"{"name":"main","ref":"refs/heads/main","source":"default"}"#)
                .expect("base");
        assert_eq!(base.name, "main");
        assert_eq!(base.reference, "refs/heads/main");
        assert_eq!(base.source, "default");
    }

    #[test]
    fn vcs_base_tolerates_missing_fields() {
        let base: VcsBase = serde_json::from_str(r#"{"name":"main"}"#).expect("base");
        assert_eq!(base.name, "main");
        assert!(base.reference.is_empty());
    }
}
