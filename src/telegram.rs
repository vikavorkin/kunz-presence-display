use anyhow::Result;
use embedded_svc::http::client::Client;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use log::{error, info};
use std::io::Write;

// Send a Telegram message via the Bot API over HTTPS.
// Returns Ok(true) on HTTP 200, Ok(false) on API-level failure.
pub fn send_message(bot_token: &str, chat_id: &str, text: &str) -> Result<bool> {
    if bot_token.is_empty() || chat_id.is_empty() {
        info!("[TG] Skipping send — no token/chat_id configured");
        return Ok(false);
    }

    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
    let body = build_json(chat_id, text);

    info!("[TG] Sending to chat {}: {}", chat_id, text);

    let conn = EspHttpConnection::new(&HttpConfig {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),
        ..Default::default()
    })?;

    let mut client = Client::wrap(conn);

    let body_len = body.len().to_string();
    let headers = [
        ("Content-Type", "application/json"),
        ("Content-Length", body_len.as_str()),
    ];

    let mut request = client.post(&url, &headers)?;
    request.write_all(body.as_bytes())?;
    request.flush()?;

    let response = request.submit()?;
    let status = response.status();

    if status == 200 {
        info!("[TG] OK");
        Ok(true)
    } else {
        error!("[TG] HTTP {}", status);
        Ok(false)
    }
}

fn build_json(chat_id: &str, text: &str) -> String {
    format!(r#"{{"chat_id":"{}","text":"{}"}}"#, chat_id, json_escape(text))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"'  => out.push_str(r#"\""#),
            '\\' => out.push_str(r#"\\"#),
            '\n' => out.push_str(r#"\n"#),
            '\r' => out.push_str(r#"\r"#),
            '\t' => out.push_str(r#"\t"#),
            c    => out.push(c),
        }
    }
    out
}
