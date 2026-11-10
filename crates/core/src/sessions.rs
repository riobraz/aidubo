//! Session list mirror.

use harness_protocol::{ModelRef, Session};

/// Sessions mirrored from the engine, newest activity first.
///
/// The store owns selection so the UI and tests share one set of rules: a
/// selected session survives a refresh, and removing the selected session falls
/// back to the first remaining session.
#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: Vec<Session>,
    selected: Option<String>,
}

impl SessionStore {
    /// Replace the list with a server snapshot.
    pub fn replace(&mut self, mut sessions: Vec<Session>) {
        sort_newest_first(&mut sessions);
        self.sessions = sessions;

        let selected_survived = self
            .selected
            .as_ref()
            .is_some_and(|id| self.sessions.iter().any(|session| &session.id == id));
        if !selected_survived {
            self.selected = self.sessions.first().map(|session| session.id.clone());
        }
    }

    /// Insert a new session or replace an existing one.
    pub fn upsert(&mut self, session: Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
        sort_newest_first(&mut self.sessions);
    }

    /// Remove a session, selecting the first remaining one if it was selected.
    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|session| session.id != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.sessions.first().map(|session| session.id.clone());
        }
    }

    /// Change a session's title in place. Ordering is left alone.
    pub fn rename(&mut self, id: &str, title: impl Into<String>) {
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.title = Some(title.into());
        }
    }

    /// Record the model the server accepted for a session.
    pub fn set_model(&mut self, id: &str, model: ModelRef) -> bool {
        let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) else {
            return false;
        };
        session.model = Some(model);
        true
    }

    /// Record the agent the server accepted for a session.
    pub fn set_agent(&mut self, id: &str, agent: impl Into<String>) -> bool {
        let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) else {
            return false;
        };
        session.agent = Some(agent.into());
        true
    }

    /// Select a session if it is present.
    pub fn select(&mut self, id: &str) {
        if self.sessions.iter().any(|session| session.id == id) {
            self.selected = Some(id.to_owned());
        }
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn selected(&self) -> Option<&Session> {
        let id = self.selected.as_deref()?;
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

fn sort_newest_first(sessions: &mut [Session]) {
    // Stable: sessions with the same timestamp keep the server's order.
    sessions.sort_by_key(|session| std::cmp::Reverse(session.time.updated));
}
