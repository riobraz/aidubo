//! Pending permission requests.

use harness_protocol::PermissionRequest;

/// Pending permission requests for the selected session, oldest first.
///
/// The queue is session-scoped like the transcript: requests that arrive for a
/// session the user has left are dropped, and each request appears once even if
/// both a live event and a snapshot report it.
#[derive(Debug, Default)]
pub struct PermissionQueue {
    session_id: Option<String>,
    requests: Vec<PermissionRequest>,
}

impl PermissionQueue {
    /// Point the queue at a session, clearing it when the selection changes.
    pub fn begin(&mut self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if self.session_id.as_deref() == Some(session_id.as_str()) {
            return;
        }
        self.session_id = Some(session_id);
        self.requests.clear();
    }

    /// Replace the queue with the server's snapshot. Stale responses for a
    /// session the user has since left are ignored.
    pub fn replace(&mut self, session_id: &str, requests: Vec<PermissionRequest>) -> bool {
        if self.session_id.as_deref() != Some(session_id) {
            return false;
        }
        if self.requests == requests {
            return false;
        }
        self.requests = requests;
        true
    }

    /// Add a request, replacing any earlier copy with the same ID.
    pub fn push(&mut self, request: PermissionRequest) -> bool {
        if self.session_id.as_deref() != Some(request.session_id.as_str()) {
            return false;
        }
        match self
            .requests
            .iter_mut()
            .find(|known| known.id == request.id)
        {
            Some(known) => {
                if *known == request {
                    return false;
                }
                *known = request;
            }
            None => self.requests.push(request),
        }
        true
    }

    /// Drop a request once it has been answered.
    pub fn remove(&mut self, request_id: &str) -> bool {
        let before = self.requests.len();
        self.requests.retain(|request| request.id != request_id);
        self.requests.len() != before
    }

    /// The oldest pending request; the one the UI should present.
    pub fn front(&self) -> Option<&PermissionRequest> {
        self.requests.first()
    }

    pub fn contains(&self, request_id: &str) -> bool {
        self.requests.iter().any(|request| request.id == request_id)
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}
