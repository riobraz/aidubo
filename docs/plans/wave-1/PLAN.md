# Plan — Wave 1 execution

Status: in progress · 2026-11-10

## Goal

Execute wave 1 of the spec graph: conversation control (M9), review (M10-01,
M10-02), projects (M11-01), automation (M12-02, M12-03), integrations
(M13-01…04), PTY transport (M14-01), preview (M15-01), and distribution
(M16-01…04), then close gate `G1`.

## Context

- `docs/specs/README.md` — loop protocol, waves, gates.
- `docs/specs/graph.json` — the 21 nodes of this wave.
- `docs/specs/000-conventions.md` — crate boundaries, tests, clean room.
- Wave 0 is merged on `main` (PR #1).

## Order

No node in this wave depends on another, except the milestone logic in M9.
The practical order is milestone by milestone, because they share the
`EngineEvent` match and the workspace event pump:

1. M10-02, M10-01 — review data path.
2. M9-02, M9-01, M9-03, M9-05, M9-04, M9-06 — conversation.
3. M13-04, M13-01, M13-03, M13-02 — integrations.
4. M12-02, M12-03 — automation.
5. M14-01 — PTY transport.
6. M16-04, M16-03, M16-02, M16-01 — distribution.
7. M11-01 — projects and location routing.
8. M15-01 — preview (may be cancelled).

## Test plan

Each node keeps its own unit table and e2e gate. After every node:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The wave gate runs every e2e gate the wave touched plus
`python3 scripts/spec-graph.py --validate`.

## Risks

- The workspace event pump is one exhaustive match; every new `EngineEvent`
  needs an arm. A node that only adds data (like M10-02) may land the arm
  before its view exists, with a comment naming the node that consumes it.
- M11-01 changes the location model for every command; land it last in the
  wave so it does not rebase the rest.
- M15-01 may be cancelled without weakening the product.
