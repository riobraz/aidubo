//! Project list selection and per-project session grouping.
//!
//! The store is independent of the server's project registry: a user can add a
//! folder before any session request materializes it on the server.

use std::collections::HashMap;

use harness_protocol::{Project, ProjectId, Session};

/// Projects shown in the switcher plus the active selection.
#[derive(Debug, Default)]
pub struct ProjectStore {
    projects: Vec<Project>,
    active: Option<ProjectId>,
    notice: Option<String>,
}

impl ProjectStore {
    /// Replace the list with a snapshot. The active project survives when its
    /// id is still present; otherwise the first project becomes active and one
    /// notice is queued for the view.
    pub fn replace(&mut self, projects: Vec<Project>) {
        let previous = self.active.clone();
        self.projects = projects;

        let survived = previous
            .as_ref()
            .is_some_and(|id| self.projects.iter().any(|project| &project.id == id));
        if !survived {
            self.active = self.projects.first().map(|project| project.id.clone());
            if previous.is_some() {
                self.notice = Some(match self.active {
                    Some(_) => "The active project is no longer available.".into(),
                    None => "No projects are open.".into(),
                });
            }
        }
    }

    /// Insert a project or replace the entry with the same id.
    pub fn upsert(&mut self, project: Project) {
        if let Some(existing) = self
            .projects
            .iter_mut()
            .find(|existing| existing.id == project.id)
        {
            *existing = project;
        } else {
            self.projects.push(project);
        }
        if self.active.is_none() {
            self.active = self.projects.first().map(|project| project.id.clone());
        }
    }

    /// Add a folder as a local project and return its id. Adding the same
    /// canonical path twice returns the existing id.
    pub fn add(&mut self, canonical: impl Into<String>) -> ProjectId {
        let canonical = canonical.into();
        if let Some(existing) = self
            .projects
            .iter()
            .find(|project| project.canonical == canonical)
        {
            return existing.id.clone();
        }
        let project = Project::local(canonical);
        let id = project.id.clone();
        self.upsert(project);
        id
    }

    pub fn select(&mut self, id: &ProjectId) -> bool {
        if !self.projects.iter().any(|project| &project.id == id) {
            return false;
        }
        self.active = Some(id.clone());
        self.notice = None;
        true
    }

    pub fn active(&self) -> Option<&Project> {
        let id = self.active.as_ref()?;
        self.projects.iter().find(|project| &project.id == id)
    }

    pub fn active_id(&self) -> Option<&ProjectId> {
        self.active.as_ref()
    }

    pub fn active_canonical(&self) -> Option<&str> {
        self.active().map(|project| project.canonical.as_str())
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// Take the one-shot notice produced by a refresh that dropped the active
    /// project.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }
}

/// Sessions belonging to one project, or to the "unknown" bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionGroup {
    /// `None` only for sessions whose `projectID` matches no open project.
    pub project_id: Option<String>,
    pub label: String,
    pub sessions: Vec<Session>,
}

/// Group sessions in project order, with unmatched sessions last rather than
/// dropped. Sessions inside a group are newest first.
pub fn group_sessions(sessions: Vec<Session>, projects: &[Project]) -> Vec<SessionGroup> {
    let mut groups: Vec<SessionGroup> = projects
        .iter()
        .map(|project| SessionGroup {
            project_id: Some(project.id.0.clone()),
            label: project.display_name().to_owned(),
            sessions: Vec::new(),
        })
        .collect();

    let mut unknown = Vec::new();
    for session in sessions {
        let matched = session.project_id.as_deref().and_then(|id| {
            groups
                .iter_mut()
                .find(|group| group.project_id.as_deref() == Some(id))
        });
        match matched {
            Some(group) => group.sessions.push(session),
            None => unknown.push(session),
        }
    }

    for group in &mut groups {
        sort_newest_first(&mut group.sessions);
    }
    if !unknown.is_empty() {
        sort_newest_first(&mut unknown);
        groups.push(SessionGroup {
            project_id: None,
            label: "Unknown project".into(),
            sessions: unknown,
        });
    }
    groups
}

fn sort_newest_first(sessions: &mut [Session]) {
    sessions.sort_by_key(|session| std::cmp::Reverse(session.time.updated));
}

