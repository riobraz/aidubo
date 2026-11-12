# Status — Wave 2 execution

Status: draft

Last updated: 2026-11-12

## Completed

- 2026-11-12 — Plan opened. Wave 1 is closed and merged to `main` as
  `297b2eb` (PR #2). `python3 scripts/spec-graph.py --validate` reports
  `ok: 33 nodes, 4 waves`; `--ready` lists M11-02, M12-01, and M14-03. The
  per-spec headers for those three nodes said `blocked (<dependency>)` even
  though every dependency is done and the central status table already said
  `ready`, so the headers were corrected to `ready`. No node is claimed yet.

## In progress / next

- Nothing claimed. Recommended order: M11-02 (worktrees), M12-01 (goals),
  M14-03 (terminal dock). M11-02 is the largest and unblocks M11-03 in wave 3.
- Each node follows the spec loop in `docs/specs/README.md` §2: claim, contract,
  core, engine, view, gates, e2e, close.

## Blockers

- None for this wave. M11-03 (wave 3) stays blocked until M11-02 closes.
- The Linux packaging smoke test remains owned by `release-linux.yml` and has
  not run on this macOS host; it does not block wave 2.

## Verification

```sh
python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves

python3 scripts/spec-graph.py --ready
# 3 ready:
#   M11-02   wave 2  Worktrees
#   M12-01   wave 2  Goals: keep working until verified
#   M14-03   wave 2  Terminal dock
```

No code gates have run for wave 2 yet; nothing in this wave has been
implemented.
