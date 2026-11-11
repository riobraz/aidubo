//! Projects returned by `GET /api/project`.
//!
//! The server sends a bare array (no `{data:...}` envelope). A project's
//! `canonical` path is the identity used for location routing; the opaque `id`
//! is what the server uses for updates.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The server's opaque project id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(pub String);

impl ProjectId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProjectId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for ProjectId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One project known to the server.
///
/// Only `id` and `canonical` are required by the client: newer servers may add
/// fields, and an older fixture may omit the optional ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub canonical: String,
    #[serde(default)]
    pub vcs: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The server models this as an object; treat it as uninterpreted metadata.
    #[serde(default)]
    pub icon: Option<serde_json::Value>,
    #[serde(default)]
    pub commands: Option<ProjectCommands>,
    #[serde(default)]
    pub time: Option<ProjectTime>,
    #[serde(default)]
    pub sandboxes: Vec<String>,
}

impl Project {
    /// A local entry for a folder the user added but the server has not seen
    /// yet. The canonical path doubles as the id until a session request makes
    /// the server materialize its own project entry.
    pub fn local(canonical: impl Into<String>) -> Self {
        let canonical = canonical.into();
        Self {
            id: ProjectId(canonical.clone()),
            canonical,
            vcs: None,
            name: None,
            icon: None,
            commands: None,
            time: None,
            sandboxes: Vec::new(),
        }
    }

    /// The label shown in the switcher: an explicit name, else the last path
    /// component, else the canonical path.
    pub fn display_name(&self) -> &str {
        if let Some(name) = self.name.as_deref().filter(|name| !name.trim().is_empty()) {
            return name;
        }
        std::path::Path::new(&self.canonical)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.canonical)
    }
}

/// The workspace startup script the server stores per project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCommands {
    #[serde(default)]
    pub start: Option<String>,
}

/// Timestamps attached to a project, in epoch milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTime {
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub active: i64,
    #[serde(default)]
    pub archived: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_parses_with_only_required_fields() {
        let projects: Vec<Project> = serde_json::from_str(
            r##"[
                {"id":"p1","canonical":"/tmp/one"},
                {
                    "id":"p2",
                    "canonical":"/tmp/two",
                    "vcs":"fossil",
                    "name":"Two",
                    "icon":{"color":"#123456"},
                    "time":{"created":1,"updated":2,"active":3},
                    "sandboxes":["/tmp/two-sandbox"],
                    "futureField":true
                }
            ]"##,
        )
        .expect("projects");
        assert_eq!(projects[0].id.as_str(), "p1");
        assert_eq!(projects[0].canonical, "/tmp/one");
        assert_eq!(projects[0].vcs, None);
        assert_eq!(projects[0].time, None);
        assert!(projects[0].sandboxes.is_empty());
        assert_eq!(projects[0].display_name(), "one");

        assert_eq!(projects[1].vcs.as_deref(), Some("fossil"));
        assert_eq!(projects[1].display_name(), "Two");
        assert_eq!(projects[1].time.expect("time").active, 3);
        assert_eq!(projects[1].sandboxes, ["/tmp/two-sandbox"]);
    }

    #[test]
    fn project_id_uses_the_wire_string_form() {
        let id = ProjectId::from("p1");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""p1""#);
        assert_eq!(serde_json::from_str::<ProjectId>(r#""p1""#).unwrap(), id);
    }
}
