//! Revert wire types.

use serde::{Deserialize, Serialize};

use crate::diff::FileDiff;

/// Body for `POST /api/session/{id}/revert/stage`.
#[derive(Debug, Clone, Serialize)]
pub struct RevertStageRequest {
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<bool>,
}

/// A staged revert as the server reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionRevert {
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "partID", default)]
    pub part_id: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub files: Vec<FileDiff>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_stage_parses_with_and_without_files() {
        let revert: SessionRevert = serde_json::from_str(
            r#"{"messageID":"m1","partID":"p1","snapshot":"s1","files":[{"file":"a.rs","additions":1,"deletions":1,"status":"modified"}]}"#,
        )
        .expect("revert");
        assert_eq!(revert.files.len(), 1);
        assert_eq!(revert.part_id.as_deref(), Some("p1"));

        let bare: SessionRevert = serde_json::from_str(r#"{"messageID":"m2"}"#).expect("bare");
        assert!(bare.files.is_empty());
        assert!(bare.snapshot.is_none());
    }

    #[test]
    fn revert_stage_serializes_files_false() {
        let body = RevertStageRequest {
            message_id: "m1".into(),
            files: Some(false),
        };
        assert_eq!(
            serde_json::to_string(&body).expect("json"),
            r#"{"messageID":"m1","files":false}"#
        );
    }
}