/// Composer state keyed by project and session, so switching projects cannot
/// leak one project's draft into another's session.
#[derive(Debug)]
pub struct ProjectDrafts<T = String> {
    entries: HashMap<(String, String), T>,
}

impl<T> Default for ProjectDrafts<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> ProjectDrafts<T> {
    pub fn get(&self, project_id: &str, session_id: &str) -> Option<&T> {
        self.entries
            .get(&(project_id.to_owned(), session_id.to_owned()))
    }

    pub fn set(&mut self, project_id: &str, session_id: &str, draft: T) {
        self.entries
            .insert((project_id.to_owned(), session_id.to_owned()), draft);
    }

    pub fn take(&mut self, project_id: &str, session_id: &str) -> Option<T> {
        self.entries
            .remove(&(project_id.to_owned(), session_id.to_owned()))
    }

    pub fn remove(&mut self, project_id: &str, session_id: &str) {
        self.entries
            .remove(&(project_id.to_owned(), session_id.to_owned()));
    }

    /// Forget every draft belonging to a project, for when it is closed.
    pub fn remove_project(&mut self, project_id: &str) {
        self.entries.retain(|(project, _), _| project != project_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::SessionTime;

    fn session(id: &str, project: Option<&str>, updated: i64) -> Session {
        Session {
            id: id.into(),
            title: Some(format!("Session {id}")),
            project_id: project.map(str::to_owned),
            agent: None,
            model: None,
            time: SessionTime {
                created: updated,
                updated,
            },
            location: None,
            tokens: None,
            cost: None,
        }
    }

    #[test]
    fn project_active_survives_refresh() {
        let mut store = ProjectStore::default();
        let one = Project::local("/tmp/one");
        let two = Project::local("/tmp/two");
        store.replace(vec![one.clone(), two.clone()]);
        assert_eq!(store.active_canonical(), Some("/tmp/one"));

        assert!(store.select(&two.id));
        store.replace(vec![one.clone(), two.clone()]);
        assert_eq!(store.active_canonical(), Some("/tmp/two"));
        assert_eq!(store.take_notice(), None);

        store.replace(vec![one.clone()]);
        assert_eq!(store.active_canonical(), Some("/tmp/one"));
        assert!(store.take_notice().is_some());
        assert_eq!(store.take_notice(), None, "the notice is one-shot");
    }

    #[test]
    fn project_sessions_group_by_project() {
        let one = Project::local("/tmp/one");
        let two = Project::local("/tmp/two");
        let sessions = vec![
            session("s1", Some(one.id.as_str()), 10),
            session("s2", Some(two.id.as_str()), 20),
            session("s3", Some(one.id.as_str()), 30),
            session("s4", None, 40),
            session("s5", Some("gone"), 50),
        ];

        let groups = group_sessions(sessions, &[one, two]);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].label, "one");
        assert_eq!(
            groups[0]
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["s3", "s1"]
        );
        assert_eq!(
            groups[1]
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["s2"]
        );
        assert_eq!(groups[2].project_id, None);
        assert_eq!(groups[2].label, "Unknown project");
        assert_eq!(
            groups[2]
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["s5", "s4"]
        );
        assert_eq!(
            groups
                .iter()
                .map(|group| group.sessions.len())
                .sum::<usize>(),
            5,
            "no session disappears"
        );
    }

    #[test]
    fn project_drafts_are_per_project() {
        let mut drafts = ProjectDrafts::default();
        drafts.set("p1", "s1", "first project".to_owned());
        drafts.set("p2", "s1", "second project".to_owned());
        assert_eq!(
            drafts.get("p1", "s1").map(String::as_str),
            Some("first project")
        );
        assert_eq!(
            drafts.get("p2", "s1").map(String::as_str),
            Some("second project")
        );
        assert_eq!(drafts.get("p1", "s2"), None);

        drafts.remove_project("p1");
        assert_eq!(drafts.get("p1", "s1"), None);
        assert_eq!(
            drafts.get("p2", "s1").map(String::as_str),
            Some("second project")
        );
    }

    #[test]
    fn add_returns_the_existing_project_for_a_known_path() {
        let mut store = ProjectStore::default();
        let first = store.add("/tmp/one");
        let second = store.add("/tmp/one");
        assert_eq!(first, second);
        assert_eq!(store.len(), 1);
    }
}
