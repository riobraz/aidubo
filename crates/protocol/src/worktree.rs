//! Worktree entries and requests for `/api/worktree`.
//!
//! `GET /api/worktree` returns a bare array; the create response is an
//! `Worktree.Info` with only `directory`. `strategy` is optional, so a newer
//! server (or an older fixture) cannot break parsing.

use serde::{Deserialize, Serialize};

/// One worktree checkout known to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub directory: String,
    #[serde(default)]
    pub strategy: Option<String>,
}

/// Request body for `POST /api/worktree`.
///
/// The server places the worktree inside its own worktree root and checks out a
/// detached copy of the base ref; it does not create a branch. `directory` is
/// omitted so the server chooses the path, and `branch` is omitted when the
/// base should be the current `HEAD` of `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorktreeRequest {
    #[serde(rename = "projectID")]
    pub project_id: String,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    pub name: String,
}

/// Request body for `DELETE /api/worktree`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorktreeRequest {
    #[serde(rename = "projectID")]
    pub project_id: String,
    pub directory: String,
    pub force: bool,
}

/// Request body for `POST /api/session/{sessionID}/move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSessionRequest {
    pub directory: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_list_parses_strategy_optional() {
        let worktrees: Vec<Worktree> = serde_json::from_str(
            r#"[
                {"directory":"/tmp/project-wt"},
                {"directory":"/tmp/other","strategy":"checkout","future":true}
            ]"#,
        )
        .expect("worktrees");
        assert_eq!(worktrees[0].directory, "/tmp/project-wt");
        assert_eq!(worktrees[0].strategy, None);
        assert_eq!(worktrees[1].strategy.as_deref(), Some("checkout"));
    }

    #[test]
    fn create_body_uses_server_field_names() {
        let request = CreateWorktreeRequest {
            project_id: "p1".into(),
            from: "/tmp/project".into(),
            branch: Some("feature".into()),
            directory: None,
            name: "feature".into(),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "projectID": "p1",
                "from": "/tmp/project",
                "branch": "feature",
                "name": "feature"
            })
        );

        let head = CreateWorktreeRequest {
            branch: None,
            ..request
        };
        assert_eq!(
            serde_json::to_value(&head).unwrap(),
            serde_json::json!({
                "projectID": "p1",
                "from": "/tmp/project",
                "name": "feature"
            })
        );

        let worktree: Worktree =
            serde_json::from_str(r#"{"directory":"/tmp/project-wt"}"#).expect("create response");
        assert_eq!(worktree.directory, "/tmp/project-wt");
        assert_eq!(worktree.strategy, None);
    }

    #[test]
    fn remove_body_uses_server_field_names() {
        let request = RemoveWorktreeRequest {
            project_id: "p1".into(),
            directory: "/tmp/project-wt".into(),
            force: true,
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "projectID": "p1",
                "directory": "/tmp/project-wt",
                "force": true
            })
        );
    }

    #[test]
    fn move_body_carries_only_the_directory() {
        let request = MoveSessionRequest {
            directory: "/tmp/project-wt".into(),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"directory": "/tmp/project-wt"})
        );
    }
}
