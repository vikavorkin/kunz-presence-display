use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use log::info;

pub const DEFAULT_WIFI_SSID: &str = "kunz_gsm2.4";
pub const DEFAULT_WIFI_PASS: &str = "kunzkunz";
pub const DEFAULT_BOT_TOKEN: &str = "";
pub const DEFAULT_CHAT_ID: &str = "";
pub const DEFAULT_MSG_ON: &str = "Place is now open!";
pub const DEFAULT_MSG_OFF: &str = "Place is closed :(";
pub const DEFAULT_BTN_PIN: u8 = 22;
pub const DEFAULT_BL_DIM_MS: u32 = 60_000;
pub const DEFAULT_BL_FULL: u8 = 255;
pub const DEFAULT_BL_DIM: u8 = 10;

pub const AP_SSID: &str = "PresenceSetup";
pub const NTP_UTC_OFFSET_SEC: i32 = 2 * 3600; // UTC+2 (Helsinki/Kyiv); change to your timezone

const NS_CFG: &str = "tgcfg";
const NS_STATE: &str = "tgstate";

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub bot_token: String,
    pub chat_id: String,
    pub msg_on: String,
    pub msg_off: String,
    pub btn_pin: u8,
    pub bl_dim_ms: u32,
    pub bl_full: u8,
    pub bl_dim: u8,
    pub ap_pass: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wifi_ssid: DEFAULT_WIFI_SSID.to_string(),
            wifi_pass: DEFAULT_WIFI_PASS.to_string(),
            bot_token: DEFAULT_BOT_TOKEN.to_string(),
            chat_id: DEFAULT_CHAT_ID.to_string(),
            msg_on: DEFAULT_MSG_ON.to_string(),
            msg_off: DEFAULT_MSG_OFF.to_string(),
            btn_pin: DEFAULT_BTN_PIN,
            bl_dim_ms: DEFAULT_BL_DIM_MS,
            bl_full: DEFAULT_BL_FULL,
            bl_dim: DEFAULT_BL_DIM,
            ap_pass: String::new(),
        }
    }
}

fn get_str(nvs: &EspNvs<NvsDefault>, key: &str) -> Option<String> {
    let mut buf = [0u8; 256];
    nvs.get_str(key, &mut buf).ok().flatten().map(|s| s.to_string())
}

pub fn load_config(partition: EspDefaultNvsPartition) -> Result<AppConfig> {
    let nvs = EspNvs::new(partition.clone(), NS_CFG, false)?;
    let mut cfg = AppConfig::default();

    if let Some(v) = get_str(&nvs, "wifiSsid") { cfg.wifi_ssid = v; }
    if let Some(v) = get_str(&nvs, "wifiPass") { cfg.wifi_pass = v; }
    if let Some(v) = get_str(&nvs, "botToken") { cfg.bot_token = v; }
    if let Some(v) = get_str(&nvs, "chatId")   { cfg.chat_id = v; }
    if let Some(v) = get_str(&nvs, "msgOn")    { cfg.msg_on = v; }
    if let Some(v) = get_str(&nvs, "msgOff")   { cfg.msg_off = v; }
    if let Ok(Some(v)) = nvs.get_u8("btnPin")  { cfg.btn_pin = v; }
    if let Ok(Some(v)) = nvs.get_u32("blDimMs"){ cfg.bl_dim_ms = v; }
    if let Ok(Some(v)) = nvs.get_u8("blFull")  { cfg.bl_full = v; }
    if let Ok(Some(v)) = nvs.get_u8("blDim")   { cfg.bl_dim = v; }

    // Generate AP password on first boot and persist it
    let ap_pass = get_str(&nvs, "apPass").unwrap_or_default();
    if ap_pass.is_empty() {
        let new_pass = gen_ap_pass();
        // Drop the read-write handle then reopen to write
        drop(nvs);
        let nvs_w = EspNvs::new(partition, NS_CFG, false)?;
        nvs_w.set_str("apPass", &new_pass)?;
        info!("[CFG] Generated AP password: {}", new_pass);
        cfg.ap_pass = new_pass;
    } else {
        cfg.ap_pass = ap_pass;
    }

    info!(
        "[CFG] wifi_ssid={} bot_token={} chat_id={} btn_pin={} bl_dim_ms={} bl_full={} bl_dim={}",
        cfg.wifi_ssid, cfg.bot_token, cfg.chat_id,
        cfg.btn_pin, cfg.bl_dim_ms, cfg.bl_full, cfg.bl_dim
    );
    info!("[CFG] msg_on={}  msg_off={}", cfg.msg_on, cfg.msg_off);
    Ok(cfg)
}

pub fn save_config(partition: EspDefaultNvsPartition, cfg: &AppConfig) -> Result<()> {
    let nvs = EspNvs::new(partition, NS_CFG, false)?;
    nvs.set_str("wifiSsid", &cfg.wifi_ssid)?;
    nvs.set_str("wifiPass", &cfg.wifi_pass)?;
    nvs.set_str("botToken", &cfg.bot_token)?;
    nvs.set_str("chatId",   &cfg.chat_id)?;
    nvs.set_str("msgOn",    &cfg.msg_on)?;
    nvs.set_str("msgOff",   &cfg.msg_off)?;
    nvs.set_u8("btnPin",    cfg.btn_pin)?;
    nvs.set_u32("blDimMs",  cfg.bl_dim_ms)?;
    nvs.set_u8("blFull",    cfg.bl_full)?;
    nvs.set_u8("blDim",     cfg.bl_dim)?;
    info!("[CFG] Saved config");
    Ok(())
}

pub fn load_state(partition: EspDefaultNvsPartition) -> Result<(String, u32)> {
    let nvs = EspNvs::new(partition, NS_STATE, false)?;
    let last_msg = get_str(&nvs, "lastMsg").unwrap_or_default();
    let toggle_time = nvs.get_u32("toggleTime").ok().flatten().unwrap_or(0);
    Ok((last_msg, toggle_time))
}

pub fn save_state(partition: EspDefaultNvsPartition, last_msg: &str, toggle_time: u32) -> Result<()> {
    let nvs = EspNvs::new(partition, NS_STATE, false)?;
    nvs.set_str("lastMsg", last_msg)?;
    nvs.set_u32("toggleTime", toggle_time)?;
    Ok(())
}

fn gen_ap_pass() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/I/1
    let mut pass = String::with_capacity(8);
    for _ in 0..8 {
        let idx = (unsafe { esp_idf_sys::esp_random() } as usize) % CHARSET.len();
        pass.push(CHARSET[idx] as char);
    }
    pass
}
