#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="flowsurface"
ARCH=${1:-universal} # x86_64 | aarch64 | universal
case "$ARCH" in
  x86_64|aarch64|universal) ;;
  *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
esac
RELEASE_DIR="target/packaging"

export MACOSX_DEPLOYMENT_TARGET="11.0"

rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

mkdir -p "$RELEASE_DIR"

if [ "$ARCH" = "x86_64" ]; then
  cargo build --profile packaging --locked --target=x86_64-apple-darwin
  cp "target/x86_64-apple-darwin/packaging/$TARGET" "$RELEASE_DIR/$TARGET"
  tar -czf "$RELEASE_DIR/${TARGET}-x86_64-macos.tar.gz" -C "$RELEASE_DIR" "$TARGET"
  echo "Created $RELEASE_DIR/${TARGET}-x86_64-macos.tar.gz"
  exit 0
fi

if [ "$ARCH" = "aarch64" ]; then
  cargo build --profile packaging --locked --target=aarch64-apple-darwin
  cp "target/aarch64-apple-darwin/packaging/$TARGET" "$RELEASE_DIR/$TARGET"
  tar -czf "$RELEASE_DIR/${TARGET}-aarch64-macos.tar.gz" -C "$RELEASE_DIR" "$TARGET"
  echo "Created $RELEASE_DIR/${TARGET}-aarch64-macos.tar.gz"
  exit 0
fi

# default: build both and create universal
cargo build --profile packaging --locked --target=x86_64-apple-darwin
cargo build --profile packaging --locked --target=aarch64-apple-darwin

lipo "target/x86_64-apple-darwin/packaging/$TARGET" "target/aarch64-apple-darwin/packaging/$TARGET" -create -output "$RELEASE_DIR/$TARGET"
