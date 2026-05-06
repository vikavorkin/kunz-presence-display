#!/usr/bin/env bash
# ota_upload.sh — build release firmware and upload it over the air.
#
# Usage:
#   scripts/ota_upload.sh <device-ip>
#   scripts/ota_upload.sh <device-ip> path/to/firmware.elf   # skip build, use existing ELF
#
# Prerequisites:
#   cargo install espflash          # or: pip install esptool
#   . ~/export-esp.sh               # activate the esp Rust toolchain
#
# The device must already be running this firmware (first flash via USB).
# The partition table must have two OTA slots (CONFIG_PARTITION_TABLE_TWO_OTA=y).

set -euo pipefail

DEVICE_IP="${1:?Usage: $0 <device-ip> [firmware.elf]}"
ELF="${2:-target/xtensa-esp32-espidf/release/kunz-presence-display}"
TMPBIN="${TMPDIR:-/tmp}/presence-ota-$$.bin"

cleanup() { rm -f "$TMPBIN"; }
trap cleanup EXIT

# ── Step 1: build ────────────────────────────────────────────────
if [[ $# -lt 2 ]]; then
    echo "[1/3] Building release firmware..."
    # Source the Espressif toolchain if the export script exists
    if [[ -f "$HOME/export-esp.sh" ]]; then
        # shellcheck disable=SC1091
        . "$HOME/export-esp.sh"
    fi
    cargo build --release
else
    echo "[1/3] Skipping build — using provided ELF: $ELF"
fi

if [[ ! -f "$ELF" ]]; then
    echo "ERROR: ELF not found: $ELF" >&2
    exit 1
fi

# ── Step 2: convert ELF → ESP32 app binary ───────────────────────
echo "[2/3] Converting ELF to ESP32 app binary..."

if command -v python3 &>/dev/null && python3 -m esptool version &>/dev/null 2>&1; then
    python3 -m esptool \
        --chip esp32 \
        elf2image \
        --flash-mode dio \
        --flash-freq 40m \
        --output "$TMPBIN" \
        "$ELF"
elif command -v esptool.py &>/dev/null; then
    esptool.py \
        --chip esp32 \
        elf2image \
        --flash-mode dio \
        --flash-freq 40m \
        --output "$TMPBIN" \
        "$ELF"
else
    echo "ERROR: esptool not found. Install with: pip install esptool" >&2
    exit 1
fi

SIZE=$(wc -c < "$TMPBIN")
echo "    Firmware size: $((SIZE / 1024)) KiB"

# ── Step 3: upload ───────────────────────────────────────────────
echo "[3/3] Uploading to http://${DEVICE_IP}/ota ..."
HTTP_CODE=$(curl \
    --silent \
    --show-error \
    --output /dev/stderr \
    --write-out "%{http_code}" \
    --request POST \
    --data-binary "@${TMPBIN}" \
    --connect-timeout 10 \
    --max-time 120 \
    "http://${DEVICE_IP}/ota" 2>&1 | tail -1)

echo
if [[ "$HTTP_CODE" == "200" ]]; then
    echo "Done — firmware accepted, device is rebooting."
    echo "Wait ~5 s, then verify at: http://${DEVICE_IP}/"
else
    echo "ERROR: device returned HTTP $HTTP_CODE" >&2
    exit 1
fi
