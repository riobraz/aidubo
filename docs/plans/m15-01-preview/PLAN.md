# M15-01 — Preview panel and element picker

Status: cancelled · 2026-11-11

## Goal

A preview panel beside the chat loads a local dev server, and picking an
element inserts a stable selector plus a short snippet into the composer.
Remote URLs are blocked by default.

## Context

- `docs/specs/M15-01-preview.md`
- `docs/specs/000-conventions.md` §11 (no XL nodes)
- `docs/ARCHITECTURE.md` — crate boundaries and the MVP non-goals

## Scope / non-goals

The spec marks this node as the first cancellation candidate: it must not
block anything else. `M15-01a` (webview host) and `M15-01b` (element picker)
were defined as the required split; neither started.

## Design

The spec requires a `WKWebView` in an `NSView` hosted by GPUI, and says to
stop and record the finding if GPUI cannot host an `NSView`. The feasibility
check came first, before any UI or protocol work.

## Finding

The pinned GPUI surface (`gpui-kit = "=0.9.0"`, `gpui-pre 0.3.11`) has no
hosted-native-view API:

- `Window` implements `HasWindowHandle` and wraps a private `PlatformWindow`;
  there is no public method that attaches an `NSView` to GPUI's element tree.
- No element accepts a platform view or reports a layout bounds bridge that
  would let the app place and clip a child `NSView`.
- `gpui-component` and `gpui-kit` ship no webview component.
- Adding a `WKWebView` directly to the window's content view would bypass
  GPUI layout, input routing, focus, and window-close destruction. That is the
  "fake browser" the spec forbids, and it cannot satisfy the acceptance
  criteria for resize, close, and process teardown.

Per the spec, the node is cancelled rather than worked around. No command,
protocol type, or crate changed, so nothing else is blocked.

## Steps

- [x] Check the pinned GPUI crates for hosted-native-view support.
- [x] Record the finding in `docs/specs/M15-01-preview.md` and the graph.
- [x] Mark the node `cancelled` without weakening any other node.
- [ ] Revive as M15-01a/M15-01b if a supported host API appears.

## Test plan

None: no code was added. The cancellation path itself is covered by
`python3 scripts/spec-graph.py --validate` accepting a `cancelled` node.

## Risks

- The product ships without an in-app preview; users still open a browser
  themselves. This is an accepted scope reduction, not a regression.
- If GPUI later gains hosted native views, the picker script and URL policy
  still need the split described in the spec.
