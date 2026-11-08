#!/usr/bin/env bash
#
# Build "Rust Harness.app" with the OpenCode sidecar inside it.
#
# The bundle is ad-hoc signed by default. Set SIGN_IDENTITY to a "Developer ID
# Application" certificate for a real signature, and NOTARY_PROFILE (or
# APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD) to notarize and staple it.
#
# Common overrides:
#   APP_NAME="Rust Harness"        bundle and display name
#   BUNDLE_ID=dev.rustharness.RustHarness
#   VERSION=0.1.0                  defaults to the workspace version
#   UNIVERSAL=1                    build arm64 + x86_64 and lipo the app
#   OPENCODE_BIN=/path/to/opencode sidecar to bundle (default: `opencode` on PATH)
#   OPENCODE_ARM64_BIN, OPENCODE_X86_64_BIN
#                                  per-arch sidecars for a universal bundle
#   SIGN_IDENTITY="Developer ID Application: …"
#   NOTARY_PROFILE=rust-harness    xcrun notarytool keychain profile
#   ARCHIVE=1                      also write dist/<name>-<version>.zip (+ .sha256)
#
# Output: dist/<APP_NAME>.app

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_NAME="${APP_NAME:-Rust Harness}"
BUNDLE_ID="${BUNDLE_ID:-dev.rustharness.RustHarness}"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
UNIVERSAL="${UNIVERSAL:-0}"
DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"
HOST_ARCH="$(uname -m)"

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

resolve_executable() {
  [[ -x "$1" ]] || fail "not an executable: $1"
  python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

step "Building $APP_NAME $VERSION"
BINARIES=()
if [[ "$UNIVERSAL" == "1" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build -p rust-harness --release --target aarch64-apple-darwin
  cargo build -p rust-harness --release --target x86_64-apple-darwin
  BINARIES=(
    "target/aarch64-apple-darwin/release/rust-harness"
    "target/x86_64-apple-darwin/release/rust-harness"
  )
else
  cargo build -p rust-harness --release
  BINARIES=("target/release/rust-harness")
fi

step "Resolving the OpenCode sidecar"
if [[ -n "${OPENCODE_BIN:-}" ]]; then
  SIDECAR="$(resolve_executable "$OPENCODE_BIN")"
elif command -v opencode >/dev/null 2>&1; then
  SIDECAR="$(resolve_executable "$(command -v opencode)")"
else
  fail "opencode not found; install it or set OPENCODE_BIN"
fi
tracing_note="OpenCode sidecar: $SIDECAR"

step "Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [[ "$UNIVERSAL" == "1" ]]; then
  lipo -create -output "$APP/Contents/MacOS/rust-harness" "${BINARIES[@]}"
else
  cp "${BINARIES[0]}" "$APP/Contents/MacOS/rust-harness"
fi
chmod 755 "$APP/Contents/MacOS/rust-harness"

if [[ "$UNIVERSAL" == "1" ]] && [[ -n "${OPENCODE_ARM64_BIN:-}" || -n "${OPENCODE_X86_64_BIN:-}" ]]; then
  [[ -n "${OPENCODE_ARM64_BIN:-}" && -n "${OPENCODE_X86_64_BIN:-}" ]] \
    || fail "set both OPENCODE_ARM64_BIN and OPENCODE_X86_64_BIN, or a universal OPENCODE_BIN"
  lipo -create \
    -output "$APP/Contents/Resources/opencode" \
    "$(resolve_executable "$OPENCODE_X86_64_BIN")" \
    "$(resolve_executable "$OPENCODE_ARM64_BIN")"
else
  cp "$SIDECAR" "$APP/Contents/Resources/opencode"
fi
chmod 755 "$APP/Contents/Resources/opencode"

SIDECAR_ARCHS="$(lipo -archs "$APP/Contents/Resources/opencode")"
if [[ "$UNIVERSAL" == "1" ]]; then
  [[ " $SIDECAR_ARCHS " == *" arm64 "* && " $SIDECAR_ARCHS " == *" x86_64 "* ]] \
    || fail "the sidecar is not universal ($SIDECAR_ARCHS); pass per-arch binaries or set UNIVERSAL=0"
else
  [[ " $SIDECAR_ARCHS " == *" $HOST_ARCH "* ]] \
    || fail "the sidecar ($SIDECAR_ARCHS) does not match the app ($HOST_ARCH); pass OPENCODE_BIN for this machine"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>rust-harness</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>MIT licensed</string>
</dict>
</plist>
PLIST
printf 'APPL????' > "$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

if [[ -f packaging/AppIcon.icns ]]; then
  cp packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string AppIcon" "$APP/Contents/Info.plist"
fi

step "Signing"
if [[ -n "${SIGN_IDENTITY:-}" ]]; then
  sign() { codesign --force --timestamp --options runtime --sign "$SIGN_IDENTITY" "$1"; }
else
  sign() { codesign --force --sign - "$1"; }
fi
# Nested code first: signing the app last seals the sidecar's signature.
sign "$APP/Contents/Resources/opencode"

step "Recording the sidecar checksum"
# After signing: codesign rewrites the Mach-O, so the manifest must hash the
# signed bytes. This runs before the app itself is signed so the seal covers it.
(
  cd "$APP/Contents/Resources"
  shasum -a 256 opencode > opencode.sha256
)

sign "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

NOTARIZE=0
if [[ -n "${NOTARY_PROFILE:-}" || ( -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ) ]]; then
  NOTARIZE=1
fi
ZIP="$DIST/${APP_NAME// /}-$VERSION.zip"

if [[ "$NOTARIZE" == "1" ]]; then
  [[ -n "${SIGN_IDENTITY:-}" ]] || fail "notarization requires SIGN_IDENTITY (Developer ID Application)"
  step "Notarizing"
  ditto -c -k --keepParent "$APP" "$ZIP"
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
  else
    xcrun notarytool submit "$ZIP" \
      --apple-id "$APPLE_ID" \
      --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" \
      --wait
  fi
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute --verbose=2 "$APP"
fi

if [[ "$NOTARIZE" == "1" || "${ARCHIVE:-0}" == "1" ]]; then
  step "Archiving"
  # Re-zip after stapling so the archive carries the notarization ticket.
  ditto -c -k --keepParent "$APP" "$ZIP"
  shasum -a 256 "$ZIP" > "$ZIP.sha256"
  printf '    zip: %s\n' "$ZIP"
fi

step "Done"
printf '    %s\n' "$APP"
printf '    %s\n' "$tracing_note"
du -sh "$APP"
