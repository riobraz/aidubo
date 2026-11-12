//! Agent form wire types.

use serde::{Deserialize, Serialize};

/// One question the agent asks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FormField {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub options: Vec<serde_json::Value>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

/// A form the agent is waiting on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FormInfo {
    pub id: String,
    #[serde(rename = "sessionID", default)]
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub fields: Vec<FormField>,
}

/// Body for the reply endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct FormReplyRequest {
    pub answer: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_parses_every_field_type() {
        let form: FormInfo = serde_json::from_str(
            r#"{"id":"f1","sessionID":"s1","title":"Ask","fields":[
                {"key":"name","type":"string","required":true},
                {"key":"count","type":"number"},
                {"key":"ok","type":"boolean"},
                {"key":"tags","type":"multiselect","options":["a","b"]},
                {"key":"link","type":"external","url":"https://example.com"}
            ]}"#,
        )
        .expect("form");
        assert_eq!(form.fields.len(), 5);
        assert!(form.fields[0].required);
        assert_eq!(form.fields[3].options.len(), 2);
    }

    #[test]
    fn form_unknown_field_is_preserved() {
        let field: FormField =
            serde_json::from_str(r#"{"key":"color","type":"color"}"#).expect("field");
        assert_eq!(field.kind, "color");
        assert!(!field.required);
    }

    #[test]
    fn form_reply_body_uses_key_map() {
        let body = FormReplyRequest {
            answer: serde_json::json!({"name":"x"}),
        };
        assert_eq!(
            serde_json::to_string(&body).expect("json"),
            r#"{"answer":{"name":"x"}}"#
        );
    }
}
