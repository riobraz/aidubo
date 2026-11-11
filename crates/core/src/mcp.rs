//! MCP servers.

use harness_protocol::McpServer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRow {
    pub name: String,
    pub title: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

impl From<McpServer> for McpRow {
    fn from(server: McpServer) -> Self {
        let error = server
            .error()
            .map(|error| error.chars().take(400).collect());
        let status = server.status_label().to_string();
        Self {
            name: server.name,
            title: server.title,
            status,
            error,
        }
    }
}

#[derive(Debug, Default)]
pub struct McpStore {
    rows: Vec<McpRow>,
}

impl McpStore {
    pub fn replace(&mut self, servers: Vec<McpServer>) {
        self.rows = servers.into_iter().map(McpRow::from).collect();
        self.rows.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn rows(&self) -> &[McpRow] {
        &self.rows
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
        store.replace(vec![server("b", "connected"), server("a", "failed")]);
        assert_eq!(store.rows()[0].name, "a");
        assert_eq!(store.rows()[0].status, "failed");
        store.replace(vec![server("a", "connected")]);
        assert_eq!(store.len(), 1);
        assert_eq!(store.rows()[0].status, "connected");
    }

    #[test]
    fn mcp_unknown_status_label() {
        let mut store = McpStore::default();
        store.replace(vec![server("a", "warming")]);
        assert_eq!(store.rows()[0].status, "unknown");
    }
}
