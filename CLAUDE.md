# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32-based presence display ("CYD" - Cheap Yellow Display) that shows open/closed status on a 3.2" ILI9341 touchscreen and reports state changes via Telegram bot. State persists across reboots via NVS flash storage.

Written in **Rust** using the ESP-IDF ecosystem (`esp-idf-hal` + `esp-idf-svc`).

## Prerequisites

Install the Espressif Rust toolchain and tools:

```bash
cargo install espup espflash ldproxy
espup install          # installs esp Rust toolchain + Xtensa LLVM
. ~/export-esp.sh      # activate the toolchain (add to ~/.bashrc for permanence)
pip install esptool    # needed for OTA binary conversion
```

## Build & Flash

```bash
cargo build                   # debug build for ESP32
cargo build --release         # release build (optimised)
cargo run                     # build + flash + serial monitor (USB)
espflash monitor              # serial monitor only (115200 baud)
```

The first build downloads ESP-IDF v5.3 automatically via `embuild`. Subsequent builds are incremental.

## Tests

The project has a `[lib]` crate (`src/lib.rs` → `src/logic.rs`) containing all pure-Rust logic with no ESP-IDF dependencies. Run the full test suite on the host:

```bash
cargo test --target x86_64-unknown-linux-gnu --lib
```

Tests cover: elapsed-time formatting, clock/date formatting, Gregorian calendar (`unix_to_date`), URL percent-decoding, form-body parsing, JSON escaping, and Telegram JSON building. The embedded binary target (`--bin`) cannot run tests; only `--lib` is tested on the host.

## OTA Updates

After the first USB flash, subsequent firmware updates can be delivered over WiFi.

```bash
scripts/ota_upload.sh <device-ip>
# e.g. scripts/ota_upload.sh 192.168.1.42
```

The script:
1. Builds the release ELF with `cargo build --release`
2. Converts it to the ESP32 app binary format with `esptool elf2image`
3. POSTs the binary to `http://<device-ip>/ota`

The device writes the firmware to the inactive OTA slot, marks it as the boot target, and restarts. The partition layout must have two OTA slots — this is already configured in `sdkconfig.defaults` (`CONFIG_PARTITION_TABLE_TWO_OTA=y`).

You can also skip the build step and supply your own ELF:
```bash
scripts/ota_upload.sh 192.168.1.42 path/to/custom.elf
```

## Architecture

```
src/
├── lib.rs       pure-Rust library crate (no ESP-IDF deps; testable on host)
├── logic.rs     formatting, URL decode, JSON helpers — tested here
├── main.rs      hardware init, WiFi/AP, NTP, main loop, button, backlight
├── config.rs    NVS config load/save (AppConfig), AP password generation
├── display.rs   ILI9341 rendering via mipidsi + embedded-graphics
├── telegram.rs  HTTPS POST to Telegram Bot API
└── web.rs       HTTP server: GET /, POST /save, POST /ota
```

### Hardware Layer
- **Display:** ILI9341 LCD 320×240 landscape — SPI2 (HSPI): CLK=14, MOSI=13, MISO=12, CS=15, DC=2
- **Touch:** XPT2046 IRQ on GPIO36 (active-LOW) — backlight-wake only
- **Input:** Physical latching button on GPIO22 (debounced 50 ms); runtime-configurable via web UI
- **LED:** Active-LOW RGB on GPIO 4 (red), 16 (green), 17 (blue)
- **Backlight:** PWM via ESP32 LEDC timer0/channel0 on GPIO21

### Core State (in `main()`)
- `toggle_state: bool` — current open/closed
- `toggled_at: Option<u64>` — `millis()` of last toggle (reconstructed from NVS+NTP on reboot)
- `last_sent_text: String` — cached Telegram message for dedup

### Configuration & Persistence
Two NVS namespaces:
- `"tgcfg"` — WiFi SSID/password, Telegram token/chat ID, on/off messages, button GPIO, backlight settings, AP password (`apPass`)
- `"tgstate"` — last message text and toggle wall-clock timestamp

Web config UI at `http://<device-ip>/` (port 80) via `esp-idf-svc`'s `EspHttpServer`.

### AP Setup Mode
On first boot (no WiFi SSID in NVS) or WiFi connection failure:
- Starts `PresenceSetup` access point with a random 8-char password (generated once, stored in NVS)
- `draw_ap_splash()` shows SSID, password, config URL on screen
- Blue LED blinks while waiting for user to configure
- After config save, `esp_restart()` is called

### Display Rendering
- `draw_frame()` — full screen redraw, called on boot or state change
- `update_live_zones()` — partial redraw every second (clock, date, elapsed time), string-diff cache avoids flicker
- Fonts: `PROFONT_24_POINT` for badge text, `FONT_10X20` for clock/date, `FONT_6X10` for small labels

### Telegram Integration
- Direct HTTPS POST to `api.telegram.org` using `esp-idf-svc` HTTP client with mbedTLS certificate bundle
- State message stored to NVS after successful send for dedup across reboots
- Only sends when new message differs from `last_sent_text`

### WiFi / NTP
- Runtime-configurable SSID/password via NVS; `DEFAULT_WIFI_SSID`/`DEFAULT_WIFI_PASS` in `config.rs` are first-boot defaults
- NTP via `esp-idf-svc`'s `EspSntp::new_default()`; 5-second sync timeout
- `NTP_UTC_OFFSET_SEC` in `config.rs` sets the timezone offset applied to display times

### OTA Updates
- Endpoint: `POST /ota` — receives the raw ESP32 app binary, writes it to the inactive OTA slot via `esp_idf_svc::ota::EspOta`, then restarts
- Two-slot OTA partition layout is required (`CONFIG_PARTITION_TABLE_TWO_OTA=y`)
- Use `scripts/ota_upload.sh` for the full build→convert→upload workflow

## Key Crates
| Crate | Purpose |
|---|---|
| `esp-idf-hal` 0.44 | GPIO, SPI, LEDC PWM, peripherals |
| `esp-idf-svc` 0.48 | WiFi, NVS, HTTP server/client, SNTP, OTA |
| `mipidsi` 0.9 | ILI9341 display driver |
| `embedded-graphics` 0.8 | 2D graphics primitives and text |
| `profont` 0.2 | Larger bitmap fonts for badge text |

## Important Configuration Points
- WiFi credentials set via web UI, persisted in NVS; compile-time defaults apply only on first boot
- AP password auto-generated on first boot, reused across reboots; erasing NVS regenerates it
- All pin assignments are constants at the top of `main.rs`
- `NTP_UTC_OFFSET_SEC` in `config.rs` sets the timezone (default UTC+2 / Helsinki/Kyiv)
- OTA requires `esptool` (`pip install esptool`) for the ELF→binary conversion step in the upload script
