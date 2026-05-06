mod config;
mod display;
mod telegram;
mod web;

use anyhow::Result;
use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget};
use esp_idf_hal::{
    delay::FreeRtos,
    gpio::PinDriver,
    ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver, Resolution},
    peripherals::Peripherals,
    spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriver, SpiDriverConfig},
    units::FromValueType,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    sntp::{EspSntp, SyncStatus},
    wifi::{
        AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration,
        EspWifi,
    },
};
use log::{info, warn};
use mipidsi::{
    interface::SpiInterface,
    options::{ColorOrder, Orientation, Rotation},
    Builder,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config::{load_config, load_state, save_state, AppConfig, AP_SSID, NTP_UTC_OFFSET_SEC};
use display::{
    draw_ap_splash, draw_frame, draw_splash, show_notification, update_live_zones, LiveCache,
};

// ── Pin assignments ─────────────────────────────────────────────
// GPIO indices are fixed at compile time; the button pin is runtime-configurable
// via NVS but queried via the raw ESP-IDF GPIO API to avoid a 40-arm match.
const LED_RED_PIN: i32   = 4;
const LED_GREEN_PIN: i32 = 16;
const LED_BLUE_PIN: i32  = 17;
const TOUCH_IRQ_PIN: i32 = 36;

// ── Raw GPIO helpers ────────────────────────────────────────────
fn gpio_init_output(pin: i32, initial_high: bool) {
    unsafe {
        esp_idf_sys::gpio_reset_pin(pin);
        esp_idf_sys::gpio_set_direction(pin, esp_idf_sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        esp_idf_sys::gpio_set_level(pin, u32::from(initial_high));
    }
}

fn gpio_init_input_pullup(pin: i32) {
    unsafe {
        esp_idf_sys::gpio_reset_pin(pin);
        esp_idf_sys::gpio_set_direction(pin, esp_idf_sys::gpio_mode_t_GPIO_MODE_INPUT);
        esp_idf_sys::gpio_pullup_en(pin);
        esp_idf_sys::gpio_pulldown_dis(pin);
    }
}

fn gpio_set(pin: i32, high: bool) {
    unsafe { esp_idf_sys::gpio_set_level(pin, u32::from(high)); }
}

fn gpio_read(pin: i32) -> bool {
    unsafe { esp_idf_sys::gpio_get_level(pin) != 0 }
}

// ── RGB LED (active LOW) ────────────────────────────────────────
fn set_led(on: bool) {
    gpio_set(LED_RED_PIN,   true);   // off
    gpio_set(LED_BLUE_PIN,  true);   // off
    gpio_set(LED_GREEN_PIN, !on);    // LOW = on
}

// ── Touch: IRQ pin is active-LOW when touched ───────────────────
fn is_touched() -> bool {
    !gpio_read(TOUCH_IRQ_PIN)
}

// ── Monotonic milliseconds via esp_timer ────────────────────────
fn millis() -> u64 {
    (unsafe { esp_idf_sys::esp_timer_get_time() } as u64) / 1000
}

// ── Unix time (UTC seconds) from system clock ───────────────────
fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_time_valid() -> bool {
    unix_now_secs() > 1_700_000_000
}

// Apply configured UTC offset so the display shows local time
fn unix_now_local_ms() -> u64 {
    let utc = unix_now_secs();
    if utc > 1_700_000_000 {
        utc.saturating_add_signed(NTP_UTC_OFFSET_SEC as i64) * 1000
    } else {
        0
    }
}

// ── AP mode loop — never returns normally ───────────────────────
fn enter_ap_mode<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    cfg: &AppConfig,
    wifi: &mut BlockingWifi<EspWifi<'_>>,
    nvs_partition: EspDefaultNvsPartition,
) -> ! {
    info!("[AP] Starting AP: SSID={} Pass={}", AP_SSID, cfg.ap_pass);

    let _ = wifi.stop(); // stop any previous state

    let auth = if cfg.ap_pass.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid:        AP_SSID.try_into().unwrap_or_default(),
        password:    cfg.ap_pass.as_str().try_into().unwrap_or_default(),
        auth_method: auth,
        ..Default::default()
    }))
    .expect("AP set_configuration failed");
    wifi.start().expect("AP start failed");

    let ap_ip = "192.168.4.1";
    draw_ap_splash(display, AP_SSID, &cfg.ap_pass, ap_ip);

    let shared_cfg = Arc::new(Mutex::new(cfg.clone()));
    let _server =
        web::start_server(shared_cfg, nvs_partition).expect("Web server start failed");

    info!("[AP] Config at http://{}/  (Blue LED blinks)", ap_ip);

    // Blink blue LED until user saves config (which triggers esp_restart)
    let mut led_on = false;
    let mut last_blink = millis();
    loop {
        let now = millis();
        if now - last_blink >= 800 {
            last_blink = now;
            led_on = !led_on;
            gpio_set(LED_BLUE_PIN, !led_on); // active LOW
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("[main] Kunz Presence Display starting");

    let peripherals = Peripherals::take()?;
    let sysloop     = EspSystemEventLoop::take()?;
    let nvs_partition = EspDefaultNvsPartition::take()?;

    // ── RGB LED: active LOW → output HIGH = off ─────────────────
    gpio_init_output(LED_RED_PIN,   true);
    gpio_init_output(LED_GREEN_PIN, true);
    gpio_init_output(LED_BLUE_PIN,  true);

    // ── Load NVS config ─────────────────────────────────────────
    let cfg = load_config(nvs_partition.clone())?;

    // ── Button + touch IRQ pin init ─────────────────────────────
    gpio_init_input_pullup(cfg.btn_pin as i32);
    gpio_init_input_pullup(TOUCH_IRQ_PIN); // IRQ is active-LOW

    // ── Display SPI (SPI2 / HSPI) ───────────────────────────────
    // CLK=14, MOSI=13, MISO=12, CS=15, DC=2, BL=21
    let disp_spi = SpiDriver::new::<esp_idf_hal::spi::SPI2>(
        peripherals.spi2,
        peripherals.pins.gpio14,
        peripherals.pins.gpio13,
        Some(peripherals.pins.gpio12),
        &SpiDriverConfig::new(),
    )?;
    let disp_dev = SpiDeviceDriver::new(
        disp_spi,
        Some(peripherals.pins.gpio15),
        &SpiConfig::new().baudrate(55_000_000u32.Hz().into()),
    )?;
    let dc_pin = PinDriver::output(peripherals.pins.gpio2)?;

    // ── Backlight PWM ────────────────────────────────────────────
    let bl_timer = std::sync::Arc::new(LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &TimerConfig::default()
            .frequency(5_000u32.Hz().into())
            .resolution(Resolution::Bits8),
    )?);
    let mut backlight = LedcDriver::new(
        peripherals.ledc.channel0,
        bl_timer,
        peripherals.pins.gpio21,
    )?;
    backlight.set_duty(cfg.bl_full as u32)?;

    // ── Init ILI9341 in landscape mode ──────────────────────────
    let mut delay = FreeRtos;
    let di = SpiInterface::new(disp_dev, dc_pin);
    let mut display = Builder::new(mipidsi::models::ILI9341Rgb565, di)
        .orientation(Orientation::new().rotate(Rotation::Deg90))
        .color_order(ColorOrder::Bgr)
        .init(&mut delay)
        .map_err(|e| anyhow::anyhow!("Display init: {:?}", e))?;

    // ── Create WiFi (modem taken here; used for both STA and AP) ─
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs_partition.clone()))?,
        sysloop.clone(),
    )?;

    // ── AP mode if no SSID configured ───────────────────────────
    if cfg.wifi_ssid.is_empty() {
        info!("[WiFi] No SSID configured — AP mode");
        enter_ap_mode(&mut display, &cfg, &mut wifi, nvs_partition);
    }

    // ── Connect to WiFi as client ────────────────────────────────
    draw_splash(&mut display, "Connecting to WiFi...");

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid:     cfg.wifi_ssid.as_str().try_into().unwrap_or_default(),
        password: cfg.wifi_pass.as_str().try_into().unwrap_or_default(),
        ..Default::default()
    }))?;
    wifi.start()?;

    let connected = wifi.connect().and_then(|_| wifi.wait_netif_up()).is_ok();

    if !connected {
        warn!("[WiFi] Connection failed — AP mode");
        enter_ap_mode(&mut display, &cfg, &mut wifi, nvs_partition);
    }

    let device_ip = wifi.wifi().sta_netif().get_ip_info()?.ip.to_string();
    info!("[WiFi] Connected: {}", device_ip);

    // Brief IP splash so user knows how to reach the config page
    draw_splash(&mut display, &format!("IP: {}", device_ip));
    std::thread::sleep(Duration::from_millis(2500));

    // ── Web config server ────────────────────────────────────────
    let shared_cfg = Arc::new(Mutex::new(cfg.clone()));
    let _web_server = web::start_server(shared_cfg.clone(), nvs_partition.clone())?;

    // ── SNTP time sync (5-second timeout) ───────────────────────
    draw_splash(&mut display, "Syncing time (NTP)...");
    let sntp = EspSntp::new_default()?;
    let t0 = millis();
    while sntp.get_sync_status() != SyncStatus::Completed && millis() - t0 < 5_000 {
        std::thread::sleep(Duration::from_millis(200));
    }
    let ntp_synced = is_time_valid();
    info!("[NTP] {}", if ntp_synced { "synced" } else { "failed — continuing without time" });

    // ── Restore toggle state from NVS ────────────────────────────
    draw_splash(&mut display, "Fetching last state...");
    let (last_msg, saved_toggle_time) = load_state(nvs_partition.clone()).unwrap_or_default();
    info!("[Init] Last TG msg: \"{}\"", last_msg);

    let mut toggle_state = last_msg == cfg.msg_on;

    // Reconstruct toggled_at millis from the wall-clock timestamp saved to NVS
    let mut toggled_at: Option<u64> = if ntp_synced && saved_toggle_time > 0 {
        let now_secs = unix_now_secs();
        if now_secs >= saved_toggle_time as u64 {
            let elapsed = now_secs - saved_toggle_time as u64;
            Some(millis().saturating_sub(elapsed * 1000))
        } else {
            None
        }
    } else {
        None
    };

    if let Some(at) = toggled_at {
        info!("[Init] Restored toggle age: {} s", (millis() - at) / 1000);
    }

    let mut last_sent_text = last_msg;

    // Sync Telegram if local state disagrees with last sent message
    let expected_msg = if toggle_state { cfg.msg_on.clone() } else { cfg.msg_off.clone() };
    if last_sent_text != expected_msg {
        info!("[Init] Out of sync — sending current state to Telegram");
        if telegram::send_message(&cfg.bot_token, &cfg.chat_id, &expected_msg).unwrap_or(false) {
            last_sent_text = expected_msg.clone();
            let _ = save_state(nvs_partition.clone(), &expected_msg, unix_now_secs() as u32);
        }
    }

    // ── Initial frame ────────────────────────────────────────────
    set_led(toggle_state);
    draw_frame(&mut display, toggle_state, &device_ip);
    let mut cache = LiveCache::default();
    cache.invalidate();
    update_live_zones(&mut display, &mut cache, ntp_synced, toggled_at, unix_now_local_ms());

    // ── Main loop ─────────────────────────────────────────────────
    let mut last_tick    = millis();
    let mut dimmed       = false;
    let mut last_activity = millis();
    let mut last_btn_val  = true; // HIGH = released (INPUT_PULLUP, latching switch)
    let mut last_btn_change = millis();

    loop {
        let now = millis();

        // ── 1-second clock/elapsed tick ────────────────────────
        if now - last_tick >= 1000 {
            last_tick = now;
            update_live_zones(
                &mut display,
                &mut cache,
                ntp_synced,
                toggled_at,
                unix_now_local_ms(),
            );
        }

        // ── Backlight auto-dim after inactivity ────────────────
        if !dimmed && now - last_activity >= cfg.bl_dim_ms as u64 {
            info!("[DIM] Dimming backlight");
            let _ = backlight.set_duty(cfg.bl_dim as u32);
            dimmed = true;
        }

        // ── Touch wakes backlight ──────────────────────────────
        if dimmed && is_touched() {
            info!("[DIM] Wake on touch");
            let _ = backlight.set_duty(cfg.bl_full as u32);
            dimmed = false;
            last_activity = now;
        }

        // ── Physical latching switch debounce ──────────────────
        // LOW = OPEN, HIGH = CLOSED; 50 ms stable before acting
        let btn_val = gpio_read(cfg.btn_pin as i32);
        if btn_val != last_btn_val {
            last_btn_change = now;
            last_btn_val    = btn_val;
        }
        if now - last_btn_change < 50 {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }

        let new_state = !btn_val; // LOW pin → OPEN state
        if new_state == toggle_state {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }

        // State changed — wake screen if dimmed
        if dimmed {
            info!("[DIM] Wake on button");
            let _ = backlight.set_duty(cfg.bl_full as u32);
            dimmed = false;
        }
        last_activity = now;

        info!(
            "[BTN] pin={} → {}",
            if btn_val { "HIGH" } else { "LOW" },
            if new_state { "OPEN" } else { "CLOSED" }
        );

        toggle_state = new_state;
        toggled_at   = Some(now);

        set_led(toggle_state);
        draw_frame(&mut display, toggle_state, &device_ip);
        cache.invalidate();
        update_live_zones(&mut display, &mut cache, ntp_synced, toggled_at, unix_now_local_ms());

        // Send to Telegram if the message would differ from last sent
        let new_msg = if toggle_state { cfg.msg_on.clone() } else { cfg.msg_off.clone() };
        if new_msg != last_sent_text {
            show_notification(&mut display, "Sending to Telegram...", 0x000F);
            let ok = telegram::send_message(&cfg.bot_token, &cfg.chat_id, &new_msg)
                .unwrap_or(false);
            if ok {
                last_sent_text = new_msg.clone();
                let _ = save_state(nvs_partition.clone(), &new_msg, unix_now_secs() as u32);
            }
            std::thread::sleep(Duration::from_millis(600));
            show_notification(
                &mut display,
                if ok { "Sent!" } else { "Send failed!" },
                if ok { 0x0340 } else { 0x7800 },
            );
            std::thread::sleep(Duration::from_millis(900));
            draw_frame(&mut display, toggle_state, &device_ip);
            cache.invalidate();
            update_live_zones(
                &mut display,
                &mut cache,
                ntp_synced,
                toggled_at,
                unix_now_local_ms(),
            );
        }

        std::thread::sleep(Duration::from_millis(1));
    }
}
