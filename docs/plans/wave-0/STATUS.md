# Status — Wave 0 execution

Status: done

Last updated: 2026-11-10

## Completed

- 2026-11-10 — Clean-room rule added to `AGENTS.md`,
  `docs/specs/000-conventions.md` §13, and `docs/specs/README.md`.
- 2026-11-10 — **T0 done.** `crates/engine/tests/support/{mod,mock}.rs`: temp
  project, engine startup, event waits, clean shutdown, and a std-only
  OpenAI-compatible scripted mock. `tests/sidecar.rs` switched to the shared
  helpers; `tests/T0.rs` adds 6 tests, including a real v2.2.6 sidecar
  streaming text from the in-process mock.
- 2026-11-10 — **M14-02 done.** New crate `harness-term` (ADR 0002): VT parser,
  grid, scrollback, alternate screen, SGR, bracketed paste. 14 tests, zero
  dependencies.
- 2026-11-10 — **E0 done.** `engine/api.rs` split into `features/*` (84-line
  `api.rs`); `harness-core` split into `status`, `sessions`, `transcript`,
  `permissions`, `composer`, `util` with a 22-line re-exporting `lib.rs`.
  Move-only where the source existed; the composer was reconstructed.
- 2026-11-10 — **Recovery.** The restore (option A) never arrived, so the M8
  composer was rebuilt clean room from the surviving 36-test contract and the
  `harness-views` call sites. `crates/core/src/tests.rs` is the original test
  suite and passes unchanged.
- 2026-11-10 — **Bug fixed:** OpenCode v2.2.6 reads `OPENCODE_PASSWORD`, so the
  supervisor now sets both names. Before this, every `/api/*` call answered 401.

## In progress / next

- Wave 0 is closed; wave 1 starts from M10-01, M10-02, M9-01…06, M12-02,
  M12-03, M13-01…04, M14-01, M15-01, M16-01…04 (see `docs/specs/README.md`).

## Blockers

None. B1 is resolved by the clean-room reconstruction; the lost original file
was not recovered.

## Verification

```sh
$ cargo test --workspace
passed: 97  failed: 0

$ cargo clippy --workspace --all-targets -- -D warnings
Finished

$ cargo fmt --all -- --check
ok

$ python3 scripts/spec-graph.py --validate
ok: 33 nodes, 4 waves
```
