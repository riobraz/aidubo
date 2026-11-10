# M15-01 — Preview panel and element picker (macOS, optional)

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | XL → split before starting |
| Depends on | T0 |
| Parallel with | any lane (isolated) |
| Owns | `crates/views/src/preview.rs`, `crates/app/src/webview.rs` |
| Shared touch | `views/src/workspace.rs`, `app/src/main.rs` |

## 1. Goal

A preview panel beside the chat loads a local dev server, and picking an
element inserts a stable selector plus a short snippet into the composer as a
reference. Remote URLs are blocked by default.

This node is the first candidate for cancellation. It must not block anything
else.

## 2. Split before starting

`XL` nodes are not allowed (000-conventions §11). Split into:

- **M15-01a** — webview host: `WKWebView` in a GPUI-hosted native view, URL bar,
  reload, close, non-local blocking. E2E: manual checklist.
- **M15-01b** — element picker: injected script, selector computation, message
  bridge, composer insertion. E2E: unit tests for the payload.

Both are required for the feature; neither is required for the product.

## 3. Design

- `objc2` + `objc2-web-kit` (exact pins) create a `WKWebView` in an `NSView`
  hosted by GPUI. If GPUI cannot host an `NSView`, stop and record the finding;
  do not build a fake browser.
- Policy: scheme `http`/`https` only; host `localhost`, `127.0.0.1`, `::1`, or
  `*.localhost`; anything else shows "Blocked" with an explicit "Open anyway"
  that requires a second click.
- No port scanning. The URL bar starts empty with a hint.
- Picker script is injected **only** into this webview after load:
  `document.elementFromPoint` → build a selector (`#id`, `[data-testid]`,
  `tag.class:nth-of-type`), plus up to 200 characters of text.
- The picked payload is inserted as text `@preview <selector>` plus a
  `file`-less attachment note; it is never executed and never treated as an
  instruction.
- Closing the panel destroys the webview and its process.

## 4. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-core` | `preview_url_policy_blocks_remote` | localhost only |
| `harness-core` | `preview_url_policy_allows_local_ports` | 5173 etc. |
| `harness-core` | `preview_picker_payload_is_sanitized` | selector/length caps |
| `harness-core` | `preview_payload_is_plain_text` | no markdown/HTML injection |
| `harness-app` | `webview_destroy_stops_process` | no orphan (manual on CI) |

E2E: none automated. The manual checklist in `STATUS.md` covers: load
`localhost:5173`, resize, reload, pick an element, close, and confirm the
process is gone (`ps`).

## 5. Acceptance

- [ ] `localhost:5173` renders and survives resize.
- [ ] A remote URL is blocked without an explicit override.
- [ ] Picking inserts a selector and snippet, not executable text.
- [ ] Closing destroys the webview.
- [ ] If the host integration fails, the node is marked cancelled with the
      finding and nothing else is blocked.

## 6. Risks

GPUI hosted-native-view support is unproven; this is macOS-only and adds a
process and memory surface. Page content is untrusted: no automatic link
opens, no downloads, no JS bridge beyond the picker message.
