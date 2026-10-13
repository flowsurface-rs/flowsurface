#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
EXE_NAME="flowsurface.exe"
ARCH=${1:-x86_64} # x86_64 | aarch64
case "$ARCH" in
  x86_64|aarch64) ;;
  *) echo "Unsupported Windows architecture: $ARCH" >&2; exit 1 ;;
esac

TARGET_TRIPLE="$ARCH-pc-windows-msvc"

# build binary
rustup target add $TARGET_TRIPLE
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static" cargo build --profile packaging --locked --target="$TARGET_TRIPLE"

# create staging directory
mkdir -p target/packaging/win-unsigned

# Fonts and sounds are embedded in the executable.
cp "target/$TARGET_TRIPLE/packaging/$EXE_NAME" target/packaging/win-unsigned/
