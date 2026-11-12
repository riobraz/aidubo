# Status — Wave 3 execution

Status: in progress

Last updated: 2026-11-12

## Completed

- 2026-11-12 — Plan opened. Wave 2 is closed and merged to `main` as
  `5daf7c5` (PR #5): M14-03 shipped the terminal dock and gate `G2` passed
  every local gate and e2e. `python3 scripts/spec-graph.py --validate` reports
  `ok: 33 nodes, 4 waves`; M11-03 was re-marked `in-progress` from `blocked`
  because its only dependency, M11-02, is done.

## In progress / next

- M11-03 (multi-run and comparison): starting with the pure `harness-core`
  model and its four tests, then the engine executor, persistence, view, and
  the three-run e2e.

## Blockers

- None.

## Verification

```sh
python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# green on main at 5daf7c5, before any wave-3 change
```
