//! MCP servers.

use std::collections::BTreeMap;

use harness_protocol::{McpServer, McpStatus};

/// Server errors are shown inline; keep them short enough not to flood the
/// settings panel.
const ERROR_LIMIT: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRow {
    pub name: String,
    pub title: Option<String>,
    pub status: McpStatus,
    pub error: Option<String>,
    pub integration_id: Option<String>,
}

impl McpRow {
    pub fn status_label(&self) -> &str {
        self.status.label()
    }

    /// The server needs a sign-in before it can run.
    pub fn can_connect(&self) -> bool {
        matches!(self.status, McpStatus::NeedsAuth)
    }

    /// Only a connected server can be disconnected.
    pub fn can_disconnect(&self) -> bool {
        matches!(self.status, McpStatus::Connected)
    }

    /// Disabled servers come from the user's config and are read-only here.
    pub fn is_read_only(&self) -> bool {
        matches!(self.status, McpStatus::Disabled)
    }
}

impl From<McpServer> for McpRow {
    fn from(server: McpServer) -> Self {
        let error = server
            .error()
            .map(|error| error.chars().take(ERROR_LIMIT).collect());
        Self {
            name: server.name,
            title: server.title,
            status: server.status,
            error,
            integration_id: server.integration_id,
        }
    }
}

/// Servers keyed by name. A reload replaces the whole map, so a server that
/// disappeared on the sidecar cannot linger in the UI.
#[derive(Debug, Default)]
pub struct McpStore {
    rows: BTreeMap<String, McpRow>,
}

impl McpStore {
    pub fn replace(&mut self, servers: Vec<McpServer>) {
        self.rows = servers
            .into_iter()
            .map(|server| (server.name.clone(), McpRow::from(server)))
            .collect();
    }

    /// Update one server without assuming the rest of the list is fresh.
    pub fn upsert(&mut self, server: McpServer) {
        self.rows.insert(server.name.clone(), McpRow::from(server));
    }

    pub fn remove(&mut self, name: &str) -> Option<McpRow> {
        self.rows.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&McpRow> {
        self.rows.get(name)
    }

    pub fn rows(&self) -> impl Iterator<Item = &McpRow> {
        self.rows.values()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, status: &str) -> McpServer {
        serde_json::from_str(&format!(
            r#"{{"name":"{name}","status":{{"status":"{status}"}}}}"#
        ))
        .expect("server")
    }

    #[test]
    fn mcp_snapshot_replaces_and_keeps_errors() {
        let mut store = McpStore::default();
        let error = "x".repeat(ERROR_LIMIT + 100);
        let failed: McpServer = serde_json::from_str(&format!(
            r#"{{"name":"a","status":{{"status":"failed","error":"{error}"}}}}"#
        ))
        .expect("server");
        store.replace(vec![server("b", "connected"), failed]);
        let names: Vec<_> = store.rows().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        let row = store.get("a").expect("failed row");
        assert_eq!(row.status_label(), "failed");
        assert_eq!(row.error.as_deref().map(str::len), Some(ERROR_LIMIT));

        store.replace(vec![server("a", "connected")]);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("a").expect("row").status, McpStatus::Connected);
        assert!(store.get("b").is_none());
    }

    #[test]
    fn mcp_add_then_reload_is_authoritative() {
        let mut store = McpStore::default();
        store.replace(vec![server("a", "pending")]);

        // Adding a server does not insert optimistically; the next snapshot is
        // the only source of truth.
        store.upsert(server("a", "connected"));
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("a").expect("row").status, McpStatus::Connected);

        store.upsert(server("b", "pending"));
        assert_eq!(store.len(), 2);
        store.remove("a");
        assert!(store.get("a").is_none());
        assert_eq!(store.len(), 1);

        store.replace(vec![server("c", "disabled")]);
        let names: Vec<_> = store.rows().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["c"]);
    }

    #[test]
    fn mcp_actions_follow_status() {
        let mut store = McpStore::default();
        store.replace(vec![
            server("connected", "connected"),
            server("pending", "pending"),
            server("disabled", "disabled"),
            server("auth", "needs_auth"),
            server("failed", "failed"),
        ]);

        let connected = store.get("connected").expect("connected");
        assert!(connected.can_disconnect());
        assert!(!connected.can_connect());
        assert!(store.get("auth").expect("auth").can_connect());
        assert!(!store.get("auth").expect("auth").can_disconnect());
        assert!(store.get("disabled").expect("disabled").is_read_only());
        assert!(!store.get("pending").expect("pending").can_connect());
        assert!(!store.get("pending").expect("pending").can_disconnect());
        assert!(!store.get("failed").expect("failed").can_connect());
        assert!(!store.get("failed").expect("failed").can_disconnect());
    }
}
