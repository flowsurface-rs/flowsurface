#!/bin/bash
set -euo pipefail

EXE_NAME="flowsurface.exe"
ARCH=${1:-x86_64} # x86_64 | aarch64
VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)

# print environment and installed toolchain info
rustc --version || true
cargo --version || true
rustup --version || true
rustup override set stable-msvc
rustup target list --installed || true
echo "Checking for MSVC toolchain and assembler (cl.exe/nasm)"
where cl || true
nasm -v || echo "nasm not found"

# update package version on Cargo.toml
cargo install --no-default-features --version $(cargo -V | cut -d' ' -f2) cargo-edit || true
cargo set-version $VERSION

# set target triple and zip name
if [ "$ARCH" = "aarch64" ]; then
  TARGET_TRIPLE="aarch64-pc-windows-msvc"
  ZIP_NAME="flowsurface-aarch64-windows.zip"
else
  TARGET_TRIPLE="x86_64-pc-windows-msvc"
  ZIP_NAME="flowsurface-x86_64-windows.zip"
fi

# build binary and fail on error
rustup target add "$TARGET_TRIPLE"
if ! cargo build --release --target="$TARGET_TRIPLE"; then
  echo "Cargo build failed for $TARGET_TRIPLE"
  ls -la "target/$TARGET_TRIPLE/release" || true
  exit 1
fi

# create staging directory
mkdir -p target/release/win-portable

# check the binary exists and show details
BINARY="target/$TARGET_TRIPLE/release/$EXE_NAME"
if [ ! -f "$BINARY" ]; then
  echo "ERROR: binary not found: $BINARY"
  ls -la "target/$TARGET_TRIPLE/release" || true
  exit 1
fi

ls -lh "$BINARY"
sha256sum "$BINARY" || true

# copy executable and assets
cp "$BINARY" target/release/win-portable/
if [ -d "assets" ]; then
    cp -r assets target/release/win-portable/
fi

# create zip archive
cd target/release
powershell -Command "Compress-Archive -Path win-portable\* -DestinationPath $ZIP_NAME -Force"
echo "Created $ZIP_NAME"