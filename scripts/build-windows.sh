#!/bin/bash
EXE_NAME="flowsurface.exe"
ARCH=${1:-x86_64} # x86_64 | aarch64
VERSION=$(grep '^version = ' Cargo.toml | cut -d'"' -f2)

# update package version on Cargo.toml
cargo install cargo-edit
cargo set-version $VERSION

rustup override set stable-msvc

# set target triple and zip name
if [ "$ARCH" = "aarch64" ]; then
  TARGET_TRIPLE="aarch64-pc-windows-msvc"
  ZIP_NAME="flowsurface-aarch64-windows.zip"
else
  TARGET_TRIPLE="x86_64-pc-windows-msvc"
  ZIP_NAME="flowsurface-x86_64-windows.zip"
fi

# build binary
rustup target add $TARGET_TRIPLE
cargo build --release --target=$TARGET_TRIPLE

# create staging directory
mkdir -p target/release/win-portable

# copy executable and assets (fix paths)
cp "target/$TARGET_TRIPLE/release/$EXE_NAME" target/release/win-portable/
if [ -d "assets" ]; then
    cp -r assets target/release/win-portable/
fi

# create zip archive
cd target/release
powershell -Command "Compress-Archive -Path win-portable\* -DestinationPath $ZIP_NAME -Force"
echo "Created $ZIP_NAME"