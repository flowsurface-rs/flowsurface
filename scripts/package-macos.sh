#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

BINARY=target/packaging/flowsurface
if ! lipo "$BINARY" -verify_arch arm64 x86_64; then
    echo "macOS packaging requires a universal arm64 + x86_64 executable: $BINARY" >&2
    exit 1
fi
VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["version"] for p in m["packages"] if p["name"] == "flowsurface"))')
APP_DIR="target/packaging/macos"
APP="$APP_DIR/Flowsurface.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/flowsurface"
chmod +x "$APP/Contents/MacOS/flowsurface"
cp assets/app-icon/flowsurface.icns "$APP/Contents/Resources/flowsurface.icns"
sed "s/@VERSION@/$VERSION/g" scripts/packaging/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"

codesign --force --sign - "$APP"
codesign --verify --strict "$APP"
touch "$APP"

ln -sfn /Applications "$APP_DIR/Applications"
hdiutil create -volname Flowsurface -fs HFS+ -srcfolder "$APP_DIR" -ov -format UDZO target/packaging/flowsurface-universal-macos.dmg
