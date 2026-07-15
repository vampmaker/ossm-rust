#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"

RELEASE_DIR="$SCRIPT_DIR/release"
mkdir -p "$RELEASE_DIR"
rm -rf "$RELEASE_DIR"/*

echo "==> Building frontend..."
(cd frontend && npm ci && npm run build)

echo "==> Building flasher..."
(cd flasher && npm ci && npm run build)
cp flasher/dist/index.html "$RELEASE_DIR/flasher.html"

echo "==> Building motor-control..."
(cd motor-control && npm ci && npm run build)
cp motor-control/dist/index.html "$RELEASE_DIR/motor-control.html"

echo "==> Building firmware for ESP32-C6 ..."
cargo b-c6 --release

echo "==> Building firmware for ESP32-S3 ..."
source "${HOME}/export-esp.sh" 2>/dev/null || true
# RUSTUP_TOOLCHAIN=esp cargo b-s3 --release
cargo b-s3 --release

echo "==> Generating merged flash image for ESP32-C6..."
espflash save-image \
    --chip esp32c6 \
    --merge \
    target/riscv32imac-unknown-none-elf/release/ossm-rust \
    "$RELEASE_DIR/ossm-esp32c6.bin"

echo "==> Generating merged flash image for ESP32-S3..."
espflash save-image \
    --chip esp32s3 \
    --merge \
    target/xtensa-esp32s3-none-elf/release/ossm-rust \
    "$RELEASE_DIR/ossm-esp32s3.bin"

echo ""
echo "Release artifacts in $RELEASE_DIR/:"
ls -lh "$RELEASE_DIR/"
