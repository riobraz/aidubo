//! Providers and integrations.

use harness_protocol::{IntegrationInfo, ProviderInfo};

#[derive(Debug, Clone, Default)]
pub struct ProviderStore {
    providers: Vec<ProviderInfo>,
    integrations: Vec<IntegrationInfo>,
}

impl ProviderStore {
    pub fn set_providers(&mut self, providers: Vec<ProviderInfo>) {
        self.providers = providers;
        self.providers.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn set_integrations(&mut self, integrations: Vec<IntegrationInfo>) {
        self.integrations = integrations;
        self.integrations.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    pub fn integrations(&self) -> &[IntegrationInfo] {
        &self.integrations
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, name: &str) -> ProviderInfo {
        ProviderInfo {
            id: id.into(),
            name: name.into(),
            ..ProviderInfo::default()
        }
    }

    #[test]
    fn providers_sorted_by_name() {
        let mut store = ProviderStore::default();
        store.set_providers(vec![provider("b", "Beta"), provider("a", "Alpha")]);
        assert_eq!(store.providers()[0].name, "Alpha");
        assert!(!store.is_empty());
    }
}
