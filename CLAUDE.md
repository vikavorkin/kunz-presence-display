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
```

## Build System

This project uses **Cargo** with the `xtensa-esp32-espidf` target.

```bash
cargo build                              # Debug build for ESP32 CYD
cargo build --release                    # Release build (smaller/faster)
cargo run                                # Build and flash via USB (uses espflash runner)
espflash monitor                         # Serial monitor (115200 baud)
```

The first build downloads ESP-IDF v5.3 automatically via `embuild`. Subsequent builds are incremental.

## Architecture

The application is split into four Rust modules under `src/`:

| Module | Responsibility |
|---|---|
| `main.rs` | Hardware init, WiFi, NTP, SNTP, main loop, button debounce, backlight |
| `config.rs` | NVS config loading/saving (`AppConfig`), AP password generation |
| `display.rs` | ILI9341 rendering via `mipidsi` + `embedded-graphics` |
| `telegram.rs` | HTTPS POST to Telegram Bot API |
| `web.rs` | HTTP config server (GET `/`, POST `/save`) |

### Hardware Layer
- **Display:** ILI9341 LCD 320×240 landscape — driven via `mipidsi` + `embedded-graphics`, SPI2 (HSPI): CLK=14, MOSI=13, MISO=12, CS=15, DC=2
- **Touch:** XPT2046 IRQ on GPIO36 (active-LOW) — used only for backlight-wake detection
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
`load_config()` / `save_config()` in `config.rs` handle NVS I/O.

### AP Setup Mode
On first boot (no WiFi SSID in NVS) or WiFi connection failure:
- Starts `PresenceSetup` access point with a random 8-char password (generated once, stored in NVS)
- `draw_ap_splash()` shows SSID, password, config URL on screen
- Blue LED blinks while waiting for user to configure
- After config save, `esp_restart()` is called

### Display Rendering
- `draw_frame()` — full screen redraw, called on boot or state change
- `update_live_zones()` — partial redraw every second (clock, date, elapsed time), with string-diff cache to avoid flicker
- Fonts: `PROFONT_24_POINT` for badge text, `FONT_10X20` for clock/date, `FONT_6X10` for small labels

### Telegram Integration
- Direct HTTPS POST to `api.telegram.org` using `esp-idf-svc` HTTP client with mbedTLS certificate bundle
- State message stored to NVS after successful send for dedup across reboots
- Only sends when new message differs from `last_sent_text`

### WiFi / NTP
- Runtime-configurable SSID/password via NVS; `DEFAULT_WIFI_SSID`/`DEFAULT_WIFI_PASS` constants in `config.rs` are first-boot defaults
- NTP via `esp-idf-svc`'s `EspSntp`; 5-second sync timeout; `NTP_UTC_OFFSET_SEC` defaults to UTC+2

### OTA Updates
OTA via the ESP-IDF native mechanism. After first USB flash, firmware updates can be delivered over HTTP using `espflash` or `curl`:
```bash
# Upload new firmware over the air
espflash upload --chip esp32 --port <device-ip> target/xtensa-esp32-espidf/release/kunz-presence-display
```

## Key Crates
| Crate | Purpose |
|---|---|
| `esp-idf-hal` 0.44 | GPIO, SPI, LEDC PWM, peripherals |
| `esp-idf-svc` 0.48 | WiFi, NVS, HTTP server/client, SNTP |
| `mipidsi` 0.9 | ILI9341 display driver |
| `embedded-graphics` 0.8 | 2D graphics primitives and text |
| `profont` 0.2 | Larger bitmap fonts for badge text |

## Important Configuration Points
- WiFi credentials set via web UI, persisted in NVS; `DEFAULT_WIFI_SSID`/`DEFAULT_WIFI_PASS` apply only on very first boot (empty NVS)
- AP password auto-generated on first boot, reused across reboots; erasing NVS regenerates it
- Touch calibration (`TOUCH_MIN_X` etc.) only affects backlight-wake on touch (IRQ is used, not raw coordinates) — no per-unit calibration needed for that
- All pin assignments are constants at the top of `main.rs`
- `NTP_UTC_OFFSET_SEC` in `config.rs` sets the timezone (default UTC+2)
