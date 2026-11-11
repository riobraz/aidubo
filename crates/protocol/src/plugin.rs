//! Plugin status wire types. Source and state are unions; keep the raw value
//! and expose labels so an unknown variant still renders.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PluginInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub source: serde_json::Value,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub features: serde_json::Value,
}

impl PluginInfo {
    pub fn source_label(&self) -> String {
        match self.source.get("type").and_then(serde_json::Value::as_str) {
            Some("builtin") => "builtin".into(),
            Some("package") => {
                let target = self
                    .source
                    .get("target")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("package");
                match self
                    .source
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                {
                    Some(version) => format!("{target} {version}"),
                    None => target.to_string(),
                }
            }
            Some("local") => "local".into(),
            Some("sdk") => "sdk".into(),
            Some(other) => other.to_string(),
            None => "unknown".into(),
        }
    }

    pub fn status_label(&self) -> &str {
        match self.state.get("status").and_then(serde_json::Value::as_str) {
            Some("active") => "active",
            Some("failed") => "failed",
            _ => "unknown",
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.state.get("error").and_then(serde_json::Value::as_str)
    }

    pub fn outdated(&self) -> bool {
        self.source
            .get("outdated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_source_variants_parse() {
        for (json, label) in [
            (r#"{"type":"builtin"}"#, "builtin"),
            (
                r#"{"type":"package","target":"@x/y","version":"1.2.3"}"#,
                "@x/y 1.2.3",
            ),
            (r#"{"type":"local","path":"/p"}"#, "local"),
            (r#"{"type":"sdk"}"#, "sdk"),
            (r#"{"type":"future"}"#, "future"),
        ] {
            let plugin: PluginInfo =
                serde_json::from_str(&format!(r#"{{"id":"p","source":{json}}}"#)).expect("plugin");
            assert_eq!(plugin.source_label(), label);
        }
    }

    #[test]
    fn plugin_failed_state_keeps_error() {
        let plugin: PluginInfo =
            serde_json::from_str(r#"{"id":"p","state":{"status":"failed","error":"boom"}}"#)
                .expect("plugin");
        assert_eq!(plugin.status_label(), "failed");
        assert_eq!(plugin.error(), Some("boom"));
    }

    #[test]
    fn plugin_outdated_flag() {
        let plugin: PluginInfo = serde_json::from_str(
            r#"{"id":"p","source":{"type":"package","target":"x","outdated":true}}"#,
        )
        .expect("plugin");
        assert!(plugin.outdated());
    }
}
