# M10-03 — Virtualized diff view

| | |
| --- | --- |
| Status | blocked (M10-02) |
| Wave | 2 |
| Size | M |
| Depends on | M10-02, M10-01 |
| Parallel with | M11-02, M12-01, M14-03 |
| Owns | `crates/views/src/diff_view.rs`, `crates/ui/src/diff.rs` |
| Shared touch | `views/src/workspace.rs` |

## 1. Goal

A 5,000-line patch scrolls at frame rate. Only the visible rows are built;
syntax highlighting is limited to the visible range; selecting another file
replaces the model instead of re-parsing per frame.

## 2. Input

`ParsedDiff` from M10-02. The view never sees raw patch text.

## 3. Design

Flatten once per selection into `Vec<DiffRow>`:

```rust
pub enum DiffRow {
    FileHeader { file: usize, path: String, status: VcsChange, additions: u64, deletions: u64 },
    HunkHeader { file: usize, hunk: usize, text: String },
    Line { file: usize, hunk: usize, line: usize },
    Notice { file: usize, text: String },
}
```

- `DiffRow::Line` resolves its text and kind by index; no string cloning in
  `Render`.
- A collapsed file keeps its header and drops its hunk/line rows from the flat
  list (rebuild the index on toggle, not per frame).
- Visible range = viewport rows ± 8 overscan. Highlighting is computed for
  that range only, cached by `(file_id, revision)`.
- Unified is the default and the only required layout; a side-by-side toggle is
  allowed but must not change the row model.
- Empty patch, `binary`, and `truncated` render a notice row, never a blank
  panel.
- Line numbers: old number on removed/context, new on added/context, both on
  context.

## 4. Interaction

| Key | Action |
| --- | --- |
| Up/Down, j/k | move one row |
| PgUp/PgDn | one viewport |
| n/p | next/previous file |
| Enter/Space | toggle the focused file |
| Cmd/Ctrl+C | copy the focused line text |
| Esc | return focus to the Changes list |

## 5. Performance budget

- Rebuild the flat index ≤ 1 per selection/collapse.
- ≤ 1 `cx.notify()` per frame while streaming a diff.
- 5,000 lines, 120 Hz: no frame over 16 ms in a release build; record the
  measurement in `STATUS.md`.
- Memory: the flat index must not clone patch text; it stores indices.

## 6. Non-goals

Word-level diffs, image diffs, editing from the diff, staging buttons.

## 7. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-core` | `diff_rows_map_visible_range` | index math with overscan |
| `harness-core` | `diff_rows_collapse_removes_hunk_rows` | toggle correctness |
| `harness-core` | `diff_rows_line_numbers_follow_kind` | old/new numbering |
| `harness-views` | `diff_view_builds_visible_rows_only` | element count ≤ viewport + overscan |
| `harness-views` | `diff_view_binary_shows_notice` | no empty panel |

E2E: the data path is covered by M10-02's engine test. The view is covered by
the two GPUI tests above; a screenshot is attached to the plan for review.

## 8. Acceptance

- [ ] 5k-line patch meets the frame budget.
- [ ] Collapsing a file does not re-parse the patch.
- [ ] Binary and truncated files show a notice.
- [ ] Keyboard-only navigation works end to end.

## 9. Risks

GPUI list virtualization APIs differ from the web reference. If the kit's list
cannot virtualize a heterogeneous row enum, build a fixed-height row model and
render variable-height rows as fixed with a measured max.
