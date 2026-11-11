//! Provider and integration discovery types.

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProviderInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(rename = "integrationID", default)]
    pub integration_id: Option<String>,
    #[serde(default)]
    pub activation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct IntegrationInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub methods: serde_json::Value,
    #[serde(default)]
    pub connections: serde_json::Value,
}

impl IntegrationInfo {
    pub fn method_count(&self) -> usize {
        self.methods.as_array().map(Vec::len).unwrap_or(0)
    }

    pub fn connection_count(&self) -> usize {
        self.connections.as_array().map(Vec::len).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_minimal_fields_parse() {
        let provider: ProviderInfo =
            serde_json::from_str(r#"{"id":"p","name":"P"}"#).expect("provider");
        assert_eq!(provider.id, "p");
        assert!(provider.package.is_none());
    }

    #[test]
    fn integration_counts_methods_and_connections() {
        let integration: IntegrationInfo = serde_json::from_str(
            r#"{"id":"i","name":"I","methods":[{"type":"key"}],"connections":[{"type":"env"}]}"#,
        )
        .expect("integration");
        assert_eq!(integration.method_count(), 1);
        assert_eq!(integration.connection_count(), 1);
    }
}
