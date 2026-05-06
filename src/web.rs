use anyhow::Result;
use esp_idf_svc::{
    http::server::{Configuration as ServerConfig, EspHttpServer},
    nvs::EspDefaultNvsPartition,
};
use kunz_presence_display::logic::parse_form_urlencoded;
use log::{error, info};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use crate::config::{save_config, AppConfig};

static CONFIG_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Presence Display — Config</title>
<style>
  body{font-family:sans-serif;background:#111;color:#eee;display:flex;
       justify-content:center;align-items:center;min-height:100vh;margin:0}
  .card{background:#1e1e1e;border-radius:12px;padding:2rem;width:min(380px,90vw);
        box-shadow:0 4px 24px #0006}
  h1{margin:0 0 .25rem;font-size:1.3rem;color:#4af}
  h2{margin:.5rem 0 1rem;font-size:.9rem;color:#666;font-weight:normal;
     border-bottom:1px solid #333;padding-bottom:.5rem}
  label{display:block;margin-bottom:.3rem;font-size:.85rem;color:#aaa}
  .hint{font-size:.75rem;color:#555;margin-top:-.1rem;margin-bottom:1rem}
  input{width:100%;box-sizing:border-box;padding:.6rem .8rem;border-radius:6px;
        border:1px solid #444;background:#111;color:#eee;font-size:.95rem;margin-bottom:.4rem}
  input:focus{outline:none;border-color:#4af}
  .row{display:flex;gap:.75rem}
  .row input{margin-bottom:.4rem}
  .mb{margin-bottom:1.2rem}
  button{width:100%;padding:.75rem;border:none;border-radius:6px;margin-top:.8rem;
         background:#1a6fb5;color:#fff;font-size:1rem;cursor:pointer}
  button:hover{background:#2280cc}
</style>
</head>
<body>
<div class="card">
  <h1>&#9881; Device Config</h1>
  <form method="POST" action="/save">
    <h2>WiFi</h2>
    <label>SSID</label>
    <input name="wifiSsid" type="text" autocomplete="off" placeholder="MyNetwork" value="{WIFISSID}">
    <p class="hint">Takes effect after restart</p>
    <label>Password</label>
    <input name="wifiPass" type="password" autocomplete="off" placeholder="••••••••" value="{WIFIPASS}" class="mb">

    <h2>Telegram</h2>
    <label>Bot Token</label>
    <input name="botToken" type="text" autocomplete="off" placeholder="1234567890:AAB..." value="{TOKEN}">
    <p class="hint">Obtain from @BotFather on Telegram</p>
    <label>Chat ID</label>
    <input name="chatId" type="text" autocomplete="off" placeholder="123456789" value="{CHATID}" class="mb">
    <label>Open message</label>
    <input name="msgOn" type="text" autocomplete="off" placeholder="Place is now open!" value="{MSGON}">
    <p class="hint">Sent when toggled to OPEN — emoji supported</p>
    <label>Closed message</label>
    <input name="msgOff" type="text" autocomplete="off" placeholder="Place is closed :(" value="{MSGOFF}" class="mb">
    <p class="hint">Sent when toggled to CLOSED — emoji supported</p>

    <h2>Hardware</h2>
    <div class="row">
      <div style="flex:1">
        <label>Button GPIO pin</label>
        <input name="btnPin" type="number" min="0" max="39" value="{BTNPIN}" class="mb">
      </div>
      <div style="flex:1">
        <label>Dim after (seconds)</label>
        <input name="blDimSec" type="number" min="5" max="3600" value="{BLDIMSEC}" class="mb">
      </div>
    </div>
    <div class="row">
      <div style="flex:1">
        <label>Backlight full (0-255)</label>
        <input name="blFull" type="number" min="0" max="255" value="{BLFULL}" class="mb">
      </div>
      <div style="flex:1">
        <label>Backlight dim (0-255)</label>
        <input name="blDim" type="number" min="0" max="255" value="{BLDIM}" class="mb">
      </div>
    </div>

    <button type="submit">Save &amp; Restart</button>
  </form>
</div>
</body>
</html>"#;

static SAVED_HTML: &str = r#"<!DOCTYPE html>
<html><head><meta charset='utf-8'>
<style>body{font-family:sans-serif;background:#111;color:#eee;
display:flex;justify-content:center;align-items:center;height:100vh}
.m{text-align:center}.m h2{color:#4d4}</style></head>
<body><div class='m'><h2>&#10003; Saved!</h2>
<p>Restarting device&hellip;</p></div></body></html>"#;

pub fn start_server(
    shared_cfg: Arc<Mutex<AppConfig>>,
    nvs_partition: EspDefaultNvsPartition,
) -> Result<EspHttpServer<'static>> {
    let server_config = ServerConfig {
        stack_size: 12288, // extra headroom for OTA write buffer
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&server_config)?;

    // ── GET / — config form ──────────────────────────────────────
    let cfg_for_get = shared_cfg.clone();
    server.fn_handler::<anyhow::Error, _>("/", esp_idf_svc::http::Method::Get, move |req| {
        let cfg = cfg_for_get.lock().unwrap();
        let page = CONFIG_HTML
            .replace("{WIFISSID}", &cfg.wifi_ssid)
            .replace("{WIFIPASS}", &cfg.wifi_pass)
            .replace("{TOKEN}",    &cfg.bot_token)
            .replace("{CHATID}",   &cfg.chat_id)
            .replace("{MSGON}",    &cfg.msg_on)
            .replace("{MSGOFF}",   &cfg.msg_off)
            .replace("{BTNPIN}",   &cfg.btn_pin.to_string())
            .replace("{BLDIMSEC}", &(cfg.bl_dim_ms / 1000).to_string())
            .replace("{BLFULL}",   &cfg.bl_full.to_string())
            .replace("{BLDIM}",    &cfg.bl_dim.to_string());
        drop(cfg);

        req.into_response(200, Some("OK"), &[("Content-Type", "text/html; charset=utf-8")])?
            .write_all(page.as_bytes())?;
        Ok(())
    })?;

    // ── POST /save — validate, persist, restart ──────────────────
    let cfg_for_save = shared_cfg;
    let nvs_for_save = nvs_partition.clone();
    server.fn_handler::<anyhow::Error, _>("/save", esp_idf_svc::http::Method::Post, move |mut req| {
        let body = read_body(&mut req);
        let form = parse_form_urlencoded(&body);

        let get = |key: &str| -> Result<String> {
            form.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| anyhow::anyhow!("Missing field: {}", key))
        };

        let wifi_ssid = get("wifiSsid")?.trim().to_string();
        let wifi_pass = get("wifiPass")?.trim().to_string();
        let bot_token = get("botToken")?.trim().to_string();
        let chat_id   = get("chatId")?.trim().to_string();
        let msg_on    = get("msgOn")?.trim().to_string();
        let msg_off   = get("msgOff")?.trim().to_string();
        let btn_pin: u8  = get("btnPin")?.trim().parse()?;
        let dim_sec: u32 = get("blDimSec")?.trim().parse()?;
        let bl_full: u8  = get("blFull")?.trim().parse()?;
        let bl_dim: u8   = get("blDim")?.trim().parse()?;

        if wifi_ssid.is_empty() {
            anyhow::bail!("WiFi SSID must not be empty");
        }
        if bot_token.is_empty() || chat_id.is_empty() || msg_on.is_empty() || msg_off.is_empty() {
            anyhow::bail!("Telegram fields must not be empty");
        }
        if btn_pin > 39                    { anyhow::bail!("Button GPIO must be 0-39"); }
        if !(5..=3600).contains(&dim_sec)  { anyhow::bail!("Dim timeout must be 5-3600 s"); }
        if bl_dim >= bl_full               { anyhow::bail!("Dim brightness must be less than full brightness"); }

        let new_cfg = {
            let mut cfg = cfg_for_save.lock().unwrap();
            cfg.wifi_ssid = wifi_ssid;
            cfg.wifi_pass = wifi_pass;
            cfg.bot_token = bot_token;
            cfg.chat_id   = chat_id;
            cfg.msg_on    = msg_on;
            cfg.msg_off   = msg_off;
            cfg.btn_pin   = btn_pin;
            cfg.bl_dim_ms = dim_sec * 1000;
            cfg.bl_full   = bl_full;
            cfg.bl_dim    = bl_dim;
            cfg.clone()
        };

        save_config(nvs_for_save.clone(), &new_cfg)?;

        req.into_response(200, Some("OK"), &[("Content-Type", "text/html; charset=utf-8")])?
            .write_all(SAVED_HTML.as_bytes())?;

        info!("[WEB] Config saved — scheduling restart");
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(800));
            unsafe { esp_idf_sys::esp_restart() };
        });

        Ok(())
    })?;

    // ── POST /ota — receive firmware binary and apply via ESP-IDF OTA ──
    //
    // Usage (after building):
    //   scripts/ota_upload.sh <device-ip>
    //
    // The device applies the update and reboots into the new firmware.
    // The flash partition layout must have two OTA slots
    // (CONFIG_PARTITION_TABLE_TWO_OTA=y in sdkconfig.defaults).
    server.fn_handler::<anyhow::Error, _>("/ota", esp_idf_svc::http::Method::Post, |mut req| {
        info!("[OTA] Firmware upload started");

        let mut ota = esp_idf_svc::ota::EspOta::new()
            .map_err(|e| anyhow::anyhow!("OTA init: {:?}", e))?;
        let mut update = ota
            .initiate_update()
            .map_err(|e| anyhow::anyhow!("OTA initiate: {:?}", e))?;

        let mut buf = [0u8; 4096];
        let mut total = 0usize;
        let mut write_err: Option<String> = None;

        loop {
            let n = match req.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    write_err = Some(format!("read error: {:?}", e));
                    break;
                }
            };
            if let Err(e) = update.write_all(&buf[..n]) {
                write_err = Some(format!("write error: {:?}", e));
                break;
            }
            total += n;
        }

        if let Some(err) = write_err {
            error!("[OTA] Aborting: {}", err);
            let _ = update.abort();
            req.into_response(500, Some("OTA Failed"), &[("Content-Type", "text/plain")])?
                .write_all(format!("OTA failed: {}", err).as_bytes())?;
            return Ok(());
        }

        info!("[OTA] Received {} bytes — completing update", total);
        update
            .complete()
            .map_err(|e| anyhow::anyhow!("OTA complete: {:?}", e))?;

        req.into_response(200, Some("OK"), &[("Content-Type", "text/plain")])?
            .write_all(format!("OTA OK ({} bytes) — rebooting", total).as_bytes())?;

        info!("[OTA] Update applied — rebooting in 500 ms");
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(500));
            unsafe { esp_idf_sys::esp_restart() };
        });

        Ok(())
    })?;

    info!("[WEB] Server started (/, /save, /ota)");
    Ok(server)
}

fn read_body<R: Read>(r: &mut R) -> Vec<u8> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body
}
