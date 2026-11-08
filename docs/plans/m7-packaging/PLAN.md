# M7 — macOS packaging

## Goal

Produce `Rust Harness.app`: a macOS bundle that contains both the GPUI binary and
a pinned OpenCode sidecar, is signed (ad-hoc locally, Developer ID for release),
can be notarized, installs into `/Applications`, launches, and leaves no orphaned
sidecar behind on quit.

## Context

- `docs/ARCHITECTURE.md` §4 (sidecar lookup), §9 (pinned, checksummed sidecar),
  §10 (packaging), §12 (M7 acceptance).
- `crates/engine/src/locate.rs` already prefers a sibling of the executable, but
  not `Contents/Resources`, and does not verify a checksum.
- No bundle script, icon, or release workflow exists yet; `.github/workflows/*`
  are empty placeholders.

## Scope / non-goals

In scope:

- A `scripts/package-macos.sh` bundler: release build, `.app` layout, `Info.plist`,
  bundled sidecar, SHA-256 manifest, optional universal build, ad-hoc or
  Developer ID signing, notarization/stapling, verification.
- Sidecar lookup in `harness-engine` for the bundle layout, with checksum
  verification of the bundled copy.
- `make bundle` / `make install` targets.
- A tag-driven release workflow that builds the bundle with a pinned OpenCode
  version and signs when the certificate secrets are present.
- Docs: README packaging and env vars, ARCHITECTURE §10 as-built notes.

Not in scope:

- Linux packaging, DMG/auto-update, App Store submission, sandboxing.
- First-run download of the sidecar; the release bundle carries it.
- A designed app icon (a placeholder may be dropped at `packaging/AppIcon.icns`).
- Live Gatekeeper testing on a quarantined machine (needs a Developer ID).

## Design

### Bundle layout

```
Rust Harness.app/
  Contents/
    Info.plist
    PkgInfo
    MacOS/rust-harness
    Resources/opencode            # sidecar, signed separately
    Resources/opencode.sha256     # shasum -a 256 manifest
    Resources/AppIcon.icns        # optional
```

The architecture puts the sidecar under `Contents/Resources`. `locate_binary`
therefore checks, in order: explicit path, `RUST_HARNESS_OPENCODE_BIN`, a sibling
of the executable (`Contents/MacOS`, kept for development), `Contents/MacOS/../Resources`,
`PATH`, and the known Bun/Homebrew locations.

### Integrity

The bundler records the sidecar's SHA-256 next to it. When the located binary is
one of the bundle candidates and the manifest exists, `locate_binary` hashes the
file and returns a new `LocateError::ChecksumMismatch` on a mismatch; a missing
manifest is allowed so `cargo run` keeps working. Verification happens before
the sidecar is spawned, satisfying §9 without trusting the app bundle alone.

### Signing and notarization

- Local builds: ad-hoc signing (`codesign -s -`) so the bundle structure and
  nested-code ordering are exercised.
- Release: `SIGN_IDENTITY` selects a Developer ID Application certificate;
  `codesign --options runtime --timestamp` signs the sidecar first, then the app.
  Hardened runtime, no sandbox (the sidecar must spawn processes and read the
  project directory).
- Notarization: `NOTARY_PROFILE` (keychain profile) or `APPLE_ID` +
  `APPLE_TEAM_ID` + `APPLE_APP_PASSWORD`; zip with `ditto`, submit with
  `xcrun notarytool --wait`, `xcrun stapler staple`, then `spctl --assess`.
- Universal (`UNIVERSAL=1`): build both Rust targets and `lipo` them; the
  sidecar must be universal or be supplied as `OPENCODE_X86_64_BIN` +
  `OPENCODE_ARM64_BIN`, otherwise the script fails loudly.

### CI

`.github/workflows/release.yml` runs on `v*` tags or manual dispatch: installs a
pinned `@opencode/cli`, runs the same script, imports the certificate from
secrets when present (skipping signing otherwise), and uploads the zipped app
with its checksum.

## Steps

- [x] Create the plan and stop ignoring `docs/plans`.
- [x] Extend `locate_binary` with `Contents/Resources` and checksum verification,
      plus unit tests.
- [x] Log the resolved sidecar path at debug level for bundle smoke tests.
- [x] Write `scripts/package-macos.sh`.
- [x] Add `make bundle` / `make install`.
- [x] Add `.github/workflows/release.yml` (and `ci.yml`).
- [x] Bundle, sign ad-hoc, verify nested code, and smoke-run the app from
      `dist/` with a unique temp project; confirm the bundled sidecar is used and
      no process remains.
- [x] Update README and ARCHITECTURE §10; record verification in `STATUS.md`.

## Test plan

- `cargo test --workspace` (new locate tests cover manifest match, mismatch, and
  the Resources layout).
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- `make bundle`, then:
  - `plutil -lint` on the generated `Info.plist`;
  - `codesign --verify --deep --strict` on the app;
  - run `dist/Rust Harness.app/Contents/MacOS/rust-harness` with
    `RUST_HARNESS_AUTOQUIT_MS`, assert the log names
    `.../Contents/Resources/opencode`, and `pgrep -fl "opencode serve"` is empty;
  - tamper the sidecar and confirm startup fails with the checksum message
    (then re-bundle);
  - measure the bundle size and record it.
- `make install` is manual/optional (writes `/Applications`).

## Risks

- Executables in `Contents/Resources` are accepted by notarization only when
  signed with hardened runtime and a timestamp; the script signs them explicitly
  and verifies. If Apple rejects the location, move the sidecar to
  `Contents/MacOS` (lookup already supports it).
- Universal builds need both Rust targets and a matching sidecar; the script
  fails with a clear message instead of producing a broken bundle.
- Real signing/notarization cannot be verified here without a Developer ID; the
  script is exercised with ad-hoc signing and documented for release.
- The sidecar is ~235 MB, so the bundle is large; the DMG/zip size is recorded
  and distribution format is left to a later decision.
