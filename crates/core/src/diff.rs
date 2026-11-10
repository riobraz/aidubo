//! Diff wire types and the patch parser.
//!
//! A patch is untrusted text from the server. The parser never panics, never
//! executes anything, and stops at fixed limits instead of growing without
//! bound.

use harness_protocol::FileDiff;

/// Maximum bytes parsed from one patch.
pub const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
/// Maximum lines parsed across one patch.
pub const MAX_PATCH_LINES: usize = 50_000;

/// How a file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsChange {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    Unknown(String),
}

impl VcsChange {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "added" => Self::Added,
            "modified" => Self::Modified,
            "deleted" => Self::Deleted,
            "renamed" => Self::Renamed,
            "untracked" => Self::Untracked,
            "conflicted" | "conflict" => Self::Conflicted,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Untracked => "untracked",
            Self::Conflicted => "conflicted",
            Self::Unknown(value) => value,
        }
    }
}

/// A parsed patch, grouped per file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDiff {
    pub files: Vec<FilePatch>,
    pub truncated: bool,
    pub binary: bool,
}

/// One file in a patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: Option<VcsChange>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

impl FilePatch {
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }
}

/// One `@@` hunk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// One line inside a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Meta,
}

/// Parse one `FileDiff` into a render model.
pub fn parse_patch(diff: &FileDiff) -> ParsedDiff {
    parse_text(&diff.patch, diff.more)
}

