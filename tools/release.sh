#!/usr/bin/env bash
# Build one Linux release tarball. Usage:
#   tools/release.sh [x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu]
# Run on a Linux host with rustup (+ `rustup target add <target>` and
# `gcc-aarch64-linux-gnu` for the aarch64 cross-build); mirrors the workflow.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:-x86_64-unknown-linux-gnu}"
ARCH="${TARGET%%-*}"
VER=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
NAME="omatach-${VER}-${ARCH}-linux"
cargo build --release --locked --target "$TARGET"
rm -rf "dist/${NAME}" && mkdir -p "dist/${NAME}"
cp "target/${TARGET}/release/omatach" "dist/${NAME}/"
cp packaging/omatach.desktop README.md "dist/${NAME}/"
cp LICENSE-MIT LICENSE-APACHE "dist/${NAME}/"
tar -C dist -czf "dist/${NAME}.tar.gz" "${NAME}"
( cd dist && sha256sum "${NAME}.tar.gz" | tee "${NAME}.tar.gz.sha256" )
echo "→ dist/${NAME}.tar.gz"
