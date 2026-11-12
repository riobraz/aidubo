//! Saved permission rules.

use harness_protocol::SavedPermission;

#[derive(Debug, Default)]
pub struct SavedPermissionStore {
    rules: Vec<SavedPermission>,
}

impl SavedPermissionStore {
    pub fn replace(&mut self, rules: Vec<SavedPermission>) {
        self.rules = rules;
        self.rules
            .sort_by(|a, b| a.action.cmp(&b.action).then(a.resource.cmp(&b.resource)));
    }

    pub fn rules(&self) -> &[SavedPermission] {
        &self.rules
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.rules.len();
        self.rules.retain(|rule| rule.id != id);
        self.rules.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, action: &str, resource: &str) -> SavedPermission {
        SavedPermission {
            id: id.into(),
            project_id: "p1".into(),
            action: action.into(),
            resource: resource.into(),
        }
    }

    #[test]
    fn saved_permission_remove_is_idempotent() {
        let mut store = SavedPermissionStore::default();
        store.replace(vec![rule("r1", "shell", "git *")]);
        assert!(store.remove("r1"));
        assert!(!store.remove("r1"));
        assert!(store.is_empty());
    }

    #[test]
    fn saved_permission_sorted_by_action() {
        let mut store = SavedPermissionStore::default();
        store.replace(vec![rule("r2", "write", "b"), rule("r1", "shell", "a")]);
        assert_eq!(store.rules()[0].action, "shell");
        assert_eq!(store.rules()[1].action, "write");
    }
}
