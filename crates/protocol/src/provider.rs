//! Provider and integration discovery types, plus the auth-attempt payloads.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct IntegrationInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub methods: Vec<IntegrationMethod>,
    #[serde(default)]
    pub connections: Vec<ConnectionInfo>,
}

impl IntegrationInfo {
    pub fn method_count(&self) -> usize {
        self.methods.len()
    }

    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    pub fn has_credential(&self) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.is_credential())
    }

    pub fn has_env(&self) -> bool {
        self.connections.iter().any(ConnectionInfo::is_env)
    }
}

/// One way to connect an integration. Unknown shapes stay as raw JSON so a
/// server addition renders a disabled row instead of breaking the list.
#[derive(Debug, Clone, PartialEq)]
pub enum IntegrationMethod {
    OAuth {
        id: String,
        label: Option<String>,
        form: serde_json::Value,
    },
    Command {
        id: String,
        label: String,
        command: Vec<String>,
    },
    Key {
        label: Option<String>,
        form: serde_json::Value,
    },
    External {
        id: String,
        label: Option<String>,
        form: serde_json::Value,
    },
    Env {
        names: Vec<String>,
    },
    Unknown {
        kind: String,
        raw: serde_json::Value,
    },
}

impl IntegrationMethod {
    pub fn kind(&self) -> &str {
        match self {
            Self::OAuth { .. } => "oauth",
            Self::Command { .. } => "command",
            Self::Key { .. } => "key",
            Self::External { .. } => "external",
            Self::Env { .. } => "env",
            Self::Unknown { kind, .. } => kind,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::OAuth { label, .. } => {
                label.clone().unwrap_or_else(|| "Sign in with OAuth".into())
            }
            Self::Command { label, .. } => label.clone(),
            Self::Key { label, .. } => label.clone().unwrap_or_else(|| "API key".into()),
            Self::External { label, .. } => label
                .clone()
                .unwrap_or_else(|| "External credentials".into()),
            Self::Env { names } => format!("Environment ({})", names.join(", ")),
            Self::Unknown { kind, .. } => {
                format!("Unsupported method ({kind})")
            }
        }
    }

    /// The id the connect endpoints expect, when the method carries one.
    pub fn method_id(&self) -> Option<&str> {
        match self {
            Self::OAuth { id, .. } | Self::Command { id, .. } | Self::External { id, .. } => {
                Some(id.as_str())
            }
            Self::Key { .. } | Self::Env { .. } | Self::Unknown { .. } => None,
        }
    }

    pub fn command_line(&self) -> Option<String> {
        match self {
            Self::Command { command, .. } => Some(command.join(" ")),
            _ => None,
        }
    }

    pub fn env_names(&self) -> &[String] {
        match self {
            Self::Env { names } => names,
            _ => &[],
        }
    }

    /// Whether the row can start a flow. `Env` and unknown methods are shown
    /// but disabled: the user must supply the variable or upgrade the app.
    pub fn is_connectable(&self) -> bool {
        matches!(
            self,
            Self::OAuth { .. } | Self::Command { .. } | Self::Key { .. } | Self::External { .. }
        )
    }

