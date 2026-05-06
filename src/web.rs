use anyhow::Result;
use esp_idf_svc::{
    http::server::{Configuration as ServerConfig, EspHttpServer},
    nvs::EspDefaultNvsPartition,
};
use log::info;
use std::io::Read;
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
        stack_size: 10240,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&server_config)?;

    // GET / — serve config form
    let cfg_for_get = shared_cfg.clone();
    server.fn_handler::<anyhow::Error, _>("/", esp_idf_svc::http::Method::Get, move |req| {
        let cfg = cfg_for_get.lock().unwrap();
        let page = CONFIG_HTML
            .replace("{WIFISSID}", &cfg.wifi_ssid)
            .replace("{WIFIPASS}", &cfg.wifi_pass)
            .replace("{TOKEN}",   &cfg.bot_token)
            .replace("{CHATID}",  &cfg.chat_id)
            .replace("{MSGON}",   &cfg.msg_on)
            .replace("{MSGOFF}",  &cfg.msg_off)
            .replace("{BTNPIN}",  &cfg.btn_pin.to_string())
            .replace("{BLDIMSEC}", &(cfg.bl_dim_ms / 1000).to_string())
            .replace("{BLFULL}",  &cfg.bl_full.to_string())
            .replace("{BLDIM}",   &cfg.bl_dim.to_string());
        drop(cfg);

        req.into_response(200, Some("OK"), &[("Content-Type", "text/html; charset=utf-8")])?
            .write_all(page.as_bytes())?;
        Ok(())
    })?;

    // POST /save — validate, persist, restart
    let cfg_for_save = shared_cfg;
    let nvs_for_save = nvs_partition;
    server.fn_handler::<anyhow::Error, _>("/save", esp_idf_svc::http::Method::Post, move |mut req| {
        let mut body = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            match req.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }

        let form = match parse_form_urlencoded(&body) {
            Ok(f) => f,
            Err(e) => {
                req.into_response(400, Some("Bad Request"), &[])?
                    .write_all(e.to_string().as_bytes())?;
                return Ok(());
            }
        };

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
        let btn_pin: u8 = get("btnPin")?.trim().parse()?;
        let dim_sec: u32 = get("blDimSec")?.trim().parse()?;
        let bl_full: u8  = get("blFull")?.trim().parse()?;
        let bl_dim: u8   = get("blDim")?.trim().parse()?;

        // Validate
        if wifi_ssid.is_empty() {
            req.into_response(400, Some("Bad Request"), &[])?
                .write_all(b"WiFi SSID must not be empty")?;
            return Ok(());
        }
        if bot_token.is_empty() || chat_id.is_empty() || msg_on.is_empty() || msg_off.is_empty() {
            req.into_response(400, Some("Bad Request"), &[])?
                .write_all(b"Telegram fields must not be empty")?;
            return Ok(());
        }
        if btn_pin > 39 { anyhow::bail!("Button GPIO must be 0-39"); }
        if !(5..=3600).contains(&dim_sec) { anyhow::bail!("Dim timeout 5-3600 s"); }
        if bl_dim >= bl_full { anyhow::bail!("Dim brightness must be less than full"); }

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

    info!("[WEB] Config server started");
    Ok(server)
}

// Decode application/x-www-form-urlencoded body into key-value pairs.
fn parse_form_urlencoded(body: &[u8]) -> Result<Vec<(String, String)>> {
    let s = std::str::from_utf8(body)?;
    let mut pairs = Vec::new();
    for part in s.split('&') {
        let mut it = part.splitn(2, '=');
        let key = url_decode(it.next().unwrap_or(""));
        let val = url_decode(it.next().unwrap_or(""));
        pairs.push((key, val));
    }
    Ok(pairs)
}

fn url_decode(s: &str) -> String {
    // Percent-encoded UTF-8: collect raw bytes first, then interpret as UTF-8.
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let src = s.as_bytes();
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'+' => { bytes.push(b' '); i += 1; }
            b'%' if i + 2 < src.len() => {
                if let (Some(h), Some(l)) = (hex_val(src[i + 1]), hex_val(src[i + 2])) {
                    bytes.push(h << 4 | l);
                    i += 3;
                } else {
                    bytes.push(b'%');
                    i += 1;
                }
            }
            b => { bytes.push(b); i += 1; }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
