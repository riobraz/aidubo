#!/usr/bin/env bash
#
# Build Linux release artifacts: an AppImage and a .deb.
#
# The bundle ships the OpenCode sidecar under usr/lib/rust-harness/ with an
# opencode.sha256 manifest written after the copy and before the archive is
# sealed. There is no sandbox: the sidecar must spawn processes and read the
# project.
#
# Common overrides:
#   VERSION=0.1.0                  defaults to the workspace version
#   OPENCODE_BIN=/path/to/opencode sidecar to bundle (default: `opencode` on PATH)
#   APPIMAGETOOL=/path/to/tool     appimagetool binary (default: on PATH)
#   ARCH=x86_64|aarch64            artifact architecture (default: host)
#   SKIP_APPIMAGE=1 / SKIP_DEB=1   build only one format
#
# Output: dist/rust-harness-<version>-<arch>.AppImage and .deb, each with a
# .sha256 sidecar.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
HOST_ARCH="$(uname -m)"
ARCH="${ARCH:-$HOST_ARCH}"
DIST="$ROOT/dist"
APPDIR="$DIST/AppDir"
DEBROOT="$DIST/debroot"
APPIMAGE="$DIST/rust-harness-$VERSION-$ARCH.AppImage"
DEB="$DIST/rust-harness-$VERSION-$ARCH.deb"

case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) echo "error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

resolve_executable() {
  [[ -x "$1" ]] || fail "not an executable: $1"
  python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

[[ "$(uname -s)" == "Linux" ]] || fail "Linux artifacts must be built on Linux"
[[ "$ARCH" == "$HOST_ARCH" ]] || fail "cross-builds are not supported ($HOST_ARCH host, $ARCH target)"

step "Building Rust Harness $VERSION"
cargo build -p rust-harness --release
BIN="$ROOT/target/release/rust-harness"
[[ -x "$BIN" ]] || fail "missing release binary: $BIN"

step "Resolving the OpenCode sidecar"
if [[ -n "${OPENCODE_BIN:-}" ]]; then
  SIDECAR="$(resolve_executable "$OPENCODE_BIN")"
elif command -v opencode >/dev/null 2>&1; then
  SIDECAR="$(resolve_executable "$(command -v opencode)")"
else
  fail "opencode not found; install it or set OPENCODE_BIN"
fi

step "Assembling the AppDir"
rm -rf "$APPDIR" "$DEBROOT"
mkdir -p \
  "$APPDIR/usr/bin" \
  "$APPDIR/usr/lib/rust-harness" \
  "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/256x256/apps"

install -m 755 "$BIN" "$APPDIR/usr/bin/rust-harness"
install -m 755 "$SIDECAR" "$APPDIR/usr/lib/rust-harness/opencode"
install -m 644 packaging/linux/rust-harness.desktop \
  "$APPDIR/usr/share/applications/rust-harness.desktop"
if [[ -f packaging/linux/rust-harness.png ]]; then
  install -m 644 packaging/linux/rust-harness.png \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps/rust-harness.png"
fi

cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/rust-harness" "$@"
APPRUN
chmod 755 "$APPDIR/AppRun"

step "Recording the sidecar checksum"
# After any rewrite and before the archive is sealed.
(
  cd "$APPDIR/usr/lib/rust-harness"
  sha256sum opencode > opencode.sha256
)
[[ -s "$APPDIR/usr/lib/rust-harness/opencode.sha256" ]] \
  || fail "the sidecar manifest is empty"

if [[ "${SKIP_APPIMAGE:-0}" != "1" ]]; then
  step "Building the AppImage"
  APPIMAGETOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  [[ -n "$APPIMAGETOOL" && -x "$APPIMAGETOOL" ]] \
    || fail "appimagetool not found; install it or set APPIMAGETOOL"
  rm -f "$APPIMAGE"
  # appimagetool refuses to overwrite and needs an explicit architecture.
  ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}" \
    "$APPIMAGETOOL" --no-appstream "$APPDIR" "$APPIMAGE"
  chmod 755 "$APPIMAGE"
  sha256sum "$APPIMAGE" > "$APPIMAGE.sha256"
fi

if [[ "${SKIP_DEB:-0}" != "1" ]]; then
  step "Building the .deb"
  mkdir -p "$DEBROOT/DEBIAN"
  cp -a "$APPDIR/usr" "$DEBROOT/usr"
  INSTALLED_SIZE="$(du -sk "$DEBROOT/usr" | awk '{print $1}')"
  cat > "$DEBROOT/DEBIAN/control" <<CONTROL
Package: rust-harness
Version: $VERSION
Section: devel
Priority: optional
Architecture: $DEB_ARCH
Installed-Size: $INSTALLED_SIZE
Maintainer: Rust Harness contributors
Depends: libc6, libfontconfig1, libfreetype6, libxkbcommon0, libwayland-client0, libx11-6, libxcb1, libgl1
Description: Native desktop client for the OpenCode agent
 Rust Harness supervises a bundled OpenCode sidecar and renders the session
 transcript, tools, permissions, and project state with GPUI.
CONTROL
  rm -f "$DEB"
  dpkg-deb --build --root-owner-group "$DEBROOT" "$DEB" >/dev/null
  sha256sum "$DEB" > "$DEB.sha256"
fi

step "Done"
for artifact in "$APPIMAGE" "$DEB"; do
  if [[ -f "$artifact" ]]; then
    du -h "$artifact"
  fi
done
printf '    sidecar: %s\n' "$SIDECAR"
