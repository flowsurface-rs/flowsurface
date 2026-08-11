#!/bin/bash
set -euo pipefail

APP_NAME="Flowsurface"
BIN_NAME="flowsurface"
MIN_MACOS_VERSION="11.0"

DIST_DIR="target/macos-dist"
STAGING_DIR="$DIST_DIR/dmg-staging"
BACKGROUND_PNG="assets/dmg-background.png"
ICON_ICNS="assets/icon.icns"

WINDOW_X=200
WINDOW_Y=120
WINDOW_WIDTH=640
WINDOW_HEIGHT=420
ICON_SIZE=128
TEXT_SIZE=13
APP_ICON_X=170
APP_ICON_Y=205
APPLICATIONS_ICON_X=470
APPLICATIONS_ICON_Y=205

usage() {
  cat <<EOF
Usage: bash scripts/package-dmg.sh [aarch64|x86_64]

Builds Flowsurface.app with cargo-bundle and packages it as a drag-to-Applications DMG.

Required tools:
  cargo install cargo-bundle
  brew install create-dmg

Optional release environment:
  SIGNING_IDENTITY="Developer ID Application: ..."
  DMG_SIGNING_IDENTITY="Developer ID Installer: ..."
  NOTARY_PROFILE="notarytool-keychain-profile"
EOF
}

die() {
  echo "Error: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Missing command: $1"
}

package_version() {
  grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2
}

target_for_arch() {
  case "$1" in
    aarch64) echo "aarch64-apple-darwin" ;;
    x86_64) echo "x86_64-apple-darwin" ;;
    *) return 1 ;;
  esac
}

prepare_tools() {
  require_cmd cargo-bundle
  require_cmd create-dmg
  require_cmd codesign
  require_cmd hdiutil
  require_cmd rustup
}

prepare_assets() {
  [ -f "$ICON_ICNS" ] || die "Missing icon: $ICON_ICNS"
  [ -f "$BACKGROUND_PNG" ] || die "Missing DMG background: $BACKGROUND_PNG"
}

sign_app() {
  local app_bundle="$1"

  if [ -n "${SIGNING_IDENTITY:-}" ]; then
    echo "Signing app with Developer ID Application..."
    codesign \
      --force \
      --deep \
      --options runtime \
      --timestamp \
      --sign "$SIGNING_IDENTITY" \
      "$app_bundle"
  else
    echo "Ad-hoc signing app for local testing..."
    codesign --force --deep --sign - "$app_bundle"
  fi

  codesign --verify --deep --strict --verbose=2 "$app_bundle"
}

sign_dmg() {
  local dmg_path="$1"

  [ -n "${DMG_SIGNING_IDENTITY:-}" ] || return 0

  echo "Signing DMG with Developer ID Installer..."
  codesign --force --timestamp --sign "$DMG_SIGNING_IDENTITY" "$dmg_path"
  codesign --verify --verbose=2 "$dmg_path"
}

notarize_dmg() {
  local dmg_path="$1"

  if [ -z "${NOTARY_PROFILE:-}" ]; then
    echo "Skipping notarization. Set NOTARY_PROFILE for release builds."
    return 0
  fi

  echo "Submitting DMG for notarization..."
  xcrun notarytool submit "$dmg_path" --keychain-profile "$NOTARY_PROFILE" --wait

  echo "Stapling notarization ticket..."
  xcrun stapler staple "$dmg_path"
}

create_dmg() {
  local dmg_path="$1"
  local staging_dir="$2"

  local args=(
    --volname "$APP_NAME $VERSION"
    --volicon "$ICON_ICNS"
    --background "$BACKGROUND_PNG"
    --window-pos "$WINDOW_X" "$WINDOW_Y"
    --window-size "$WINDOW_WIDTH" "$WINDOW_HEIGHT"
    --icon-size "$ICON_SIZE"
    --text-size "$TEXT_SIZE"
    --icon "$APP_NAME.app" "$APP_ICON_X" "$APP_ICON_Y"
    --hide-extension "$APP_NAME.app"
    --app-drop-link "$APPLICATIONS_ICON_X" "$APPLICATIONS_ICON_Y"
    "$dmg_path"
    "$staging_dir"
  )

  if ! create-dmg "${args[@]}"; then
    echo "create-dmg Finder styling failed; retrying without Finder styling..."
    rm -f "$dmg_path"
    create-dmg --skip-jenkins "${args[@]}"
  fi
}

ARCH="${1:-aarch64}"
TARGET="$(target_for_arch "$ARCH")" || {
  usage >&2
  die "Unsupported arch: $ARCH"
}

VERSION="$(package_version)"
DMG_PATH="$DIST_DIR/${BIN_NAME}-${VERSION}-${ARCH}-macos.dmg"
APP_BUNDLE="target/$TARGET/release/bundle/osx/$APP_NAME.app"

prepare_tools
prepare_assets

export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS_VERSION"

rustup target add "$TARGET"

echo "Building $APP_NAME.app for $TARGET..."
cargo bundle --release --target "$TARGET" --format osx

[ -d "$APP_BUNDLE" ] || die "Expected app bundle not found: $APP_BUNDLE"
sign_app "$APP_BUNDLE"

mkdir -p "$DIST_DIR"
rm -f "$DMG_PATH"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
cp -R "$APP_BUNDLE" "$STAGING_DIR/"

echo "Creating DMG: $DMG_PATH"
create_dmg "$DMG_PATH" "$STAGING_DIR"
sign_dmg "$DMG_PATH"
notarize_dmg "$DMG_PATH"
hdiutil verify "$DMG_PATH"
echo "Created $DMG_PATH"
