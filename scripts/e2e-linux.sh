#!/usr/bin/env bash
#
# M16-01 packaging smoke test (Linux only).
#
# Gate: RUST_HARNESS_PACKAGING_E2E=1. Without it (or off Linux) this prints a
# skip line and exits 0, so the regular test story stays usable on other hosts.
#
# Steps: build the AppImage unless SKIP_BUILD=1, extract it, verify the sidecar
# checksum manifest, launch the AppImage's AppRun and the deb's binary under
# xvfb with RUST_HARNESS_AUTOQUIT_MS=3000, assert a clean exit, and assert no
# sidecar process remains. Each launch is retried once; the log is printed on
# failure.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "${RUST_HARNESS_PACKAGING_E2E:-0}" != "1" ]]; then
  echo "skipping Linux packaging e2e: set RUST_HARNESS_PACKAGING_E2E=1 to run it"
  exit 0
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "skipping Linux packaging e2e: Linux only"
  exit 0
fi

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

command -v xvfb-run >/dev/null 2>&1 || fail "xvfb-run is required"
command -v timeout >/dev/null 2>&1 || fail "timeout is required"
command -v dpkg-deb >/dev/null 2>&1 || fail "dpkg-deb is required"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ -n "${APPIMAGE:-}" ]]; then
  APPIMAGE="$(realpath "$APPIMAGE")"
elif [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  step "Building the Linux artifacts"
  scripts/package-linux.sh
  APPIMAGE="$(ls -t dist/rust-harness-*.AppImage 2>/dev/null | head -n 1 || true)"
else
  APPIMAGE="$(ls -t dist/rust-harness-*.AppImage 2>/dev/null | head -n 1 || true)"
fi
[[ -n "${APPIMAGE:-}" && -f "$APPIMAGE" ]] || fail "no AppImage found; build it or set APPIMAGE"
DEB="${DEB:-$(ls -t dist/rust-harness-*.deb 2>/dev/null | head -n 1 || true)}"
[[ -n "$DEB" && -f "$DEB" ]] || fail "no .deb found; build it or set DEB"

step "Extracting $APPIMAGE"
mkdir -p "$WORK/appimage"
(
  cd "$WORK/appimage"
  APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE" --appimage-extract >/dev/null
)
APP_ROOT="$WORK/appimage/squashfs-root"
SIDECAR="$APP_ROOT/usr/lib/rust-harness/opencode"
BIN="$APP_ROOT/usr/bin/rust-harness"
[[ -x "$BIN" ]] || fail "the AppImage has no app binary"
[[ -x "$SIDECAR" ]] || fail "the AppImage has no bundled sidecar"
[[ -s "$APP_ROOT/usr/lib/rust-harness/opencode.sha256" ]] \
  || fail "the AppImage has no sidecar checksum manifest"
(
  cd "$APP_ROOT/usr/lib/rust-harness"
  sha256sum -c opencode.sha256
)

step "Extracting $DEB"
mkdir -p "$WORK/deb"
dpkg-deb -x "$DEB" "$WORK/deb"
DEB_BIN="$WORK/deb/usr/bin/rust-harness"
DEB_SIDECAR="$WORK/deb/usr/lib/rust-harness/opencode"
[[ -x "$DEB_BIN" ]] || fail "the deb has no app binary"
[[ -x "$DEB_SIDECAR" ]] || fail "the deb has no bundled sidecar"
(
  cd "$WORK/deb/usr/lib/rust-harness"
  sha256sum -c opencode.sha256
)

smoke() {
  local label="$1" runner="$2" sidecar="$3"
  local project log attempt success=0
  project="$(mktemp -d)"
  log="$WORK/$label.log"

  for attempt in 1 2; do
    if RUST_HARNESS_AUTOQUIT_MS=3000 LIBGL_ALWAYS_SOFTWARE=1 \
      timeout 90 xvfb-run -a "$runner" "$project" >"$log" 2>&1; then
      success=1
      break
    fi
    printf '    %s attempt %s failed\n' "$label" "$attempt"
    sleep 1
  done

  if [[ "$success" != "1" ]]; then
    printf '\n--- %s log ---\n' "$label" >&2
    cat "$log" >&2
    rm -rf "$project"
    fail "$label did not exit cleanly"
  fi

  # Give shutdown a moment, then require the sidecar process to be gone.
  local waited=0
  while ps -eo args= | grep -F -- "$sidecar" | grep -v grep >/dev/null 2>&1; do
    if (( waited >= 50 )); then
      printf '\n--- %s log ---\n' "$label" >&2
      cat "$log" >&2
      rm -rf "$project"
      fail "$label left the sidecar running: $sidecar"
    fi
    sleep 0.1
    waited=$((waited + 1))
  done

  rm -rf "$project"
  printf '    %s exited cleanly; no sidecar left\n' "$label"
}

step "Smoke-testing the AppImage"
smoke appimage "$APP_ROOT/AppRun" "$SIDECAR"

step "Smoke-testing the deb"
smoke deb "$DEB_BIN" "$DEB_SIDECAR"

step "Artifact sizes"
du -h "$APPIMAGE" "$DEB"
step "Packaging e2e passed"
