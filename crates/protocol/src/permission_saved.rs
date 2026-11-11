//! Saved permission rules created by "always allow".

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SavedPermission {
    pub id: String,
    #[serde(rename = "projectID", default)]
    pub project_id: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub resource: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_permission_parses_without_time() {
        let rule: SavedPermission = serde_json::from_str(
            r#"{"id":"r1","projectID":"p1","action":"shell","resource":"git *"}"#,
        )
        .expect("rule");
        assert_eq!(rule.action, "shell");
        assert_eq!(rule.resource, "git *");
    }

    #[test]
    fn saved_permission_keeps_long_resource() {
        let long = "x".repeat(200);
        let rule: SavedPermission =
            serde_json::from_str(&format!(r#"{{"id":"r","resource":"{long}"}}"#)).expect("rule");
        assert_eq!(rule.resource.len(), 200);
    }
}
