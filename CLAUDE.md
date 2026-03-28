# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32-based presence display ("CYD" - Cheap Yellow Display) that shows open/closed status on a 3.2" ILI9341 touchscreen and reports state changes via Telegram bot. State persists across reboots via NVS flash storage.

## Build System

This project uses **PlatformIO** (not CMake or Make directly).

```bash
pio run -e cyd                          # Build for ESP32 CYD
pio run -e cyd --target upload          # Build and flash to device
pio monitor                             # Serial monitor (115200 baud)
pio run -e esp_wroom_02                 # Build for ESP8266 (secondary target)
```

Simulator: Wokwi (configured via `wokwi.toml` + `diagram.json`). Uses `.pio/build/cyd/firmware.bin`.

## Architecture

The entire application is a single Arduino sketch: `src/cyd_telegram_toggle.ino` (~803 lines).

### Hardware Layer
- **Display:** ILI9341 LCD 320×240, landscape — driven via TFT_eSPI with custom pin build flags
- **Touch:** XPT2046 touchscreen controller (separate SPI CS)
- **Input:** Physical latching button on GPIO22 (debounced 50ms)
- **LED:** Active-LOW RGB on GPIO 4 (red), 16 (green), 17 (blue)
- **Backlight:** PWM via ESP32 LEDC on GPIO21

### Core State
Three pieces of runtime state drive everything:
- `toggleState` — current open/closed bool
- `toggledAt` — `millis()` of last toggle (reconstructed from NVS on reboot using NTP)
- `lastSentText` — cached Telegram message for dedup

### Configuration & Persistence
Two NVS namespaces:
- `"tgcfg"` — Telegram token/chat ID, button GPIO, backlight settings
- `"tgstate"` — last message text and timestamp (enables state restoration on reboot)

Web config UI served at `http://<device-ip>/` (port 80) via `WebServer`. `loadConfig()` / `saveConfig()` in lines 149–177 handle NVS I/O. Compile-time `#define` defaults apply when NVS is empty.

### Display Rendering
- `drawFrame()` — full redraw, called only on boot or state change
- `updateLiveZones()` — runs every 1 second, caches previous strings and only redraws changed zones (clock, date, elapsed time)
- Screen layout defined by `Y_*` constants (lines ~102–143)

### Telegram Integration
- `sendTelegram()` — sends state message and auto-pins it (unpins previous)
- `fetchLastBotMessage()` — called on boot to restore `toggleState` from NVS
- Only sends if new message text differs from `lastSentText`

### WiFi / NTP
- Connects to hardcoded SSID (line ~30); falls back gracefully after 20s timeout
- NTP via `configTime()` with `UTC_OFFSET_SEC` (default UTC+2); `ntpSynced` flag guards time-dependent logic

## Key Libraries (managed by PlatformIO)
| Library | Purpose |
|---|---|
| TFT_eSPI ^2.5.43 | ILI9341 driver; pin mapping via build flags in `platformio.ini` |
| XPT2046_Touchscreen | Touch input |
| UniversalTelegramBot ^1.3.0 | Telegram Bot API |
| ArduinoJson ^6.21.5 | JSON parsing for Telegram responses |

## Important Configuration Points
- WiFi credentials are hardcoded in the source (`WIFI_SSID` / `WIFI_PASS` defines)
- Touch calibration constants (`TOUCH_MIN_X`, `TOUCH_MAX_X`, etc.) may need adjustment per unit
- TFT_eSPI pin assignments are set via `build_flags` in `platformio.ini`, not via `User_Setup.h`
