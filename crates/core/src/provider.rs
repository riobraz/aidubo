//! Providers, integrations, and the per-integration auth attempt state.

use std::collections::BTreeMap;

use harness_protocol::{CommandAttempt, IntegrationInfo, OAuthAttempt, ProviderInfo};

/// Which connect endpoint started an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    OAuth,
    Command,
    Key,
    External,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::Command => "command",
            Self::Key => "key",
            Self::External => "external",
        }
    }
}

/// The attempt the UI is waiting on. It holds no credential: only the ids and
/// the instructions the server returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAttempt {
    pub integration_id: String,
    pub attempt_id: String,
    pub method: String,
    pub kind: AuthKind,
    pub url: Option<String>,
    pub command: Option<String>,
    pub instructions: String,
}

impl AuthAttempt {
    /// The host to show next to an external link. Only `https` URLs yield one;
    /// anything else is never offered for opening.
    pub fn host(&self) -> Option<String> {
        self.url.as_deref().and_then(url_host)
    }
}

pub fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// A single integration's connect flow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AuthFlow {
    #[default]
    Idle,
    /// The server returned an attempt; the user must act.
    AwaitingUser(AuthAttempt),
    /// The engine is polling the attempt.
    Polling(AuthAttempt),
    Failed {
        message: String,
    },
    Connected {
        method: String,
    },
}

impl AuthFlow {
    pub fn attempt(&self) -> Option<&AuthAttempt> {
        match self {
            Self::AwaitingUser(attempt) | Self::Polling(attempt) => Some(attempt),
            Self::Idle | Self::Failed { .. } | Self::Connected { .. } => None,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::AwaitingUser(_) | Self::Polling(_))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderStore {
    providers: Vec<ProviderInfo>,
    integrations: Vec<IntegrationInfo>,
    flows: BTreeMap<String, AuthFlow>,
}

impl ProviderStore {
    pub fn set_providers(&mut self, mut providers: Vec<ProviderInfo>) {
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        self.providers = providers;
    }

    pub fn set_integrations(&mut self, mut integrations: Vec<IntegrationInfo>) {
        integrations.sort_by(|a, b| a.name.cmp(&b.name));
        let known: Vec<String> = integrations
            .iter()
            .map(|integration| integration.id.clone())
            .collect();
        self.integrations = integrations;
        // A fresh snapshot may finish a flow the poller already reported; keep
        // local state so a list refresh cannot resurrect a waiting dialog.
        self.flows
            .retain(|id, flow| flow.is_active() || known.contains(id));
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

    pub fn flow(&self, integration_id: &str) -> AuthFlow {
        self.flows.get(integration_id).cloned().unwrap_or_default()
    }

    pub fn set_flow(&mut self, integration_id: &str, flow: AuthFlow) {
        if flow == AuthFlow::Idle {
            self.flows.remove(integration_id);
        } else {
            self.flows.insert(integration_id.to_string(), flow);
        }
    }

    pub fn begin_oauth(
        &mut self,
        integration_id: &str,
        method: &str,
        attempt: &OAuthAttempt,
    ) -> AuthAttempt {
        let state = AuthAttempt {
            integration_id: integration_id.to_string(),
            attempt_id: attempt.attempt_id.clone(),
            method: method.to_string(),
            kind: AuthKind::OAuth,
            url: (!attempt.url.is_empty()).then(|| attempt.url.clone()),
            command: None,
            instructions: attempt.instructions.clone(),
        };
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::AwaitingUser(state.clone()),
        );
        state
    }

    pub fn begin_command(
        &mut self,
        integration_id: &str,
        method: &str,
        attempt: &CommandAttempt,
        command: String,
    ) -> AuthAttempt {
        let state = AuthAttempt {
            integration_id: integration_id.to_string(),
            attempt_id: attempt.attempt_id.clone(),
            method: method.to_string(),
            kind: AuthKind::Command,
            url: None,
            command: Some(command),
            instructions: String::new(),
        };
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::AwaitingUser(state.clone()),
        );
        state
    }

    pub fn begin_external(&mut self, integration_id: &str, method: &str) -> AuthAttempt {
        let state = AuthAttempt {
            integration_id: integration_id.to_string(),
            attempt_id: String::new(),
            method: method.to_string(),
            kind: AuthKind::External,
            url: None,
            command: None,
            instructions: String::new(),
        };
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::AwaitingUser(state.clone()),
        );
        state
    }

