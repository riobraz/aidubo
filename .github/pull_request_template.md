## Summary

<!-- What changes, and why? Keep the title imperative and capitalized. -->

- Closes #
- Plan: `docs/plans/<plan-name>/STATUS.md` <!-- remove if not applicable -->

## Changes

<!-- Summarize by crate; delete lines that do not apply. -->

- `harness-protocol`:
- `harness-core`:
- `harness-engine`:
- `harness-views` / `harness-ui`:
- `harness-app`:
- Docs / tooling:

## Testing

<!-- Paste the commands you actually ran and their results. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Manual check (describe below)
- [ ] Sidecar e2e (describe which mock env vars were set, if any)

Manual verification / notes:

```text

```

## Checklist

- [ ] Title is imperative, correctly capitalized, and has no `fix:`/`feat:` prefix
- [ ] Change is focused; unrelated refactors are in a separate PR
- [ ] Read `docs/ARCHITECTURE.md`; crate boundaries are respected
- [ ] Protocol changes tolerate unknown variants and missing fields
- [ ] Behavior changes have tests (unit, and integration where it crosses the engine)
- [ ] Plan `STATUS.md` updated, or no plan applies
- [ ] README / ARCHITECTURE updated if status, behavior, or commands changed
- [ ] No credentials, prompts, message content, session data, or `target/` committed
- [ ] Security considered: loopback only, no auto-approved permissions, untrusted output treated as text

## Screenshots / recordings

<!-- For UI changes; include light and dark if relevant. -->
