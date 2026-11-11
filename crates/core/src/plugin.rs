//! Installed plugins.

use harness_protocol::PluginInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub source: String,
    pub status: String,
    pub error: Option<String>,
    pub outdated: bool,
}

impl From<PluginInfo> for PluginRow {
    fn from(info: PluginInfo) -> Self {
        let error = info.error().map(|error| error.chars().take(400).collect());
        let source = info.source_label();
        let status = info.status_label().to_string();
        let outdated = info.outdated();
        Self {
            id: info.id,
            source,
            status,
            error,
            outdated,
        }
    }
}

#[derive(Debug, Default)]
pub struct PluginStore {
    rows: Vec<PluginRow>,
}

impl PluginStore {
    pub fn replace(&mut self, plugins: Vec<PluginInfo>) {
        self.rows = plugins.into_iter().map(PluginRow::from).collect();
        self.rows.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn rows(&self) -> &[PluginRow] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, state: &str) -> PluginInfo {
        serde_json::from_str(&format!(
            r#"{{"id":"{id}","source":{{"type":"builtin"}},"state":{state}}}"#
        ))
        .expect("plugin")
    }

    #[test]
    fn plugin_sorted_and_error_truncated() {
        let mut store = PluginStore::default();
        let long = "x".repeat(500);
        store.replace(vec![
            plugin("b", r#"{"status":"active"}"#),
            plugin("a", &format!(r#"{{"status":"failed","error":"{long}"}}"#)),
        ]);
        assert_eq!(store.rows()[0].id, "a");
        assert_eq!(store.rows()[0].error.as_ref().map(String::len), Some(400));
    }

    #[test]
    fn plugin_status_labels() {
        let mut store = PluginStore::default();
        store.replace(vec![
            plugin("a", r#"{"status":"active"}"#),
            plugin("b", r#"{"status":"failed","error":"x"}"#),
            plugin("c", r#"{"status":"warming"}"#),
        ]);
        let statuses: Vec<&str> = store.rows().iter().map(|row| row.status.as_str()).collect();
        assert_eq!(statuses, ["active", "failed", "unknown"]);
    }
}
