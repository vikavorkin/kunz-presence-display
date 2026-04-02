# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32-based presence display ("CYD" - Cheap Yellow Display) that shows open/closed status on a 3.2" ILI9341 touchscreen and reports state changes via Telegram bot. State persists across reboots via NVS flash storage.

## Build System

This project uses **PlatformIO** (not CMake or Make directly).

```bash
pio run -e cyd                                                      # Build for ESP32 CYD
pio run -e cyd --target upload                                      # Build and flash via USB
pio run -e cyd --target upload --upload-port <device-ip>           # OTA upload (after first USB flash)
pio monitor                                                         # Serial monitor (115200 baud)
pio run -e esp_wroom_02                                             # Build for ESP8266 (secondary target)
```


## Architecture

The entire application is a single Arduino sketch: `src/cyd_telegram_toggle.ino` (~960 lines).

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
- `"tgcfg"` — WiFi SSID/password, Telegram token/chat ID, on/off messages, button GPIO, backlight settings, AP password (`apPass`)
- `"tgstate"` — last message text and timestamp (enables state restoration on reboot)

Web config UI served at `http://<device-ip>/` (port 80) via `WebServer`. `loadConfig()` / `saveConfig()` handle NVS I/O. Compile-time `#define` / `const char*` defaults apply when NVS is empty.

The on/off message fields (`cfgMsgOn` / `cfgMsgOff`) support arbitrary UTF-8 text including emoji — users type them directly into the web UI message fields.

### AP Setup Mode
On first boot (no WiFi SSID in NVS) or when WiFi fails to connect, the device starts a `PresenceSetup` access point instead of going offline:
- A random 8-char password (`cfgApPass`) is generated once via `esp_random()`, stored under `"apPass"` in NVS `"tgcfg"`, and reused across reboots
- Characters are drawn from an unambiguous set (no `0/O/I/1`)
- `drawApSplash()` shows the SSID, password, and config URL (`http://192.168.4.1/`) on screen
- `startAPMode()` — sets `WIFI_AP` mode, registers the same web config routes, blinks the blue LED, and loops forever until the user saves config and the device restarts
- After saving credentials via the web UI, `ESP.restart()` is called and the device connects to the configured network normally

### Display Rendering
- `drawFrame()` — full redraw, called only on boot or state change
- `updateLiveZones()` — runs every 1 second, caches previous strings and only redraws changed zones (clock, date, elapsed time)
- Screen layout defined by `Y_*` constants (lines ~102–143)

### Telegram Integration
- `stateToMsg(bool)` — returns the configured on/off message for a given state
- `sendTelegram()` — sends state message; persists text to NVS for dedup
- `fetchLastBotMessage()` — called on boot to restore `toggleState` from NVS by comparing the saved message against `stateToMsg(true)`
- Only sends if new message text differs from `lastSentText`

### WiFi / NTP
- SSID and password are runtime-configurable via the web UI (`cfgWifiSsid` / `cfgWifiPass`), stored in NVS `"tgcfg"`; compile-time `WIFI_SSID` / `WIFI_PASSWORD` consts serve as first-boot defaults
- If no SSID is configured or connection fails after 20 s, the device enters AP setup mode (see above) rather than going offline
- NTP via `configTime()` with `UTC_OFFSET_SEC` (default UTC+2); `ntpSynced` flag guards time-dependent logic

### OTA Updates
- `setupOTA()` — initialises Arduino OTA with hostname `presence-display`; called after WiFi connects
- `ArduinoOTA.handle()` called at the top of every `loop()` iteration
- Shows "OTA update..." splash during flashing
- `ArduinoOTA` is part of the ESP32 Arduino core — no extra `lib_deps` entry needed

## Key Libraries (managed by PlatformIO)
| Library | Purpose |
|---|---|
| TFT_eSPI ^2.5.43 | ILI9341 driver; pin mapping via build flags in `platformio.ini` |
| XPT2046_Touchscreen | Touch input |
| UniversalTelegramBot ^1.3.0 | Telegram Bot API |
| ArduinoJson ^6.21.5 | JSON parsing for Telegram responses |

## Important Configuration Points
- WiFi credentials are set via the web UI and persisted in NVS; compile-time `WIFI_SSID` / `WIFI_PASSWORD` consts in the source are no longer the first-boot fallback — if NVS SSID is empty the device enters AP mode
- The AP password is auto-generated on first boot and printed on screen; it does not change unless NVS is erased
- Touch calibration constants (`TOUCH_MIN_X`, `TOUCH_MAX_X`, etc.) may need adjustment per unit
- TFT_eSPI pin assignments are set via `build_flags` in `platformio.ini`, not via `User_Setup.h`
