# Plan — Post-MVP specification suite

Status: done · 2026-11-10

## Goal

Turn `docs/ROADMAP.md` into an executable specification suite: one file per
feature, a dependency graph with waves and lanes, explicit parallelism, and a
unit + e2e test contract for every feature.

## Context

- `docs/ROADMAP.md` — feature sequence and API evidence.
- `docs/ARCHITECTURE.md` — crate boundaries and invariants.
- `docs/research/references.md` — OpenChamber 2.7.0 and Monocode Native 0.16
  findings (features, architecture, measured optimizations).
- OpenCode v2.2.6 `/doc/openapi.json`, re-read on 2026-11-10 (139 paths, 147
  event names).

## Scope

In: `docs/specs/` (index, conventions, 29 specs), `docs/specs/graph.json`,
`scripts/spec-graph.py`, `docs/research/references.md`, a pointer from
`ROADMAP.md` and `README.md`.

Out: implementation of any node; new ADRs (each node writes its own if it needs
a crate, a dependency, or a new trust boundary); changes to Rust code.

## Design

- Flat node files named `<ID>-<slug>.md`, one node per file, so a runner can
  claim one file at a time.
- Machine-readable graph with `kind: spec | gate`, `wave`, `deps`, `size`,
  `e2e`, `status`.
- Waves are computed from dependencies and validated against the declared
  values; a gate closes a wave and is never a separate file.
- Shared files (`protocol/lib.rs`, `engine/{lib,supervisor}.rs`,
  `core/lib.rs`, `views/workspace.rs`) are edited append-only with a
  `// spec:<id>` tag; the wave gate resolves ordering.

## Steps

- [x] Re-read the live OpenAPI document and record the exact shapes.
- [x] Write conventions (`000-conventions.md`).
- [x] Write the index with lanes, waves, loop protocol, and gates.
- [x] Write 29 feature specs (M9–M16, E0, T0, M14-02).
- [x] Write `graph.json` and validate cycles, waves, files, and tests.
- [x] Write `scripts/spec-graph.py` (`--validate`, `--ready`, `--waves`,
      `--set-status`).
- [x] Write the reference note.
- [x] Point `ROADMAP.md` and `README.md` at the suite.

## Test plan

- `python3 scripts/spec-graph.py --validate` — 33 nodes, 4 waves, no cycles,
  every spec file exists, has a heading, a status row, and names an e2e test.
- Markdown table lint — no malformed rows.
- `--set-status` exercised on a throwaway graph copy (no repository mutation).
- No Rust change, so the workspace suite is unaffected; `STATUS.md` records
  that it was not re-run for a docs-only change.

## Risks

- The suite can drift from the roadmap; the validator enforces structure, not
  content. The wave gate is the place to re-check the contract.
- Wave 1 is wide (21 nodes). A solo developer should walk lanes, not waves.
- Some endpoints are experimental (`/api/experimental/*`); each such spec says
  so and isolates the surface in one module.
