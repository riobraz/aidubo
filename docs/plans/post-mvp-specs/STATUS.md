# Status — Post-MVP specification suite

Status: done

Last updated: 2026-11-10

## Completed

- 2026-11-10 — Re-read OpenCode v2.2.6 `/doc/openapi.json` from a throwaway
  loopback server (139 paths) and confirmed the 147 event names used by the
  specs. Server stopped; no process left listening.
- 2026-11-10 — `docs/specs/000-conventions.md` (crate boundaries, protocol,
  engine, core, rendering budgets, security, tests, done criteria).
- 2026-11-10 — `docs/specs/README.md` (loop protocol, lanes, waves, shared-file
  rule, wave gates, env gates, status table).
- 2026-11-10 — 29 specs: E0, T0, M14-02, M9-01…06, M10-01…03, M11-01…03,
  M12-01…03, M13-01…04, M14-01, M14-03, M15-01, M16-01…04.
- 2026-11-10 — `docs/specs/graph.json` (33 nodes: 29 specs + 4 gates) and
  `scripts/spec-graph.py`.
- 2026-11-10 — `docs/research/references.md` (OpenChamber and Monocode notes
  plus the mapping of what the specs adopt).
- 2026-11-10 — `ROADMAP.md` and `README.md` point at the suite.

## In progress / next

Nothing in this plan. Execution starts with `python3 scripts/spec-graph.py
--ready` → E0, T0, M14-02.

## Blockers

None.

## Verification

```sh
$ python3 scripts/spec-graph.py --validate
ok: 33 nodes, 4 waves

$ python3 scripts/spec-graph.py --waves
wave 0 (3): E0 M14-02 T0   gate: G0
wave 1 (21): M9-01 … M16-04   gate: G1
wave 2 (4): M10-03 M11-02 M12-01 M14-03   gate: G2
wave 3 (1): M11-03   gate: G3

$ python3 scripts/spec-graph.py --ready
3 ready: E0, M14-02, T0
```

- Markdown table lint over `docs/specs/*.md`: no malformed rows.
- `--set-status` exercised on a throwaway graph copy; the repository graph was
  not mutated.
- No Rust file changed; `cargo test --workspace` was not re-run for this
  docs-only change.
