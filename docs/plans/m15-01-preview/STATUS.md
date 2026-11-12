# M15-01 — Status

Status: done (cancelled)
Last updated: 2026-11-11

## Completed

- Feasibility check of the pinned GPUI surface for hosting an `NSView`.
  `gpui-kit = "=0.9.0"` depends on `gpui-pre 0.3.11`; its `Window` exposes only
  a raw window handle and a private platform window. No element, layout, or
  lifecycle API hosts a native child view, so the spec's "stop and record the
  finding" branch applies.
- Node marked `cancelled` in `docs/specs/M15-01-preview.md` and
  `docs/specs/graph.json`. No code or protocol surface was added.

## In progress / next

- Nothing. Gate G1 is the remaining wave-1 step.

## Blockers

None. Nothing depends on M15-01.

## Verification

```text
python3 scripts/spec-graph.py --validate    ok: 33 nodes, 4 waves
cargo test --workspace                      ok (unchanged by this node)
cargo fmt --all -- --check                  ok
cargo clippy --workspace --all-targets -- -D warnings  ok
```

Manual: none; no preview panel exists to exercise.
