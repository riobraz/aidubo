# M7 — macOS packaging: status

- **Status:** done
- **Last updated:** 2026-11-08

## Completed

- [x] Plan added; `.gitignore` no longer hides `docs/plans` and ignores `dist/`.
- [x] `locate_binary` prefers `Contents/MacOS`, then `Contents/Resources`, and
      verifies `opencode.sha256` when present; new `ChecksumMismatch` error and
      three unit tests (match, missing manifest, tampered binary).
- [x] Debug log of the resolved sidecar path and of a startup stop reason.
- [x] `scripts/package-macos.sh`: release build, `.app` layout, generated and
      linted `Info.plist`, bundled sidecar, checksum after signing, ad-hoc or
      Developer ID signing with hardened runtime/timestamp, notarization and
      stapling, universal builds, optional icon, `ARCHIVE=1` zip.
- [x] `make bundle` / `make install`.
- [x] `.github/workflows/release.yml` (tag-driven, pinned OpenCode, optional
      signing via secrets) and `.github/workflows/ci.yml`.
- [x] README packaging section and shortcuts unaffected; ARCHITECTURE §10
      rewritten as-built; AGENTS/CONTRIBUTING mention `make bundle`.

## In progress / next

None for M7. Follow-ups belong to distribution work: verify a
Developer ID–signed, notarized bundle on a quarantined machine, run `make
install` on a real machine, and decide whether releases ship universal builds
and a designed icon.

## Blockers

None. A Developer ID Application certificate and notarization credentials were
not available during implementation, so only ad-hoc signing was exercised; the
release workflow has not run on GitHub Actions yet.

## Verification

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 24 core, 12 engine unit (3 new locate tests), 5
  sidecar, 11 protocol; all pass.
- `scripts/package-macos.sh` — produced `dist/Rust Harness.app` (265 MB) with
  the sidecar at `Contents/Resources/opencode`; `plutil -lint` and
  `codesign --verify --deep --strict` pass (ad-hoc).
- Bundle smoke run with `RUST_HARNESS_AUTOQUIT_MS=6000`: log shows
  `located the OpenCode binary binary=…/Rust Harness.app/Contents/Resources/opencode`,
  readiness probe answered, event stream connected, exit 0, no sidecar left
  running.
- Tamper check: appending a byte to the bundled sidecar produced
  `engine stopped The bundled OpenCode binary failed its integrity check.`
  The bundle was rebuilt afterwards.
- `ARCHIVE=1` produced `dist/RustHarness-0.1.0.zip` (119 MB) with a matching
  `.sha256`; workflow YAML parses.
