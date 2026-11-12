//! Prompts queued for or steering the running turn.

use harness_protocol::{InboxDelivery, InboxInfo};

/// One inbox row as the strip renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    pub id: String,
    pub session_id: String,
    pub created: i64,
    pub kind: String,
    pub text: String,
    pub delivery: InboxDelivery,
}

impl From<InboxInfo> for InboxItem {
    fn from(info: InboxInfo) -> Self {
        let text = info.text().to_string();
        Self {
            id: info.id,
            session_id: info.session_id,
            created: info.time.created,
            kind: info.kind,
            text,
            delivery: info.delivery.unwrap_or(InboxDelivery::Queue),
        }
    }
}

/// The selected session's inbox.
#[derive(Debug, Default)]
pub struct InboxStore {
    session_id: Option<String>,
    items: Vec<InboxItem>,
}

impl InboxStore {
    /// Point the store at a session, clearing it when the selection changes.
    pub fn begin(&mut self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if self.session_id.as_deref() == Some(session_id.as_str()) {
            return;
        }
        self.session_id = Some(session_id);
        self.items.clear();
    }

    /// Replace with a server snapshot. A stale session is ignored.
    pub fn replace(&mut self, session_id: &str, items: Vec<InboxInfo>) -> bool {
        if self.session_id.as_deref() != Some(session_id) {
            return false;
        }
        let mut next: Vec<InboxItem> = items.into_iter().map(InboxItem::from).collect();
        next.sort_by_key(|item| item.created);
        if self.items == next {
            return false;
        }
        self.items = next;
        true
    }

    pub fn upsert(&mut self, item: InboxInfo) {
        let item = InboxItem::from(item);
        if self.session_id.as_deref() != Some(item.session_id.as_str()) {
            return;
        }
        match self
            .items
            .iter_mut()
            .find(|existing| existing.id == item.id)
        {
            Some(existing) => *existing = item,
            None => self.items.push(item),
        }
        self.items.sort_by_key(|item| item.created);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.items.len() != before
    }

    pub fn set_delivery(&mut self, id: &str, delivery: InboxDelivery) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return false;
        };
        item.delivery = delivery;
        true
    }

    pub fn items(&self) -> &[InboxItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::InboxTime;

    fn item(id: &str, created: i64, text: &str) -> InboxInfo {
        InboxInfo {
            id: id.into(),
            session_id: "ses_1".into(),
            time: InboxTime { created },
            kind: "user".into(),
            payload: serde_json::json!({ "text": text }),
            delivery: Some(InboxDelivery::Queue),
        }
    }

    #[test]
    fn inbox_enqueue_then_deliver_removes_item() {
        let mut store = InboxStore::default();
        store.begin("ses_1");
        assert!(store.replace("ses_1", vec![item("in_1", 1, "one")]));
        assert_eq!(store.len(), 1);
        assert!(store.remove("in_1"));
        assert!(store.is_empty());
    }

    #[test]
    fn inbox_replayed_events_are_idempotent() {
        let mut store = InboxStore::default();
        store.begin("ses_1");
        store.upsert(item("in_1", 1, "one"));
        store.upsert(item("in_1", 1, "one"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn inbox_snapshot_wins_over_events() {
        let mut store = InboxStore::default();
        store.begin("ses_1");
        store.upsert(item("in_1", 1, "one"));
        assert!(store.replace("ses_1", vec![item("in_2", 2, "two")]));
        assert_eq!(store.items()[0].id, "in_2");
    }

    #[test]
    fn inbox_delivery_toggle_updates_item() {
        let mut store = InboxStore::default();
        store.begin("ses_1");
        store.upsert(item("in_1", 1, "one"));
        assert!(store.set_delivery("in_1", InboxDelivery::Steer));
        assert_eq!(store.items()[0].delivery, InboxDelivery::Steer);
        assert!(!store.set_delivery("missing", InboxDelivery::Queue));
    }

    #[test]
    fn inbox_ignores_other_sessions() {
        let mut store = InboxStore::default();
        store.begin("ses_1");
        let mut other = item("in_9", 1, "other");
        other.session_id = "ses_2".into();
        store.upsert(other);
        assert!(store.is_empty());
        assert!(!store.replace("ses_2", vec![item("in_1", 1, "one")]));
    }
}
