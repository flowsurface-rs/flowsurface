#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
ARCH=${2:-x86_64}
case "$ARCH" in
    x86_64|aarch64) TRIPLE="$ARCH-unknown-linux-gnu" ;;
    *) echo "Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
esac
RELEASE_DIR=target/packaging
ARCHIVE_DIR="$RELEASE_DIR/linux-$ARCH"
ARCHIVE_PATH="$RELEASE_DIR/flowsurface-$ARCH-linux.tar.gz"
case "${1:-}" in
    archive_path) echo "$ARCHIVE_PATH" ;;
    package)
        rustup target add "$TRIPLE"
        cargo build --locked --profile packaging --target "$TRIPLE"
        rm -rf -- "$ARCHIVE_DIR"
        install -Dm755 "target/$TRIPLE/packaging/flowsurface" "$ARCHIVE_DIR/bin/flowsurface"
        install -Dm755 scripts/packaging/linux/install.sh "$ARCHIVE_DIR/install.sh"
        install -Dm755 scripts/packaging/linux/uninstall.sh "$ARCHIVE_DIR/uninstall.sh"
        install -Dm644 scripts/packaging/linux/com.flowsurface.flowsurface.desktop "$ARCHIVE_DIR/share/applications/com.flowsurface.flowsurface.desktop"
        install -Dm644 assets/app-icon/linux/flowsurface.png "$ARCHIVE_DIR/share/icons/hicolor/512x512/apps/com.flowsurface.flowsurface.png"
        tar czf "$ARCHIVE_PATH" -C "$ARCHIVE_DIR" .
        echo "Packaged archive: $ARCHIVE_PATH"
        ;;
    *) echo "Usage: $0 {package|archive_path} [x86_64|aarch64]" >&2; exit 1 ;;
esac
