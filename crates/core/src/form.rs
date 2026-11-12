//! Agent forms: answers and the gate on Reply.

use std::collections::HashMap;

use harness_protocol::FormInfo;

/// Pending forms for the selected session.
#[derive(Debug, Default)]
pub struct FormStore {
    forms: Vec<FormInfo>,
    answers: HashMap<String, HashMap<String, serde_json::Value>>,
    error: Option<String>,
}

impl FormStore {
    pub fn replace(&mut self, forms: Vec<FormInfo>) {
        self.forms = forms;
        self.error = None;
        self.answers
            .retain(|id, _| self.forms.iter().any(|form| &form.id == id));
    }

    pub fn forms(&self) -> &[FormInfo] {
        &self.forms
    }

    /// Insert a form, replacing any pending form with the same id. The live
    /// stream and a command reply can both announce the same form.
    pub fn upsert(&mut self, form: FormInfo) {
        match self.forms.iter_mut().find(|pending| pending.id == form.id) {
            Some(pending) => *pending = form,
            None => self.forms.push(form),
        }
        self.error = None;
    }

    pub fn front(&self) -> Option<&FormInfo> {
        self.forms.first()
    }

    pub fn remove(&mut self, form_id: &str) {
        self.forms.retain(|form| form.id != form_id);
        self.answers.remove(form_id);
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn value(&self, form_id: &str, key: &str) -> Option<&serde_json::Value> {
        self.answers.get(form_id)?.get(key)
    }

    pub fn set_answer(&mut self, form_id: &str, key: &str, value: serde_json::Value) {
        self.answers
            .entry(form_id.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// True when every visible required field has an answer.
    pub fn can_reply(&self, form_id: &str) -> bool {
        let Some(form) = self.forms.iter().find(|form| form.id == form_id) else {
            return false;
        };
        let answers = self.answers.get(form_id);
        form.fields
            .iter()
            .filter(|field| field.required && !field.hidden)
            .all(|field| {
                answers
                    .and_then(|answers| answers.get(&field.key))
                    .is_some_and(|value| !value.is_null())
            })
    }

    /// The answer map to send. Unknown field kinds are never included.
    pub fn answer(&self, form_id: &str) -> Option<serde_json::Value> {
        let form = self.forms.iter().find(|form| form.id == form_id)?;
        let answers = self.answers.get(form_id)?;
        let mut map = serde_json::Map::new();
        for field in &form.fields {
            if field.hidden {
                continue;
            }
            let known = matches!(
                field.kind.as_str(),
                "string" | "number" | "integer" | "boolean" | "multiselect"
            );
            if !known {
                continue;
            }
            if let Some(value) = answers.get(&field.key) {
                map.insert(field.key.clone(), value.clone());
            }
        }
        Some(serde_json::Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::FormField;

    fn field(key: &str, kind: &str, required: bool) -> FormField {
        FormField {
            key: key.into(),
            kind: kind.into(),
            title: None,
            description: None,
            required,
            hidden: false,
            options: Vec::new(),
            default: None,
        }
    }

    fn form(id: &str, fields: Vec<FormField>) -> FormInfo {
        FormInfo {
            id: id.into(),
            session_id: "s1".into(),
            title: "Ask".into(),
            fields,
        }
    }

    #[test]
    fn form_required_blocks_reply() {
        let mut store = FormStore::default();
        store.replace(vec![form("f1", vec![field("name", "string", true)])]);
        assert!(!store.can_reply("f1"));
        store.set_answer("f1", "name", serde_json::json!("x"));
        assert!(store.can_reply("f1"));
    }

    #[test]
    fn form_unknown_field_is_not_answered() {
        let mut store = FormStore::default();
        store.replace(vec![form("f1", vec![field("color", "color", false)])]);
        store.set_answer("f1", "color", serde_json::json!("red"));
        let answer = store.answer("f1").expect("answer");
        assert_eq!(answer, serde_json::json!({}));
    }

    #[test]
    fn form_answer_includes_known_fields() {
        let mut store = FormStore::default();
        store.replace(vec![form(
            "f1",
            vec![field("name", "string", true), field("ok", "boolean", false)],
        )]);
        store.set_answer("f1", "name", serde_json::json!("x"));
        store.set_answer("f1", "ok", serde_json::json!(true));
        assert_eq!(
            store.answer("f1").expect("answer"),
            serde_json::json!({"name":"x","ok":true})
        );
    }

    #[test]
    fn form_remove_clears_answers() {
        let mut store = FormStore::default();
        store.replace(vec![form("f1", vec![field("name", "string", true)])]);
        store.set_answer("f1", "name", serde_json::json!("x"));
        store.remove("f1");
        assert!(store.forms().is_empty());
        assert!(store.answer("f1").is_none());
    }

    #[test]
    fn form_upsert_replaces_by_id() {
        let mut store = FormStore::default();
        store.upsert(form("f1", Vec::new()));
        let mut updated = form("f1", vec![field("name", "string", true)]);
        updated.title = "Renamed".into();
        store.upsert(updated);
        store.upsert(form("f2", Vec::new()));
        assert_eq!(store.forms().len(), 2);
        assert_eq!(store.forms()[0].title, "Renamed");
    }
}