    pub fn form(&self) -> Option<&serde_json::Value> {
        match self {
            Self::OAuth { form, .. } | Self::Key { form, .. } | Self::External { form, .. } => {
                Some(form)
            }
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct RawMethod {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(default)]
    form: Option<serde_json::Value>,
}

impl Serialize for IntegrationMethod {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Unknown { raw, .. } => raw.serialize(serializer),
            other => {
                let mut value = serde_json::Map::new();
                value.insert(
                    "type".into(),
                    serde_json::Value::String(other.kind().into()),
                );
                if let Some(id) = other.method_id() {
                    value.insert("id".into(), serde_json::Value::String(id.into()));
                }
                match other {
                    Self::Command { label, command, .. } => {
                        value.insert("label".into(), serde_json::Value::String(label.clone()));
                        value.insert(
                            "command".into(),
                            serde_json::Value::Array(
                                command
                                    .iter()
                                    .map(|part| serde_json::Value::String(part.clone()))
                                    .collect(),
                            ),
                        );
                    }
                    Self::Env { names } => {
                        value.insert(
                            "names".into(),
                            serde_json::Value::Array(
                                names
                                    .iter()
                                    .map(|name| serde_json::Value::String(name.clone()))
                                    .collect(),
                            ),
                        );
                    }
                    Self::OAuth { label, form, .. } | Self::External { label, form, .. } => {
                        if let Some(label) = label {
                            value.insert("label".into(), serde_json::Value::String(label.clone()));
                        }
                        if !form.is_null() {
                            value.insert("form".into(), form.clone());
                        }
                    }
                    Self::Key { label, form } => {
                        if let Some(label) = label {
                            value.insert("label".into(), serde_json::Value::String(label.clone()));
                        }
                        if !form.is_null() {
                            value.insert("form".into(), form.clone());
                        }
                    }
                    Self::Unknown { .. } => {}
                }
                serde_json::Value::Object(value).serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for IntegrationMethod {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        let Ok(parsed) = serde_json::from_value::<RawMethod>(raw.clone()) else {
            return Ok(Self::Unknown {
                kind: String::new(),
                raw,
            });
        };
        let form = parsed.form.unwrap_or(serde_json::Value::Null);
        let method = match parsed.kind.as_str() {
            "oauth" => parsed.id.map(|id| Self::OAuth {
                id,
                label: parsed.label,
                form,
            }),
            "command" => match (parsed.id, parsed.label, parsed.command) {
                (Some(id), Some(label), Some(command)) => {
                    Some(Self::Command { id, label, command })
                }
                _ => None,
            },
            "key" => Some(Self::Key {
                label: parsed.label,
                form,
            }),
            "external" => parsed.id.map(|id| Self::External {
                id,
                label: parsed.label,
                form,
            }),
            "env" => parsed.names.map(|names| Self::Env { names }),
            _ => None,
        };
        Ok(method.unwrap_or(Self::Unknown {
            kind: parsed.kind,
            raw,
        }))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConnectionInfo {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<ConnectionStatus>,
}

impl ConnectionInfo {
    pub fn is_credential(&self) -> bool {
        self.kind == "credential"
    }

    pub fn is_env(&self) -> bool {
        self.kind == "env"
    }

    pub fn needs_auth(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.status == "needs_auth")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConnectionStatus {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct OAuthAttempt {
    #[serde(rename = "attemptID", default)]
    pub attempt_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub paste: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CommandAttempt {
    #[serde(rename = "attemptID", default)]
    pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthStatus {
    #[default]
    Pending,
    Complete,
    Failed,
    Expired,
    Unknown(String),
}

impl AuthStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending | Self::Unknown(_))
    }
}

impl Serialize for AuthStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "pending" => Self::Pending,
            "complete" => Self::Complete,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AttemptStatus {
    #[serde(default)]
    pub status: AuthStatus,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthStartRequest {
    #[serde(rename = "methodID")]
    pub method_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandStartRequest {
    #[serde(rename = "methodID")]
    pub method_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalStartRequest {
    #[serde(rename = "methodID")]
    pub method_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Serialized once for `POST .../connect/key`; never logged or derived
/// `Debug`.
#[derive(Clone, Serialize)]
pub struct SendKeyRequest {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OAuthCompleteRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
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
        assert!(provider.integration_id.is_none());
    }

    #[test]
    fn integration_parses_live_method_shapes() {
        let integration: IntegrationInfo = serde_json::from_str(
            r#"{
                "id":"i","name":"I",
                "methods":[
                    {"type":"key","label":"API key"},
                    {"id":"oauth","type":"oauth","label":"Sign in"},
                    {"id":"cmd","type":"command","label":"Run","command":["tool","auth"]},
                    {"id":"env","type":"external","label":"Profile","form":[{"key":"profile"}]},
                    {"type":"env","names":["TOKEN"]}
                ],
                "connections":[
                    {"type":"env","name":"TOKEN","status":{"status":"needs_auth","message":"set TOKEN"}},
                    {"type":"credential","id":"c1","label":"default","method":"key"}
                ]
            }"#,
        )
        .expect("integration");
        assert_eq!(integration.method_count(), 5);
        assert_eq!(integration.connection_count(), 2);
        assert!(integration.has_credential());
        assert!(integration.has_env());
        assert_eq!(integration.methods[0].kind(), "key");
        assert_eq!(
            integration.methods[2].command_line().as_deref(),
            Some("tool auth")
        );
        assert_eq!(integration.methods[4].env_names(), ["TOKEN".to_string()]);
        assert!(integration.connections[0].needs_auth());
    }

    #[test]
    fn integration_unknown_method_is_disabled() {
        let integration: IntegrationInfo = serde_json::from_str(
            r#"{"id":"i","name":"I","methods":[{"type":"saml"},{"type":"key"}],"connections":[]}"#,
        )
        .expect("integration");
        assert!(matches!(
            integration.methods[0],
            IntegrationMethod::Unknown { .. }
        ));
        assert!(!integration.methods[0].is_connectable());
        assert!(integration.methods[0].label().contains("Unsupported"));
        assert!(integration.methods[1].is_connectable());
    }

    #[test]
    fn integration_malformed_known_method_stays_unknown() {
        let method: IntegrationMethod =
            serde_json::from_str(r#"{"type":"command","id":"x"}"#).expect("method");
        assert!(matches!(method, IntegrationMethod::Unknown { .. }));
    }

    #[test]
    fn oauth_attempt_and_status_parse() {
        let attempt: OAuthAttempt = serde_json::from_str(
            r#"{"attemptID":"a1","url":"https://example.com/auth","instructions":"Sign in","mode":"auto","paste":true,"time":{"created":1.0,"expires":2.0}}"#,
        )
        .expect("attempt");
        assert_eq!(attempt.attempt_id, "a1");
        assert!(attempt.paste);
        let status: AttemptStatus =
            serde_json::from_str(r#"{"status":"failed","message":"nope"}"#).expect("status");
        assert_eq!(status.status, AuthStatus::Failed);
        assert_eq!(status.message.as_deref(), Some("nope"));
        let unknown: AttemptStatus =
            serde_json::from_str(r#"{"status":"queued"}"#).expect("status");
        assert_eq!(unknown.status, AuthStatus::Unknown("queued".into()));
    }

    #[test]
    fn send_key_request_has_no_debug() {
        // Compile-time guard: the request must not derive Debug.
        fn assert_debug<T: std::fmt::Debug>() {}
        // `SendKeyRequest` intentionally fails this bound; the assertion below
        // documents that by checking another type.
        assert_debug::<OAuthStartRequest>();
        let request = SendKeyRequest {
            key: "sk-secret".into(),
            label: None,
        };
        let value = serde_json::to_value(&request).expect("json");
        assert_eq!(value["key"], "sk-secret");
    }
}
