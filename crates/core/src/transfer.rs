//! Session transfer state and import rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harness_protocol::{SessionStats, SessionTransfer, StatsRange};

/// What the user is about to import, shown before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPreview {
    pub session_id: String,
    pub title: String,
    pub project: Option<String>,
    pub message_count: usize,
}

/// A message the server would accept: an object with a non-empty string id.
fn is_message(value: &serde_json::Value) -> bool {
    value
        .get("id")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.trim().is_empty())
}

/// Refuse an export locally when importing it would drop or corrupt messages.
/// The server cannot fix a malformed payload, so this runs before any request.
pub fn validate_import(transfer: &SessionTransfer) -> Result<ImportPreview, String> {
    let session_id = transfer.session_id().trim();
    if session_id.is_empty() {
        return Err("This export has no session id.".into());
    }

    let readable = transfer
        .messages
        .iter()
        .filter(|message| is_message(message))
        .count();
    if readable != transfer.messages.len() {
        return Err(format!(
            "The export lists {} messages but only {readable} are readable; importing it would lose data.",
            transfer.messages.len()
        ));
    }

    let title = transfer
        .title()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Untitled session")
        .to_string();
    let project = transfer
        .project_directory()
        .or_else(|| transfer.project_id())
        .map(str::to_string);

    Ok(ImportPreview {
        session_id: session_id.to_string(),
        title,
        project,
        message_count: transfer.message_count(),
    })
}

/// Turn an import failure into a message the user can act on. The server
/// refuses a duplicate id with 409; that is a conflict, not a broken file.
pub fn import_error(message: &str, preview: &ImportPreview) -> String {
    if message.contains("409") || message.contains("already exists") {
        format!(
            "Session already exists: {}. The server will not overwrite it; import a copy with a new id.",
            preview.session_id
        )
    } else {
        format!("Could not import the session: {message}")
    }
}

/// The last export written and the stats already fetched. Keeping the cache
/// keyed by range means switching presets never shows another range's totals.
#[derive(Debug, Default)]
pub struct TransferStore {
    last_export: Option<ExportRecord>,
    last_import: Option<ImportPreview>,
    stats: BTreeMap<StatsRange, SessionStats>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecord {
    pub session_id: String,
    pub path: PathBuf,
    pub bytes: u64,
}

impl TransferStore {
    pub fn record_export(&mut self, session_id: String, path: PathBuf, bytes: u64) {
        self.last_export = Some(ExportRecord {
            session_id,
            path,
            bytes,
        });
    }

    pub fn last_export(&self) -> Option<&ExportRecord> {
        self.last_export.as_ref()
    }

    pub fn record_import(&mut self, preview: ImportPreview) {
        self.last_import = Some(preview);
    }

    pub fn last_import(&self) -> Option<&ImportPreview> {
        self.last_import.as_ref()
    }

    pub fn cache_stats(&mut self, stats: SessionStats) {
        self.stats.insert(stats.range, stats);
    }

    pub fn stats(&self, range: StatsRange) -> Option<&SessionStats> {
        self.stats.get(&range)
    }

    pub fn exported_path(&self) -> Option<&Path> {
        self.last_export
            .as_ref()
            .map(|export| export.path.as_path())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn transfer(messages: Vec<serde_json::Value>) -> SessionTransfer {
        SessionTransfer {
            info: json!({
                "id": "ses_1",
                "title": "Import test",
                "location": { "directory": "/tmp/import" }
            }),
            messages,
        }
    }

    #[test]
    fn import_rejects_message_count_mismatch() {
        let good = json!({ "type": "user", "id": "msg_1", "text": "hi" });
        let preview = validate_import(&transfer(vec![good.clone(), good])).expect("preview");
        assert_eq!(preview.session_id, "ses_1");
        assert_eq!(preview.title, "Import test");
        assert_eq!(preview.project.as_deref(), Some("/tmp/import"));
        assert_eq!(preview.message_count, 2);

        let broken = transfer(vec![
            json!({ "type": "user", "id": "msg_1" }),
            json!("junk"),
        ]);
        let error = validate_import(&broken).expect_err("mismatch must be refused");
        assert!(error.contains("lists 2 messages but only 1"), "{error}");

        let missing_id = transfer(vec![json!({ "type": "user", "text": "hi" })]);
        assert!(validate_import(&missing_id).is_err());

        let mut empty = transfer(Vec::new());
        empty.info = json!({ "title": "no id" });
        assert!(validate_import(&empty).is_err());
    }

    #[test]
    fn import_conflict_shows_message() {
        let preview = validate_import(&transfer(Vec::new())).expect("preview");
        let server = r#"OpenCode returned 409 Conflict: {"_tag":"ConflictError","message":"Session already exists: ses_1"}"#;
        let message = import_error(server, &preview);
        assert!(
            message.contains("Session already exists: ses_1"),
            "{message}"
        );

        let other = import_error("OpenCode returned 500: boom", &preview);
        assert!(other.starts_with("Could not import the session"), "{other}");
    }

    #[test]
    fn stats_cache_keyed_by_range() {
        let mut store = TransferStore::default();
        let first = StatsRange { from: 0, to: 100 };
        let second = StatsRange { from: 100, to: 200 };
        store.cache_stats(SessionStats {
            range: first,
            sessions: 1,
            prompts: 2,
            ..SessionStats::default()
        });
        store.cache_stats(SessionStats {
            range: second,
            sessions: 3,
            prompts: 4,
            ..SessionStats::default()
        });

        assert_eq!(store.stats(first).map(|stats| stats.sessions), Some(1));
        assert_eq!(store.stats(second).map(|stats| stats.prompts), Some(4));
        assert!(store.stats(StatsRange::default()).is_none());
    }

    #[test]
    fn export_and_import_records_are_kept() {
        let mut store = TransferStore::default();
        store.record_export("ses_1".into(), PathBuf::from("/tmp/export.json"), 42);
        let export = store.last_export().expect("export");
        assert_eq!(export.session_id, "ses_1");
        assert_eq!(export.bytes, 42);
        assert_eq!(store.exported_path(), Some(Path::new("/tmp/export.json")));

        let preview = validate_import(&transfer(Vec::new())).expect("preview");
        store.record_import(preview.clone());
        assert_eq!(store.last_import(), Some(&preview));
    }
}
