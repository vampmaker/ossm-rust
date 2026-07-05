#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

RELEASE_DIR="$SCRIPT_DIR/release"
rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"

echo "==> Building frontend..."
(cd frontend && npm ci && npm run build)

echo "==> Building flasher..."
(cd flasher && npm ci && npm run build)
cp flasher/dist/index.html "$RELEASE_DIR/flasher.html"

echo "==> Building firmware for ESP32-C6 ..."
cargo b-c6

echo "==> Building firmware for ESP32-S3 ..."
cargo b-s3

echo "==> Generating merged flash image for ESP32-C6..."
espflash save-image \
    --chip esp32c6 \
    --merge \
    --bootloader target/riscv32imac-esp-espidf/debug/bootloader.bin \
    target/riscv32imac-esp-espidf/debug/ossm-rust \
    "$RELEASE_DIR/ossm-esp32c6.bin"

echo "==> Generating merged flash image for ESP32-S3..."
espflash save-image \
    --chip esp32s3 \
    --merge \
    --bootloader target/xtensa-esp32s3-espidf/debug/bootloader.bin \
    target/xtensa-esp32s3-espidf/debug/ossm-rust \
    "$RELEASE_DIR/ossm-esp32s3.bin"

echo ""
echo "Release artifacts in $RELEASE_DIR/:"
ls -lh "$RELEASE_DIR/"