    /// The user finished the interactive step; the engine may poll now.
    pub fn start_polling(&mut self, integration_id: &str) -> Option<AuthAttempt> {
        let flow = self.flows.get(integration_id)?.clone();
        let attempt = flow.attempt()?.clone();
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::Polling(attempt.clone()),
        );
        Some(attempt)
    }

    pub fn complete(&mut self, integration_id: &str, method: &str) {
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::Connected {
                method: method.to_string(),
            },
        );
    }

    /// Mark an integration connected, keeping the method when it is known.
    pub fn mark_connected(&mut self, integration_id: &str) {
        let method = self
            .flow(integration_id)
            .attempt()
            .map(|attempt| attempt.method.clone())
            .unwrap_or_default();
        self.complete(integration_id, &method);
    }

    pub fn fail(&mut self, integration_id: &str, message: &str) {
        self.flows.insert(
            integration_id.to_string(),
            AuthFlow::Failed {
                message: redact_error(message),
            },
        );
    }

    pub fn cancel(&mut self, integration_id: &str) {
        self.flows.remove(integration_id);
    }

    pub fn last_error(&self, integration_id: &str) -> Option<&str> {
        match self.flows.get(integration_id) {
            Some(AuthFlow::Failed { message }) => Some(message.as_str()),
            _ => None,
        }
    }

    /// Whether the integration has stored a credential the server will use.
    pub fn has_credential(&self, integration_id: &str) -> bool {
        self.integrations
            .iter()
            .find(|integration| integration.id == integration_id)
            .is_some_and(IntegrationInfo::has_credential)
    }

    /// Number of connections the server reports as needing auth.
    pub fn needs_auth(&self, integration_id: &str) -> bool {
        self.integrations
            .iter()
            .find(|integration| integration.id == integration_id)
            .is_some_and(|integration| {
                integration
                    .connections
                    .iter()
                    .any(|connection| connection.needs_auth())
            })
    }
}

const MAX_ERROR_CHARS: usize = 400;

/// Strip anything that looks like a credential and cap the length. Error text
/// reaches the UI and logs; it must never carry a key, a bearer token, or a
/// long opaque secret the server echoed back.
pub fn redact_error(message: &str) -> String {
    redact_error_with(message, &[])
}

/// Like [`redact_error`], additionally scrubbing known secret values.
pub fn redact_error_with(message: &str, secrets: &[&str]) -> String {
    let mut text = message.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            text = text.replace(secret, "[redacted]");
        }
    }
    let text = redact_tokens(&text);
    truncate_chars(&text, MAX_ERROR_CHARS)
}

fn redact_tokens(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut token = String::new();
    let is_token_char =
        |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '/' | '=');
    let flush = |token: &mut String, output: &mut String| {
        if is_secret_like(token) {
            output.push_str("[redacted]");
        } else {
            output.push_str(token);
        }
        token.clear();
    };
    for ch in message.chars() {
        if is_token_char(ch) {
            token.push(ch);
        } else {
            flush(&mut token, &mut output);
            output.push(ch);
        }
    }
    flush(&mut token, &mut output);
    output
}

