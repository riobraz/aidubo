# Plan — Wave 0 execution

Status: blocked · 2026-11-10

## Goal

Execute the first wave of the post-MVP spec graph: `E0` (engine and core module
seams), `T0` (shared e2e harness and scripted mock), `M14-02` (VT parser and
grid model), then close the wave with gate `G0`.

## Context

- `docs/specs/README.md` — loop protocol, waves, gate procedure.
- `docs/specs/E0-engine-bus.md`, `T0-e2e-harness.md`, `M14-02-vt-model.md`.
- `docs/specs/000-conventions.md` — crate boundaries, tests, clean room.

## Scope

In: the three nodes and the wave gate.

Out: wave 1 features; any change to the OpenCode engine contract; any reference
source code (clean room).

## Incident 2026-11-10 (blocker)

While splitting `crates/core/src/lib.rs` for E0, a second extraction pass re-read
the already-replaced file and overwrote three generated modules with empty
content. The uncommitted M8 composer code that lived in that file was lost.

- Working `lib.rs` before the split: 2,607 lines. `HEAD`: 1,494 lines.
- Lost: the M8 composer (`PromptDraft`, `PickerState`, `PickerItem`,
  `PickerEffect`, `SNIPPETS`) and the working copies of `sessions`/`transcript`.
- Survived: the M8 test suite (893 lines) and the `status`, `permissions`,
  `util` fragments, copied to
  `/private/var/folders/1m/1fqjz4h12zgbpt_jgpf6_qv80000gn/T/opencode/e0-recovery/`.
- Recovery: **option A**, the author restores `crates/core/src/lib.rs` from a
  backup. No git object, stash, worktree, APFS snapshot, or editor history
  contained it.

## Design (after recovery)

E0 is restarted with a safe procedure:

1. Copy the file to a scratch path and take a checksum before touching it.
2. Split from the **scratch copy**, never from the file being replaced.
3. Verify each generated module ends on a real item boundary (no trailing doc
   comment) before writing anything.
4. Keep the original as `lib.rs.orig` until `cargo test -p harness-core`
   passes; delete it only at the end.
5. Diff with `git diff -M` and confirm the change is move-only.

## Steps

- [x] Restore `crates/core/src/lib.rs` — not delivered; the composer was
      reconstructed clean room instead.
- [x] Verify the reconstruction: `PromptDraft` present and the original 36
      tests pass unchanged.
- [x] Remove the abandoned split modules created during the failed attempt.
- [x] Restart E0 with the safe procedure above; both halves done.
- [x] T0 — done 2026-11-10.
- [x] M14-02 — done 2026-11-10.
- [x] Gate G0 — all three nodes green.

## Test plan

Per node, exactly as written in its spec. On top of that, for E0: the whole
existing suite must pass with no test renamed, deleted, or weakened, and the
diff must be move-only.

## Risks

- A partial restore leaves the workspace uncompilable; the verification step
  gates the restart.
- `harness-views` imports the composer API; any divergence in the restored file
  will surface as a compile error in the workspace check.
