#!/bin/bash
TARGET="flowsurface"
VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)
ARCH=${1:-universal} # x86_64 | aarch64 | universal
RELEASE_DIR="target/release"

export MACOSX_DEPLOYMENT_TARGET="11.0"

rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

mkdir -p "$RELEASE_DIR"

if [ "$ARCH" = "x86_64" ]; then
  cargo build --release --target=x86_64-apple-darwin
  cp "target/x86_64-apple-darwin/release/$TARGET" "$RELEASE_DIR/$TARGET"
  tar -czf "$RELEASE_DIR/${TARGET}-x86_64-macos.tar.gz" -C "$RELEASE_DIR" "$TARGET"
  echo "Created $RELEASE_DIR/${TARGET}-x86_64-macos.tar.gz"
  exit 0
fi

if [ "$ARCH" = "aarch64" ]; then
  cargo build --release --target=aarch64-apple-darwin
  cp "target/aarch64-apple-darwin/release/$TARGET" "$RELEASE_DIR/$TARGET"
  tar -czf "$RELEASE_DIR/${TARGET}-aarch64-macos.tar.gz" -C "$RELEASE_DIR" "$TARGET"
  echo "Created $RELEASE_DIR/${TARGET}-aarch64-macos.tar.gz"
  exit 0
fi

# default: build both and create universal
cargo build --release --target=x86_64-apple-darwin
cargo build --release --target=aarch64-apple-darwin

lipo "target/x86_64-apple-darwin/release/$TARGET" "target/aarch64-apple-darwin/release/$TARGET" -create -output "$RELEASE_DIR/$TARGET"
tar -czf "$RELEASE_DIR/${TARGET}-universal-macos.tar.gz" -C "$RELEASE_DIR" "$TARGET"
echo "Created $RELEASE_DIR/${TARGET}-universal-macos.tar.gz"