fn is_secret_like(token: &str) -> bool {
    const PREFIXES: [&str; 5] = ["sk-", "sk_", "ghp_", "xoxb-", "xoxp-"];
    if PREFIXES.iter().any(|prefix| token.starts_with(prefix)) {
        return true;
    }
    token.len() >= 32
        && token.chars().any(|ch| ch.is_ascii_digit())
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(max.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::ConnectionInfo;

    fn provider(id: &str, name: &str) -> ProviderInfo {
        ProviderInfo {
            id: id.into(),
            name: name.into(),
            ..ProviderInfo::default()
        }
    }

    fn oauth(attempt_id: &str) -> OAuthAttempt {
        OAuthAttempt {
            attempt_id: attempt_id.into(),
            url: "https://auth.example.com/start?state=abc".into(),
            instructions: "Sign in".into(),
            ..OAuthAttempt::default()
        }
    }

    #[test]
    fn providers_sorted_by_name() {
        let mut store = ProviderStore::default();
        store.set_providers(vec![provider("b", "Beta"), provider("a", "Alpha")]);
        assert_eq!(store.providers()[0].name, "Alpha");
        assert!(!store.is_empty());
    }

    #[test]
    fn provider_attempt_state_machine() {
        let mut store = ProviderStore::default();
        let started = store.begin_oauth("acme", "oauth", &oauth("att_1"));
        assert_eq!(started.host().as_deref(), Some("auth.example.com"));
        assert!(matches!(
            store.flow("acme"),
            AuthFlow::AwaitingUser(attempt) if attempt.attempt_id == "att_1"
        ));

        let polling = store.start_polling("acme").expect("polling");
        assert_eq!(polling.kind, AuthKind::OAuth);
        assert!(matches!(store.flow("acme"), AuthFlow::Polling(_)));

        store.fail("acme", "attempt expired");
        assert!(matches!(
            store.flow("acme"),
            AuthFlow::Failed { ref message } if message == "attempt expired"
        ));
        store.cancel("acme");
        assert_eq!(store.flow("acme"), AuthFlow::Idle);

        let attempt = CommandAttempt {
            attempt_id: "att_2".into(),
        };
        store.begin_command("acme", "cli", &attempt, "tool login".into());
        assert!(matches!(
            store.flow("acme"),
            AuthFlow::AwaitingUser(ref attempt)
                if attempt.command.as_deref() == Some("tool login")
        ));
        store.complete("acme", "cli");
        assert!(matches!(
            store.flow("acme"),
            AuthFlow::Connected { ref method } if method == "cli"
        ));
        assert!(store.last_error("acme").is_none());
    }

    #[test]
    fn provider_error_is_redacted() {
        let plain = "OpenCode returned 404: Integration not found";
        assert_eq!(redact_error(plain), plain);

        let leaked = "connect failed for sk-live-abcdef1234567890";
        let redacted = redact_error(leaked);
        assert!(!redacted.contains("sk-live"));
        assert!(redacted.contains("[redacted]"));

        let known = redact_error_with("bad key hunter2-secret", &["hunter2-secret"]);
        assert!(!known.contains("hunter2"));
        assert!(known.contains("[redacted]"));

        let long = "x".repeat(1000);
        assert!(redact_error(&long).chars().count() <= MAX_ERROR_CHARS);
    }

    #[test]
    fn provider_flow_survives_an_integration_reload() {
        let mut store = ProviderStore::default();
        store.set_integrations(vec![IntegrationInfo {
            id: "acme".into(),
            name: "Acme".into(),
            connections: vec![ConnectionInfo {
                kind: "credential".into(),
                id: Some("c1".into()),
                ..ConnectionInfo::default()
            }],
            ..IntegrationInfo::default()
        }]);
        store.begin_oauth("acme", "oauth", &oauth("att_1"));
        store.set_integrations(vec![IntegrationInfo {
            id: "acme".into(),
            name: "Acme".into(),
            ..IntegrationInfo::default()
        }]);
        assert!(!store.has_credential("acme"));
        assert!(matches!(store.flow("acme"), AuthFlow::AwaitingUser(_)));
    }

    #[test]
    fn url_host_only_accepts_https() {
        assert_eq!(
            url_host("https://example.com/path").as_deref(),
            Some("example.com")
        );
        assert_eq!(url_host("http://example.com"), None);
        assert_eq!(url_host("file:///etc/passwd"), None);
        assert_eq!(url_host("https://"), None);
    }
}
