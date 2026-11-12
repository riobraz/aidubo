# M16-01 — Linux packaging

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | M |
| Depends on | E0 |
| Parallel with | M16-02 … M16-04 |
| Owns | `scripts/package-linux.sh`, `.github/workflows/release-linux.yml`, `packaging/linux/` |
| Shared touch | `Makefile`, `README.md` |

## 1. Goal

A Linux build of the same app, with the same checksummed-sidecar rule as
macOS: AppImage and deb, a manifest that records the sidecar hash, and a smoke
test that proves the bundle starts, verifies the sidecar, and exits cleanly.

## 2. Design

- `scripts/package-linux.sh` mirrors `package-macos.sh`: build release, copy
  the sidecar to `usr/lib/rust-harness/opencode`, write `opencode.sha256`,
  assemble an AppDir, run `appimagetool`, and build a `.deb` with `dpkg-deb`.
- `locate_binary` gains the bundle path for Linux (`/usr/lib/rust-harness/`)
  and keeps the order bundle → `RUST_HARNESS_OPENCODE_BIN` → `PATH`.
- The manifest is written **after** any binary rewriting, before the archive.
- No sandbox: the sidecar must spawn processes and read the project.
- CI builds on `ubuntu-24.04` with a pinned `@opencode/cli` version, and
  refuses a bundle whose sidecar hash does not verify at launch.

## 3. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-engine` | `locate_prefers_bundle_then_env_then_path` | lookup order |
| `harness-engine` | `manifest_rejects_wrong_hash` | checksum gate |
| `harness-engine` | `manifest_missing_blocks_start` | fail closed |

E2E (`scripts/e2e-linux.sh`, gate `RUST_HARNESS_PACKAGING_E2E`): build the
AppImage, extract it, verify the manifest exposes the bundled sidecar, run it
with `RUST_HARNESS_AUTOQUIT_MS=3000` under `xvfb-run`, assert exit code 0 and
no `opencode` child left (`pgrep -f opencode` empty). Record the AppImage and
deb sizes in `STATUS.md`.

## 4. Acceptance

- [ ] AppImage and deb both launch on a clean Ubuntu 24.04.
- [x] A tampered sidecar blocks startup with a clear message.
- [ ] Quit leaves no sidecar process.
- [ ] CI produces the artifacts and the smoke test runs on every release.

The two unchecked items need a Linux host or a CI run. Unit tests prove the
checksum gate; the smoke script asserts the process and clean-exit rules but
was not executed here (this host is macOS).

## 5. Non-goals

Windows, Flatpak, Snap, auto-update (M16-02), arm64 cross-builds on x64 hosts.

## 6. Implementation notes (2026-11-11)

- `locate_binary` now searches packaged sidecars before `PATH`: macOS
  `Contents/Resources/opencode`, then Linux `usr/lib/rust-harness/opencode`.
  An explicit path argument still wins; a verified bundle now wins over
  `RUST_HARNESS_OPENCODE_BIN` so environment poisoning cannot replace an
  installed release's sidecar.
- Fail-closed split: a packaged resource dir requires `opencode.sha256` and
  reports `ManifestMissing`; a developer binary dropped beside the executable
  may omit it. Tests: `locate_prefers_bundle_then_env_then_path`,
  `manifest_rejects_wrong_hash`, `manifest_missing_blocks_start`,
  `linux_bundle_directory_is_found`,
  `dev_sibling_binary_without_manifest_is_accepted`.
- `scripts/package-linux.sh` builds one AppDir, writes the manifest after the
  sidecar copy, then produces the AppImage (`appimagetool`) and the deb
  (`dpkg-deb --root-owner-group`). SHA-256 files are written for both
  artifacts. It refuses non-Linux hosts and cross-architecture builds.
- The AppImage is executed through `AppRun`, which resolves its own directory,
  so both the AppImage-extracted tree and the installed deb find the sidecar
  relative to `usr/bin`.
- `.github/workflows/release-linux.yml` builds on `ubuntu-24.04`, pins
  `@opencode/cli@2.2.6`, installs `appimagetool` (continuous build, run with
  `APPIMAGE_EXTRACT_AND_RUN=1`), runs the packaging e2e, uploads both
  artifacts, and attaches them to a tag release.
- `make package-linux` and `make test-packaging-linux` wrap the scripts.
  Artifact sizes are printed by `du -h`; they are not recorded here because no
  Linux artifact was built on this host.

## 7. Risks

FUSE and sandbox differences across distros; `xvfb` flakiness. The smoke test
retries once and reports the log on failure.