/// Parse raw unified-diff text.
pub fn parse_text(patch: &str, truncated_by_server: bool) -> ParsedDiff {
    let mut parsed = ParsedDiff {
        truncated: truncated_by_server,
        ..ParsedDiff::default()
    };
    if patch.is_empty() {
        return parsed;
    }

    let bytes = patch.len();
    if bytes > MAX_PATCH_BYTES {
        parsed.truncated = true;
    }
    let budget = patch.as_bytes();
    let limit = budget.len().min(MAX_PATCH_BYTES);
    let text = String::from_utf8_lossy(&budget[..limit]);
    let mut file: Option<FilePatch> = None;
    let mut hunk: Option<Hunk> = None;
    let mut old_no = 0_u32;
    let mut new_no = 0_u32;
    let mut lines = 0_usize;

    let finish_file = |file: &mut Option<FilePatch>, parsed: &mut ParsedDiff| {
        if let Some(mut value) = file.take() {
            if value.status.is_none() {
                value.status = infer_status(&value);
            }
            parsed.files.push(value);
        }
    };

    for raw in text.lines() {
        lines += 1;
        if lines > MAX_PATCH_LINES {
            parsed.truncated = true;
            break;
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        if let Some(rest) = line.strip_prefix("diff --git ") {
            finish_file(&mut file, &mut parsed);
            hunk = None;
            let (old, new) = split_git_paths(rest);
            file = Some(FilePatch {
                old_path: old,
                new_path: new,
                ..FilePatch::default()
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("--- ") {
            let path = clean_path(rest);
            let entry = file.get_or_insert_with(FilePatch::default);
            entry.old_path = path;
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            let path = clean_path(rest);
            let entry = file.get_or_insert_with(FilePatch::default);
            entry.new_path = path;
            continue;
        }
        if line.starts_with("new file mode") {
            if let Some(entry) = file.as_mut() {
                entry.status = Some(VcsChange::Added);
            }
            continue;
        }
        if line.starts_with("deleted file mode") {
            if let Some(entry) = file.as_mut() {
                entry.status = Some(VcsChange::Deleted);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename from ") {
            if let Some(entry) = file.as_mut() {
                entry.old_path = Some(rest.to_string());
                entry.status = Some(VcsChange::Renamed);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename to ") {
            if let Some(entry) = file.as_mut() {
                entry.new_path = Some(rest.to_string());
                entry.status = Some(VcsChange::Renamed);
            }
            continue;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            if let Some(entry) = file.as_mut() {
                entry.binary = true;
            }
            parsed.binary = true;
            continue;
        }

        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(previous) = hunk.take()
                && let Some(entry) = file.as_mut()
            {
                entry.hunks.push(previous);
            }
            let (old_start, old_count, new_start, new_count, header) = parse_hunk_header(rest);
            old_no = old_start;
            new_no = new_start;
            hunk = Some(Hunk {
                old_start,
                old_lines: old_count,
                new_start,
                new_lines: new_count,
                header: header.to_string(),
                lines: Vec::new(),
            });
            continue;
        }

        let Some(current) = hunk.as_mut() else {
            continue;
        };
        let (kind, body) = match line.chars().next() {
            Some('+') => (DiffLineKind::Added, &line[1..]),
            Some('-') => (DiffLineKind::Removed, &line[1..]),
            Some(' ') => (DiffLineKind::Context, &line[1..]),
            Some('\\') => (DiffLineKind::Meta, line),
            _ => (DiffLineKind::Context, line),
        };
        let (old_value, new_value) = match kind {
            DiffLineKind::Added => {
                let value = Some(new_no);
                new_no = new_no.saturating_add(1);
                (None, value)
            }
            DiffLineKind::Removed => {
                let value = Some(old_no);
                old_no = old_no.saturating_add(1);
                (value, None)
            }
            DiffLineKind::Context => {
                let old_value = Some(old_no);
                let new_value = Some(new_no);
                old_no = old_no.saturating_add(1);
                new_no = new_no.saturating_add(1);
                (old_value, new_value)
            }
            DiffLineKind::Meta => (None, None),
        };
        current.lines.push(DiffLine {
            kind,
            text: body.to_string(),
            old_no: old_value,
            new_no: new_value,
        });
    }

    if let Some(previous) = hunk.take()
        && let Some(entry) = file.as_mut()
    {
        entry.hunks.push(previous);
    }
    finish_file(&mut file, &mut parsed);
    parsed
}

fn infer_status(file: &FilePatch) -> Option<VcsChange> {
    match (&file.old_path, &file.new_path) {
        (None, Some(_)) => Some(VcsChange::Added),
        (Some(_), None) => Some(VcsChange::Deleted),
        (Some(old), Some(new)) if old != new => Some(VcsChange::Renamed),
        _ => Some(VcsChange::Modified),
    }
}

fn clean_path(raw: &str) -> Option<String> {
    let value = raw.split('\t').next().unwrap_or(raw).trim();
    if value == "/dev/null" {
        return None;
    }
    let value = value
        .strip_prefix("a/")
        .or_else(|| value.strip_prefix("b/"))
        .unwrap_or(value);
    Some(value.to_string())
}

fn split_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    // `a/path b/path`; paths with spaces are ambiguous without quoting, so the
    // first ` b/` is the split point.
    if let Some(index) = rest.find(" b/") {
        let old = clean_path(&rest[..index]);
        let new = clean_path(&rest[index + 1..]);
        (old, new)
    } else {
        (clean_path(rest), None)
    }
}

fn parse_hunk_header(rest: &str) -> (u32, u32, u32, u32, &str) {
    // `-a,b +c,d @@ optional`
    let mut parts = rest.splitn(3, ' ');
    let old = parts.next().unwrap_or("-1,1");
    let new = parts.next().unwrap_or("+1,1");
    let header = parts.next().unwrap_or("");
    let (old_start, old_count) = parse_range(old.trim_start_matches('-'));
    let (new_start, new_count) = parse_range(new.trim_start_matches('+'));
    let header = header.strip_prefix("@@").unwrap_or(header).trim();
    (old_start, old_count, new_start, new_count, header)
}

fn parse_range(value: &str) -> (u32, u32) {
    let mut parts = value.splitn(2, ',');
    let start = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);
    let count = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);
    (start, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(patch: &str) -> FileDiff {
        FileDiff {
            file: "src/main.rs".into(),
            patch: patch.into(),
            additions: 0,
            deletions: 0,
            status: "modified".into(),
            more: false,
        }
    }

    #[test]
    fn patch_parses_unified_hunks() {
        let patch = "diff --git a/src/main.rs b/src/main.rs\n\
--- a/src/main.rs\n\
+++ b/src/main.rs\n\
@@ -1,3 +1,4 @@ fn main\n\
 use std::io;\n\
-old();\n\
+new();\n\
+extra();\n\
 done();\n";
        let parsed = parse_patch(&diff(patch));
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.path(), "src/main.rs");
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (1, 3, 1, 4)
        );
        assert_eq!(hunk.header, "fn main");
        let kinds: Vec<_> = hunk.lines.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            [
                DiffLineKind::Context,
                DiffLineKind::Removed,
                DiffLineKind::Added,
                DiffLineKind::Added,
                DiffLineKind::Context
            ]
        );
        assert_eq!(hunk.lines[1].old_no, Some(2));
        assert_eq!(hunk.lines[2].new_no, Some(2));
        assert_eq!(hunk.lines[4].new_no, Some(4));
    }

    #[test]
    fn patch_handles_rename_without_hunks() {
        let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
        let parsed = parse_patch(&diff(patch));
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.status, Some(VcsChange::Renamed));
        assert_eq!(file.old_path.as_deref(), Some("old.rs"));
        assert_eq!(file.new_path.as_deref(), Some("new.rs"));
        assert!(file.hunks.is_empty());
    }

    #[test]
    fn patch_handles_binary_marker() {
        let patch =
            "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n";
        let parsed = parse_patch(&diff(patch));
        assert!(parsed.binary);
        assert!(parsed.files[0].binary);
    }

    #[test]
    fn patch_handles_crlf() {
        let patch = "diff --git a/a b/a\r\n--- a/a\r\n+++ b/a\r\n@@ -1 +1 @@\r\n-old\r\n+new\r\n";
        let parsed = parse_patch(&diff(patch));
        let line = &parsed.files[0].hunks[0].lines[0];
        assert_eq!(line.text, "old");
        assert!(!line.text.contains('\r'));
    }

    #[test]
    fn patch_survives_malformed_hunk() {
        let patch = "diff --git a/a b/a\n@@ nonsense\n+kept\n@@ -2 +2 @@\n-x\n+y\n";
        let parsed = parse_patch(&diff(patch));
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].hunks.len(), 2);
        assert_eq!(parsed.files[0].hunks[1].lines.len(), 2);
    }

    #[test]
    fn patch_truncates_at_line_limit() {
        let mut patch = String::from("diff --git a/a b/a\n@@ -1,60000 +1,60000 @@\n");
        for _ in 0..60_000 {
            patch.push_str(" line\n");
        }
        let parsed = parse_patch(&diff(&patch));
        assert!(parsed.truncated);
        assert!(parsed.files[0].hunks[0].lines.len() <= MAX_PATCH_LINES);
    }

    #[test]
    fn patch_keeps_no_newline_marker() {
        let patch = "diff --git a/a b/a\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n";
        let parsed = parse_patch(&diff(patch));
        let kinds: Vec<_> = parsed.files[0].hunks[0]
            .lines
            .iter()
            .map(|line| line.kind)
            .collect();
        assert!(kinds.contains(&DiffLineKind::Meta));
    }

    #[test]
    fn patch_reports_server_truncation() {
        let mut value = diff("diff --git a/a b/a\n@@ -1 +1 @@\n-a\n+b\n");
        value.more = true;
        let parsed = parse_patch(&value);
        assert!(parsed.truncated);
    }

    #[test]
    fn patch_handles_added_and_deleted_files() {
        let added = "diff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+hi\n";
        let parsed = parse_patch(&diff(added));
        assert_eq!(parsed.files[0].status, Some(VcsChange::Added));
        assert_eq!(parsed.files[0].old_path, None);

        let deleted = "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let parsed = parse_patch(&diff(deleted));
        assert_eq!(parsed.files[0].status, Some(VcsChange::Deleted));
        assert_eq!(parsed.files[0].new_path, None);
    }
}
