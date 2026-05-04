/*
 * CYD Telegram Toggle Bot  — v2 with Clock + Elapsed Timer
 * Hardware: ESP32-2432S028R ("Cheap Yellow Display")
 *
 * Screen layout (320x240 landscape):
 * ┌─────────────────────────────────────────┐  y=0
 * │  [HH:MM:SS]           [DD Mon YYYY]     │  top bar  0-38
 * ├─────────────────────────────────────────┤  y=38
 * │  ┌────────────────────────────────────┐ │
 * │  │           ON  /  OFF               │ │  state badge 44-104
 * │  └────────────────────────────────────┘ │
 * │  Since last toggle: 02m 14s             │  elapsed 112-124
 * └─────────────────────────────────────────┘  y=240
 *
 * Toggle: physical button on IO22 (pull to GND)
 *
 * Features:
 *   - NTP time sync (configure UTC offset below)
 *   - Live clock + date, updated every second via partial redraw (no flicker)
 *   - Elapsed time since last toggle, counting up every second
 *   - Boot: fetches last Telegram bot message to restore state
 *   - Touch toggle button -> sends Telegram only when state changes
 *   - RGB LED mirrors state (green = ON)
 *   - Web config UI at http://<device-ip>/ to set bot token & chat ID
 *
 * Libraries (Arduino Library Manager):
 *   - TFT_eSPI            by Bodmer          ** needs User_Setup.h (see bottom) **
 *   - XPT2046_Touchscreen by Paul Stoffregen
 *   - UniversalTelegramBot by Brian Lough    >= 1.3.0
 *   - ArduinoJson         by Benoit Blanchon >= 6.x
 */

#include <SPI.h>
#include <WiFi.h>
#include <WiFiClientSecure.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <WebServer.h>
#include <ArduinoOTA.h>
#include <time.h>
#include <TFT_eSPI.h>
#include <XPT2046_Touchscreen.h>
#include <UniversalTelegramBot.h>
#include <ArduinoJson.h>

// ─────────────────────────────────────────────
// USER CONFIGURATION (compile-time defaults)
// These are used only if no value has been saved via the web interface.
// ─────────────────────────────────────────────
const char* WIFI_SSID     = "kunz_gsm2.4";
const char* WIFI_PASSWORD = "kunzkunz";

// Default Telegram credentials (overridden by NVS / web config)
#define DEFAULT_BOT_TOKEN  ""
#define DEFAULT_CHAT_ID    ""
#define DEFAULT_MSG_ON     "Place is now open!"
#define DEFAULT_MSG_OFF    "Place is closed :("

// Default Cloudflare Queue settings (overridden by NVS / web config)
// Queue HTTP endpoint: https://api.cloudflare.com/client/v4/accounts/{id}/queues/{id}/messages
#define DEFAULT_CF_QUEUE_URL  ""
#define DEFAULT_CF_API_TOKEN  ""
#define DEFAULT_CF_QUEUE_SEC  60   // periodic status publish interval in seconds
#define DEFAULT_CF_SVC_NAME   "presence-display"  // identifies this device in queue messages

// NTP timezone — seconds east of UTC
//   UTC+0  London winter:  0
//   UTC+1  Berlin winter:  3600
//   UTC+2  Helsinki/Kyiv:  7200
//   UTC+3  Moscow:         10800
//   UTC-5  New York EST:  -18000
const long  NTP_UTC_OFFSET = 2 * 3600; // <-- set your offset here
const int   NTP_DST_OFFSET = 0;        // add 3600 during summer/DST if not auto
const char* NTP_SERVER     = "pool.ntp.org";

// MSG_ON / MSG_OFF are now runtime-configurable; these compile-time strings are kept
// only as fallback defaults when NVS is empty — use cfgMsgOn / cfgMsgOff everywhere.
// ─────────────────────────────────────────────

// ── CYD Pin Definitions ───────────────────────
#define XPT2046_IRQ   36
#define XPT2046_MOSI  32
#define XPT2046_MISO  39
#define XPT2046_CLK   25
#define XPT2046_CS    33

#define LED_RED    4
#define LED_GREEN 16
#define LED_BLUE  17

// PIN_BUTTON, BL_DIM_AFTER_MS, BL_FULL, and BL_DIM are runtime-configurable via the web UI.
// These compile-time values are the defaults written to NVS on first boot.
#define DEFAULT_BTN_PIN      22
#define DEFAULT_BL_DIM_AFTER_MS (60 * 1000UL)
#define DEFAULT_BL_FULL      255
#define DEFAULT_BL_DIM        10

#define TFT_BL_PIN      21
#define BL_PWM_CHANNEL   0
#define BL_PWM_FREQ   5000
#define BL_PWM_RES       8   // 8-bit: 0–255

#define SCREEN_W 320
#define SCREEN_H 240

// Touch calibration — run TouchCalibrate example if touches feel offset
#define TOUCH_MIN_X  200
#define TOUCH_MAX_X  3700
#define TOUCH_MIN_Y  240
#define TOUCH_MAX_Y  3800

// ── Layout ────────────────────────────────────
#define Y_TOPBAR_H    38
#define Y_BADGE_TOP   44
#define Y_BADGE_H     60
#define Y_ELAPSED_TOP 112

// ── Colours ───────────────────────────────────
#define COL_BG        TFT_BLACK
#define COL_TOPBAR    0x0014    // very dark navy
#define COL_ON_BADGE  0x0460    // dark green-teal
#define COL_OFF_BADGE 0x2104    // dark charcoal
#define COL_ELAPSED   0x8C71    // medium grey

// ── Runtime config (loaded from NVS, overrides compile-time defaults) ─
char     cfgWifiSsid[64];
char     cfgWifiPass[64];
char     cfgBotToken[128];
char     cfgChatId[32];
char     cfgMsgOn[128];
char     cfgMsgOff[128];
uint8_t  cfgBtnPin    = DEFAULT_BTN_PIN;
uint32_t cfgBlDimMs   = DEFAULT_BL_DIM_AFTER_MS;
uint8_t  cfgBlFull    = DEFAULT_BL_FULL;
uint8_t  cfgBlDim     = DEFAULT_BL_DIM;
char     cfgApPass[16]; // random AP password, generated once and stored in NVS
char     cfgCfQueueUrl[192]; // Cloudflare Queue HTTP endpoint URL (empty = disabled)
char     cfgCfApiToken[72];  // Cloudflare API token with Queue write permission
uint32_t cfgCfQueueSec = DEFAULT_CF_QUEUE_SEC; // periodic publish interval (seconds)
char     cfgCfSvcName[64];   // service/device label included in every queue message

// ── Objects ───────────────────────────────────
TFT_eSPI tft;
SPIClass touchSPI(VSPI);
XPT2046_Touchscreen ts(XPT2046_CS, XPT2046_IRQ);
WiFiClientSecure secureClient;
UniversalTelegramBot* bot = nullptr;
Preferences prefs;
WebServer webServer(80);

// ── State ─────────────────────────────────────
String        deviceIP       = "";
bool          toggleState    = false;
volatile bool          otaInProgress  = false;
String        lastSentText   = "";
unsigned long toggledAt      = 0;    // millis() at last toggle; 0 = unknown
bool          ntpSynced      = false;
unsigned long lastActivityAt = 0;    // millis() of last button press (for dimming)

// Cache for partial redraws
static char prevTime[12]    = "";
static char prevDate[16]    = "";
static char prevElapsed[32] = "";

// ─────────────────────────────────────────────
// Telegram config — NVS persistence
// ─────────────────────────────────────────────

void loadConfig() {
  prefs.begin("tgcfg", /*readOnly=*/true);
  String wifiSsid = prefs.getString("wifiSsid",  WIFI_SSID);
  String wifiPass = prefs.getString("wifiPass",  WIFI_PASSWORD);
  String token  = prefs.getString("botToken",  DEFAULT_BOT_TOKEN);
  String chatId = prefs.getString("chatId",    DEFAULT_CHAT_ID);
  String msgOn  = prefs.getString("msgOn",     DEFAULT_MSG_ON);
  String msgOff = prefs.getString("msgOff",    DEFAULT_MSG_OFF);
  cfgBtnPin   = (uint8_t)  prefs.getUInt("btnPin",   DEFAULT_BTN_PIN);
  cfgBlDimMs  = (uint32_t) prefs.getUInt("blDimMs",  DEFAULT_BL_DIM_AFTER_MS);
  cfgBlFull   = (uint8_t)  prefs.getUInt("blFull",   DEFAULT_BL_FULL);
  cfgBlDim    = (uint8_t)  prefs.getUInt("blDim",    DEFAULT_BL_DIM);
  String apPass = prefs.getString("apPass", "");
  String cfQueueUrl = prefs.getString("cfQueueUrl", DEFAULT_CF_QUEUE_URL);
  String cfApiToken = prefs.getString("cfApiToken", DEFAULT_CF_API_TOKEN);
  cfgCfQueueSec     = prefs.getUInt("cfQueueSec",  DEFAULT_CF_QUEUE_SEC);
  String cfSvcName  = prefs.getString("cfSvcName",  DEFAULT_CF_SVC_NAME);
  prefs.end();
  wifiSsid.toCharArray(cfgWifiSsid, sizeof(cfgWifiSsid));
  wifiPass.toCharArray(cfgWifiPass, sizeof(cfgWifiPass));
  token.toCharArray(cfgBotToken, sizeof(cfgBotToken));
  chatId.toCharArray(cfgChatId,  sizeof(cfgChatId));
  msgOn.toCharArray(cfgMsgOn,    sizeof(cfgMsgOn));
  msgOff.toCharArray(cfgMsgOff,  sizeof(cfgMsgOff));

  if (apPass.length() == 0) {
    // Generate a random 8-char AP password on first boot and persist it
    static const char charset[] = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // omit I, O, 0, 1
    char newPass[9];
    for (int i = 0; i < 8; i++)
      newPass[i] = charset[esp_random() % (sizeof(charset) - 1)];
    newPass[8] = '\0';
    prefs.begin("tgcfg", false);
    prefs.putString("apPass", newPass);
    prefs.end();
    apPass = newPass;
    Serial.printf("[CFG] Generated new AP password: %s\n", newPass);
  }
  apPass.toCharArray(cfgApPass, sizeof(cfgApPass));
  cfQueueUrl.toCharArray(cfgCfQueueUrl, sizeof(cfgCfQueueUrl));
  cfApiToken.toCharArray(cfgCfApiToken, sizeof(cfgCfApiToken));
  cfSvcName.toCharArray(cfgCfSvcName,  sizeof(cfgCfSvcName));

  Serial.printf("[CFG] wifiSsid=%s  botToken=%s  chatId=%s  btnPin=%u  blDimMs=%u  blFull=%u  blDim=%u\n",
                cfgWifiSsid, cfgBotToken, cfgChatId, cfgBtnPin, cfgBlDimMs, cfgBlFull, cfgBlDim);
  Serial.printf("[CFG] msgOn=%s  msgOff=%s\n", cfgMsgOn, cfgMsgOff);
}

void saveConfig(const char* wifiSsid, const char* wifiPass,
                const char* token, const char* chatId,
                const char* msgOn, const char* msgOff,
                uint8_t btnPin, uint32_t blDimMs,
                uint8_t blFull, uint8_t blDim,
                const char* cfQueueUrl, const char* cfApiToken,
                uint32_t cfQueueSec, const char* cfSvcName) {
  prefs.begin("tgcfg", /*readOnly=*/false);
  prefs.putString("wifiSsid",  wifiSsid);
  prefs.putString("wifiPass",  wifiPass);
  prefs.putString("botToken",  token);
  prefs.putString("chatId",    chatId);
  prefs.putString("msgOn",     msgOn);
  prefs.putString("msgOff",    msgOff);
  prefs.putUInt("btnPin",      btnPin);
  prefs.putUInt("blDimMs",     blDimMs);
  prefs.putUInt("blFull",      blFull);
  prefs.putUInt("blDim",       blDim);
  prefs.putString("cfQueueUrl", cfQueueUrl);
  prefs.putString("cfApiToken", cfApiToken);
  prefs.putUInt("cfQueueSec",   cfQueueSec);
  prefs.putString("cfSvcName",  cfSvcName);
  prefs.end();
  Serial.printf("[CFG] Saved wifiSsid=%s  botToken=%s  chatId=%s  btnPin=%u  blDimMs=%u  blFull=%u  blDim=%u\n",
                wifiSsid, token, chatId, btnPin, blDimMs, blFull, blDim);
  Serial.printf("[CFG] Saved msgOn=%s  msgOff=%s\n", msgOn, msgOff);
  Serial.printf("[CFG] Saved cfQueueUrl=%s  cfQueueSec=%u  cfSvcName=%s\n",
                cfQueueUrl, cfQueueSec, cfSvcName);
}

// ─────────────────────────────────────────────
// Web server — config page
// ─────────────────────────────────────────────

static const char CONFIG_HTML[] PROGMEM = R"rawhtml(<!DOCTYPE html>
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
    <label for="wssid">SSID</label>
    <input id="wssid" name="wifiSsid" type="text" autocomplete="off"
           placeholder="MyNetwork" value="%WIFISSID%">
    <p class="hint">Takes effect after restart</p>
    <label for="wpass">Password</label>
    <input id="wpass" name="wifiPass" type="password" autocomplete="off"
           placeholder="&#x2022;&#x2022;&#x2022;&#x2022;&#x2022;&#x2022;&#x2022;&#x2022;" value="%WIFIPASS%" class="mb">

    <h2>Telegram</h2>
    <label for="tok">Bot Token</label>
    <input id="tok" name="botToken" type="text" autocomplete="off"
           placeholder="1234567890:AAB..." value="%TOKEN%">
    <p class="hint">Obtain from @BotFather on Telegram</p>
    <label for="cid">Chat ID</label>
    <input id="cid" name="chatId" type="text" autocomplete="off"
           placeholder="123456789" value="%CHATID%" class="mb">
    <label for="mon">Open message</label>
    <input id="mon" name="msgOn" type="text" autocomplete="off"
           placeholder="&#x1F7E2; Place is now open!" value="%MSGON%">
    <p class="hint">Sent when toggled to OPEN — emoji supported</p>
    <label for="moff">Closed message</label>
    <input id="moff" name="msgOff" type="text" autocomplete="off"
           placeholder="&#x1F534; Place is closed" value="%MSGOFF%" class="mb">
    <p class="hint">Sent when toggled to CLOSED — emoji supported</p>

    <h2>Hardware</h2>
    <div class="row">
      <div style="flex:1">
        <label for="btn">Button GPIO pin</label>
        <input id="btn" name="btnPin" type="number" min="0" max="39"
               value="%BTNPIN%" class="mb">
      </div>
      <div style="flex:1">
        <label for="dim">Dim after (seconds)</label>
        <input id="dim" name="blDimSec" type="number" min="5" max="3600"
               value="%BLDIMSEC%" class="mb">
      </div>
    </div>
    <div class="row">
      <div style="flex:1">
        <label for="blfull">Backlight full (0-255)</label>
        <input id="blfull" name="blFull" type="number" min="0" max="255"
               value="%BLFULL%" class="mb">
      </div>
      <div style="flex:1">
        <label for="bldim">Backlight dim (0-255)</label>
        <input id="bldim" name="blDim" type="number" min="0" max="255"
               value="%BLDIM%" class="mb">
      </div>
    </div>

    <h2>Cloudflare Queue</h2>
    <label for="cfsvc">Service name</label>
    <input id="cfsvc" name="cfSvcName" type="text" autocomplete="off"
           placeholder="presence-display" value="%CFSVNAME%">
    <p class="hint">Identifies this device in queue messages — useful when multiple devices share one queue</p>
    <label for="cfurl">Queue URL</label>
    <input id="cfurl" name="cfQueueUrl" type="text" autocomplete="off"
           placeholder="https://api.cloudflare.com/client/v4/accounts/.../queues/.../messages"
           value="%CFQUEUEURL%">
    <p class="hint">Leave blank to disable. Paste full endpoint URL from CF dashboard.</p>
    <label for="cftoken">API Token</label>
    <input id="cftoken" name="cfApiToken" type="password" autocomplete="off"
           placeholder="CF API token with Queue write permission" value="%CFAPITOKEN%">
    <p class="hint">Create at dash.cloudflare.com &rarr; My Profile &rarr; API Tokens</p>
    <label for="cfinterval">Publish interval (seconds)</label>
    <input id="cfinterval" name="cfQueueSec" type="number" min="10" max="3600"
           value="%CFQUEUESEC%" class="mb">
    <p class="hint">How often to periodically push status (10&ndash;3600 s). Also sent immediately on toggle.</p>

    <button type="submit">Save &amp; Restart</button>
  </form>
</div>
</body>
</html>)rawhtml";

void handleConfigRoot() {
  String page = FPSTR(CONFIG_HTML);
  page.replace("%WIFISSID%", String(cfgWifiSsid));
  page.replace("%WIFIPASS%", String(cfgWifiPass));
  page.replace("%TOKEN%",   String(cfgBotToken));
  page.replace("%CHATID%",  String(cfgChatId));
  page.replace("%MSGON%",   String(cfgMsgOn));
  page.replace("%MSGOFF%",  String(cfgMsgOff));
  page.replace("%BTNPIN%",  String(cfgBtnPin));
  page.replace("%BLDIMSEC%", String(cfgBlDimMs / 1000));
  page.replace("%BLFULL%",     String(cfgBlFull));
  page.replace("%BLDIM%",      String(cfgBlDim));
  page.replace("%CFSVNAME%",   String(cfgCfSvcName));
  page.replace("%CFQUEUEURL%", String(cfgCfQueueUrl));
  page.replace("%CFAPITOKEN%", String(cfgCfApiToken));
  page.replace("%CFQUEUESEC%", String(cfgCfQueueSec));
  webServer.send(200, "text/html", page);
}

void handleConfigSave() {
  if (!webServer.hasArg("wifiSsid") || !webServer.hasArg("wifiPass") ||
      !webServer.hasArg("botToken") || !webServer.hasArg("chatId") ||
      !webServer.hasArg("msgOn")    || !webServer.hasArg("msgOff") ||
      !webServer.hasArg("btnPin")   || !webServer.hasArg("blDimSec") ||
      !webServer.hasArg("blFull")   || !webServer.hasArg("blDim")   ||
      !webServer.hasArg("cfQueueSec")) {
    webServer.send(400, "text/plain", "Missing fields");
    return;
  }

  String wifiSsid   = webServer.arg("wifiSsid");
  String wifiPass   = webServer.arg("wifiPass");
  String token      = webServer.arg("botToken");
  String chatId     = webServer.arg("chatId");
  String msgOn      = webServer.arg("msgOn");
  String msgOff     = webServer.arg("msgOff");
  String btnPinS    = webServer.arg("btnPin");
  String dimSecS    = webServer.arg("blDimSec");
  String blFullS    = webServer.arg("blFull");
  String blDimS     = webServer.arg("blDim");
  String cfQueueUrl  = webServer.hasArg("cfQueueUrl") ? webServer.arg("cfQueueUrl") : "";
  String cfApiToken  = webServer.hasArg("cfApiToken") ? webServer.arg("cfApiToken") : "";
  String cfQueueSecS = webServer.arg("cfQueueSec");
  String cfSvcName   = webServer.hasArg("cfSvcName")  ? webServer.arg("cfSvcName")  : DEFAULT_CF_SVC_NAME;
  wifiSsid.trim(); wifiPass.trim();
  token.trim(); chatId.trim(); msgOn.trim(); msgOff.trim();
  btnPinS.trim(); dimSecS.trim(); blFullS.trim(); blDimS.trim();
  cfQueueUrl.trim(); cfApiToken.trim(); cfQueueSecS.trim(); cfSvcName.trim();
  if (cfSvcName.length() == 0) cfSvcName = DEFAULT_CF_SVC_NAME;

  if (wifiSsid.length() == 0 ||
      token.length() == 0 || chatId.length() == 0 ||
      msgOn.length() == 0  || msgOff.length() == 0 ||
      btnPinS.length() == 0 || dimSecS.length() == 0 ||
      blFullS.length() == 0 || blDimS.length() == 0) {
    webServer.send(400, "text/plain", "Fields must not be empty (WiFi password may be blank)");
    return;
  }
  if (cfQueueUrl.length() > 0 && cfApiToken.length() == 0) {
    webServer.send(400, "text/plain", "API Token required when Queue URL is set");
    return;
  }
  if (wifiSsid.length() >= sizeof(cfgWifiSsid) || wifiPass.length() >= sizeof(cfgWifiPass) ||
      token.length() >= sizeof(cfgBotToken) || chatId.length() >= sizeof(cfgChatId) ||
      msgOn.length() >= sizeof(cfgMsgOn)    || msgOff.length() >= sizeof(cfgMsgOff) ||
      cfQueueUrl.length() >= sizeof(cfgCfQueueUrl) || cfApiToken.length() >= sizeof(cfgCfApiToken) ||
      cfSvcName.length() >= sizeof(cfgCfSvcName)) {
    webServer.send(400, "text/plain", "Value too long");
    return;
  }

  int btnPin    = btnPinS.toInt();
  int dimSec    = dimSecS.toInt();
  int blFull    = blFullS.toInt();
  int blDim     = blDimS.toInt();
  int cfQueueSec = cfQueueSecS.length() > 0 ? cfQueueSecS.toInt() : DEFAULT_CF_QUEUE_SEC;
  if (btnPin < 0 || btnPin > 39) {
    webServer.send(400, "text/plain", "Button GPIO must be 0-39");
    return;
  }
  if (dimSec < 5 || dimSec > 3600) {
    webServer.send(400, "text/plain", "Dim timeout must be 5-3600 s");
    return;
  }
  if (blFull < 0 || blFull > 255 || blDim < 0 || blDim > 255) {
    webServer.send(400, "text/plain", "Backlight values must be 0-255");
    return;
  }
  if (blDim >= blFull) {
    webServer.send(400, "text/plain", "Dim brightness must be less than full brightness");
    return;
  }
  if (cfQueueSec < 10 || cfQueueSec > 3600) {
    webServer.send(400, "text/plain", "CF Queue interval must be 10-3600 s");
    return;
  }

  saveConfig(wifiSsid.c_str(), wifiPass.c_str(),
             token.c_str(), chatId.c_str(),
             msgOn.c_str(), msgOff.c_str(),
             (uint8_t)btnPin, (uint32_t)(dimSec * 1000),
             (uint8_t)blFull, (uint8_t)blDim,
             cfQueueUrl.c_str(), cfApiToken.c_str(),
             (uint32_t)cfQueueSec, cfSvcName.c_str());

  webServer.send(200, "text/html",
    "<html><head><meta charset='utf-8'>"
    "<style>body{font-family:sans-serif;background:#111;color:#eee;"
    "display:flex;justify-content:center;align-items:center;height:100vh}"
    ".m{text-align:center}.m h2{color:#4d4}</style></head>"
    "<body><div class='m'><h2>&#10003; Saved!</h2>"
    "<p>Restarting device&hellip;</p></div></body></html>");

  delay(800);
  ESP.restart();
}

void setupWebServer() {
  webServer.on("/",     HTTP_GET,  handleConfigRoot);
  webServer.on("/save", HTTP_POST, handleConfigSave);
  webServer.begin();
  Serial.printf("[WEB] Config server at http://%s/\n",
                WiFi.localIP().toString().c_str());
}

// ─────────────────────────────────────────────
// OTA
// ─────────────────────────────────────────────

void setupOTA() {
  ArduinoOTA.setHostname("presence-display");

  ArduinoOTA.onStart([]() {
    otaInProgress = true;
    drawSplash("OTA update...");
    Serial.println("[OTA] Start");
  });
  ArduinoOTA.onEnd([]() {
    otaInProgress = false;
    Serial.println("\n[OTA] Done");
  });
  ArduinoOTA.onProgress([](unsigned int progress, unsigned int total) {
    Serial.printf("[OTA] %u%%\r", progress * 100 / total);
  });
  ArduinoOTA.onError([](ota_error_t error) {
    otaInProgress = false;
    Serial.printf("[OTA] Error[%u]\n", error);
  });

  ArduinoOTA.begin();
  Serial.println("[OTA] Ready");
}

// ─────────────────────────────────────────────
// Time helpers
// ─────────────────────────────────────────────

bool isTimeValid() {
  return time(nullptr) > 1700000000UL;
}

void getTimeStr(char* buf, size_t len) {
  time_t now = time(nullptr);
  struct tm* t = localtime(&now);
  snprintf(buf, len, "%02d:%02d:%02d", t->tm_hour, t->tm_min, t->tm_sec);
}

void getDateStr(char* buf, size_t len) {
  static const char* mon[] = {
    "Jan","Feb","Mar","Apr","May","Jun",
    "Jul","Aug","Sep","Oct","Nov","Dec"
  };
  time_t now = time(nullptr);
  struct tm* t = localtime(&now);
  snprintf(buf, len, "%02d %s %04d", t->tm_mday, mon[t->tm_mon], t->tm_year + 1900);
}

void formatElapsed(unsigned long totalSec, char* buf, size_t len) {
  unsigned long s = totalSec % 60;
  unsigned long m = (totalSec / 60) % 60;
  unsigned long h = (totalSec / 3600) % 24;
  unsigned long d = totalSec / 86400;

  if (d > 0)
    snprintf(buf, len, "%lud %02luh %02lum %02lus", d, h, m, s);
  else if (h > 0)
    snprintf(buf, len, "%luh %02lum %02lus", h, m, s);
  else
    snprintf(buf, len, "%02lum %02lus", m, s);
}

// ─────────────────────────────────────────────
// Touch helper
// ─────────────────────────────────────────────

void getTouchXY(int& sx, int& sy) {
  TS_Point p = ts.getPoint();
  sx = map(p.x, TOUCH_MIN_X, TOUCH_MAX_X, 0, SCREEN_W);
  sy = map(p.y, TOUCH_MIN_Y, TOUCH_MAX_Y, 0, SCREEN_H);
  sx = constrain(sx, 0, SCREEN_W - 1);
  sy = constrain(sy, 0, SCREEN_H - 1);
}

// ─────────────────────────────────────────────
// Display — full static frame (drawn on boot or after toggle)
// ─────────────────────────────────────────────

void drawFrame(bool state) {
  tft.fillScreen(COL_BG);

  // Top bar
  tft.fillRect(0, 0, SCREEN_W, Y_TOPBAR_H, COL_TOPBAR);
  tft.drawFastHLine(0, Y_TOPBAR_H - 1, SCREEN_W, TFT_DARKGREY);

  // Separator line above elapsed
  tft.drawFastHLine(10, Y_ELAPSED_TOP - 4, SCREEN_W - 20, 0x2104);

  // State badge
  uint16_t badgeCol = state ? COL_ON_BADGE : COL_OFF_BADGE;
  tft.fillRoundRect(10, Y_BADGE_TOP, SCREEN_W - 20, Y_BADGE_H, 10, badgeCol);
  tft.drawRoundRect(10, Y_BADGE_TOP, SCREEN_W - 20, Y_BADGE_H, 10,
                    state ? TFT_GREEN : TFT_DARKGREY);
  tft.setTextSize(4);
  const char* label = state ? "OPEN" : "CLOSE";
  int tw = tft.textWidth(label);
  tft.setTextColor(TFT_WHITE, badgeCol);
  tft.setCursor((SCREEN_W - tw) / 2, Y_BADGE_TOP + 14);
  tft.print(label);

  // Elapsed label (static prefix)
  tft.setTextSize(1);
  tft.setTextColor(TFT_DARKGREY, COL_BG);
  tft.setCursor(10, Y_ELAPSED_TOP);
  tft.print("Since last toggle:");

  // IP address at bottom — links to web config page
  if (deviceIP.length() > 0) {
    String ipLabel = "Config: http://" + deviceIP + "/";
    tft.setTextSize(1);
    tft.setTextColor(0x3186, COL_BG);   // dim blue-grey
    int iw = tft.textWidth(ipLabel.c_str());
    tft.setCursor((SCREEN_W - iw) / 2, SCREEN_H - 14);
    tft.print(ipLabel);
  }
}

// ─────────────────────────────────────────────
// Display — live zones only (1 Hz, no full redraw)
// ─────────────────────────────────────────────

void invalidateLiveCache() {
  prevTime[0] = prevDate[0] = prevElapsed[0] = '\0';
}

void updateLiveZones() {
  // ── Clock ──────────────────────────────────
  char tStr[12];
  if (ntpSynced && isTimeValid())
    getTimeStr(tStr, sizeof(tStr));
  else
    snprintf(tStr, sizeof(tStr), "--:--:--");

  if (strcmp(tStr, prevTime) != 0) {
    strncpy(prevTime, tStr, sizeof(prevTime));
    tft.fillRect(6, 7, 125, 22, COL_TOPBAR);
    tft.setTextSize(2);
    tft.setTextColor(TFT_CYAN, COL_TOPBAR);
    tft.setCursor(6, 11);
    tft.print(tStr);
  }

  // ── Date ───────────────────────────────────
  char dStr[16];
  if (ntpSynced && isTimeValid())
    getDateStr(dStr, sizeof(dStr));
  else
    dStr[0] = '\0';

  if (strcmp(dStr, prevDate) != 0) {
    strncpy(prevDate, dStr, sizeof(prevDate));
    tft.fillRect(150, 7, SCREEN_W - 156, 22, COL_TOPBAR);
    if (dStr[0]) {
      tft.setTextSize(2);
      tft.setTextColor(TFT_LIGHTGREY, COL_TOPBAR);
      int dw = tft.textWidth(dStr);
      tft.setCursor(SCREEN_W - dw - 6, 11);
      tft.print(dStr);
    }
  }

  // ── Elapsed ────────────────────────────────
  char eStr[32];
  if (toggledAt == 0)
    snprintf(eStr, sizeof(eStr), "unknown");
  else
    formatElapsed((millis() - toggledAt) / 1000, eStr, sizeof(eStr));

  if (strcmp(eStr, prevElapsed) != 0) {
    strncpy(prevElapsed, eStr, sizeof(prevElapsed));
    tft.fillRect(118, Y_ELAPSED_TOP, SCREEN_W - 124, 10, COL_BG);
    tft.setTextSize(1);
    tft.setTextColor(COL_ELAPSED, COL_BG);
    tft.setCursor(118, Y_ELAPSED_TOP);
    tft.print(eStr);
  }
}

// ─────────────────────────────────────────────
// Splash / notification
// ─────────────────────────────────────────────

void drawSplash(const char* msg) {
  tft.fillScreen(TFT_BLACK);
  tft.setTextColor(TFT_CYAN, TFT_BLACK);
  tft.setTextSize(2);
  int tw = tft.textWidth(msg);
  tft.setCursor((SCREEN_W - tw) / 2, 108);
  tft.print(msg);
}

void showNotification(const char* msg, uint16_t col) {
  int ny = SCREEN_H - 22;
  tft.fillRoundRect(12, ny, SCREEN_W - 24, 18, 4, col);
  tft.setTextSize(1);
  tft.setTextColor(TFT_WHITE, col);
  int tw = tft.textWidth(msg);
  tft.setCursor((SCREEN_W - tw) / 2, ny + 5);
  tft.print(msg);
}

// ─────────────────────────────────────────────
// RGB LED (active LOW)
// ─────────────────────────────────────────────

void setLED(bool on) {
  digitalWrite(LED_RED,   HIGH);
  digitalWrite(LED_BLUE,  HIGH);
  digitalWrite(LED_GREEN, on ? LOW : HIGH);
}

// ─────────────────────────────────────────────
// Telegram helpers
// ─────────────────────────────────────────────

String fetchLastBotMessage() {
  prefs.begin("tgstate", /*readOnly=*/true);
  String saved = prefs.getString("lastMsg", "");
  prefs.end();
  Serial.printf("[TG] fetchLastBotMessage from NVS: \"%s\"\n", saved.c_str());
  return saved;
}

bool sendTelegram(const String& text) {
  Serial.printf("[TG] Sending: %s\n", text.c_str());
  if (bot->sendMessage(cfgChatId, text, "")) {
    lastSentText = text;
    prefs.begin("tgstate", /*readOnly=*/false);
    prefs.putString("lastMsg", text);
    prefs.putUInt("toggleTime", (uint32_t)time(nullptr));
    prefs.end();
    Serial.println("[TG] OK (state persisted to NVS)");

    // int msgId = bot->last_sent_message_id;
    // if (msgId > 0) {
    //   String pinUrl = String("https://api.telegram.org/bot") + cfgBotToken +
    //                   "/pinChatMessage?chat_id=" + cfgChatId +
    //                   "&message_id=" + msgId +
    //                   "&disable_notification=true";
    //   HTTPClient https;
    //   secureClient.setInsecure();
    //   https.begin(secureClient, pinUrl);
    //   int code = https.GET();
    //   Serial.printf("[TG] Pin message_id=%d: HTTP %d\n", msgId, code);
    //   https.end();
    // } else {
    //   Serial.println("[TG] Pin skipped: message_id not available");
    // }

    return true;
  }
  Serial.printf("[TG] FAILED (WiFi status=%d, last_err=%d)\n",
                (int)WiFi.status(), (int)secureClient.lastError(nullptr, 0));
  return false;
}

String stateToMsg(bool s) { return s ? String(cfgMsgOn) : String(cfgMsgOff); }

// ─────────────────────────────────────────────
// Cloudflare Queue — HTTP publish
// POST {"messages":[{"body":{"status":"open","ts":1234},"content_type":"application/json"}]}
// to the configured queue endpoint with Bearer auth.
// ─────────────────────────────────────────────

bool sendToCloudflareQueue(bool state) {
  if (strlen(cfgCfQueueUrl) == 0) return false;

  StaticJsonDocument<320> doc;
  JsonArray messages = doc.createNestedArray("messages");
  JsonObject msg     = messages.createNestedObject();
  JsonObject body    = msg.createNestedObject("body");
  body["service"] = cfgCfSvcName;
  body["status"]  = state ? "open" : "closed";
  body["ts"]      = (uint32_t)time(nullptr);
  msg["content_type"] = "application/json";

  String payload;
  serializeJson(doc, payload);

  WiFiClientSecure cfClient;
  cfClient.setInsecure();
  HTTPClient https;
  https.begin(cfClient, cfgCfQueueUrl);
  https.addHeader("Content-Type",  "application/json");
  https.addHeader("Authorization", String("Bearer ") + cfgCfApiToken);
  int code = https.POST(payload);
  https.end();

  Serial.printf("[CF] Queue POST status=%s ts=%u → HTTP %d\n",
                state ? "open" : "closed", (uint32_t)time(nullptr), code);
  return code >= 200 && code < 300;
}

// ─────────────────────────────────────────────
// Access Point setup mode
// ─────────────────────────────────────────────

#define AP_SSID "PresenceSetup"

void drawApSplash(const char* apSsid, const char* apPass, const char* apIp) {
  tft.fillScreen(TFT_BLACK);

  // Title bar
  tft.fillRect(0, 0, SCREEN_W, 30, 0x0014);
  tft.setTextSize(2);
  tft.setTextColor(TFT_CYAN, 0x0014);
  const char* title = "WiFi Setup Mode";
  tft.setCursor((SCREEN_W - tft.textWidth(title)) / 2, 8);
  tft.print(title);

  // Instructions
  tft.setTextSize(1);
  tft.setTextColor(TFT_LIGHTGREY, TFT_BLACK);
  const char* instr = "Connect to this network, then open:";
  tft.setCursor((SCREEN_W - tft.textWidth(instr)) / 2, 42);
  tft.print(instr);

  // SSID row
  tft.fillRoundRect(6, 58, SCREEN_W - 12, 34, 5, 0x1082);
  tft.setTextSize(1);
  tft.setTextColor(TFT_DARKGREY, 0x1082);
  tft.setCursor(14, 63);
  tft.print("Network (SSID)");
  tft.setTextSize(2);
  tft.setTextColor(TFT_WHITE, 0x1082);
  tft.setCursor(14, 74);
  tft.print(apSsid);

  // Password row
  tft.fillRoundRect(6, 100, SCREEN_W - 12, 34, 5, 0x1082);
  tft.setTextSize(1);
  tft.setTextColor(TFT_DARKGREY, 0x1082);
  tft.setCursor(14, 105);
  tft.print("Password");
  tft.setTextSize(2);
  tft.setTextColor(TFT_YELLOW, 0x1082);
  tft.setCursor(14, 116);
  tft.print(apPass);

  // URL row
  tft.fillRoundRect(6, 142, SCREEN_W - 12, 34, 5, 0x1082);
  tft.setTextSize(1);
  tft.setTextColor(TFT_DARKGREY, 0x1082);
  tft.setCursor(14, 147);
  tft.print("Config URL");
  String url = String("http://") + apIp + "/";
  tft.setTextSize(2);
  tft.setTextColor(0x07FF, 0x1082);  // cyan
  tft.setCursor(14, 158);
  tft.print(url);

  // Footer hint
  tft.setTextSize(1);
  tft.setTextColor(0x4208, TFT_BLACK);
  const char* hint = "Device will restart after saving";
  tft.setCursor((SCREEN_W - tft.textWidth(hint)) / 2, SCREEN_H - 12);
  tft.print(hint);
}

void startAPMode() {
  Serial.printf("[AP] Starting AP: SSID=%s  Pass=%s\n", AP_SSID, cfgApPass);
  WiFi.disconnect(true);
  delay(100);
  WiFi.mode(WIFI_AP);
  WiFi.softAP(AP_SSID, cfgApPass);
  delay(200);

  String apIp = WiFi.softAPIP().toString();
  Serial.printf("[AP] IP: %s\n", apIp.c_str());

  // Web server still uses the same handlers; update deviceIP for any frame rendering
  deviceIP = apIp;
  webServer.on("/",     HTTP_GET,  handleConfigRoot);
  webServer.on("/save", HTTP_POST, handleConfigSave);
  webServer.begin();
  Serial.println("[AP] Web server started");

  drawApSplash(AP_SSID, cfgApPass, apIp.c_str());

  // Blink blue LED while in AP mode
  pinMode(LED_BLUE, OUTPUT);
  bool ledState = false;
  while (true) {
    webServer.handleClient();
    static unsigned long lastBlink = 0;
    if (millis() - lastBlink >= 800) {
      lastBlink = millis();
      ledState = !ledState;
      digitalWrite(LED_BLUE, ledState ? LOW : HIGH);
    }
  }
}

// ─────────────────────────────────────────────
// setup()
// ─────────────────────────────────────────────

void setup() {
  Serial.begin(115200);

  pinMode(LED_RED,   OUTPUT); digitalWrite(LED_RED,   HIGH);
  pinMode(LED_GREEN, OUTPUT); digitalWrite(LED_GREEN, HIGH);
  pinMode(LED_BLUE,  OUTPUT); digitalWrite(LED_BLUE,  HIGH);

  // Load config early so cfgBtnPin is set before pinMode
  loadConfig();
  bot = nullptr;  // will be (re-)created after WiFi/NVS

  pinMode(cfgBtnPin, INPUT_PULLUP);

  tft.init();
  tft.setRotation(1);

  // Re-attach PWM after tft.init(), which calls digitalWrite(TFT_BL, HIGH) and detaches PWM
  ledcSetup(BL_PWM_CHANNEL, BL_PWM_FREQ, BL_PWM_RES);
  ledcAttachPin(TFT_BL_PIN, BL_PWM_CHANNEL);
  ledcWrite(BL_PWM_CHANNEL, cfgBlFull);
  Serial.printf("[DIM] Backlight init: full brightness. Dim timeout: %lu ms\n", cfgBlDimMs);
  touchSPI.begin(XPT2046_CLK, XPT2046_MISO, XPT2046_MOSI, XPT2046_CS);
  ts.begin(touchSPI);
  ts.setRotation(1);

  // If no WiFi SSID has ever been configured, go straight to AP setup mode.
  if (strlen(cfgWifiSsid) == 0) {
    Serial.println("[WiFi] No SSID configured — starting AP setup mode");
    startAPMode();  // never returns; device restarts after config save
  }

  drawSplash("Connecting to WiFi...");
  WiFi.begin(cfgWifiSsid, cfgWifiPass);
  int att = 0;
  while (WiFi.status() != WL_CONNECTED && att++ < 40) delay(500);

  if (WiFi.status() != WL_CONNECTED) {
    Serial.println("[WiFi] Connection failed — starting AP setup mode");
    startAPMode();  // never returns; device restarts after config save
  }

  deviceIP = WiFi.localIP().toString();
  Serial.printf("[WiFi] %s\n", deviceIP.c_str());

  // Show IP on splash so the user knows where to find the config page
  {
    tft.fillScreen(TFT_BLACK);
    tft.setTextColor(TFT_CYAN, TFT_BLACK);
    tft.setTextSize(2);
    String ipLine = "IP: " + WiFi.localIP().toString();
    int tw = tft.textWidth(ipLine.c_str());
    tft.setCursor((SCREEN_W - tw) / 2, 95);
    tft.print(ipLine);
    tft.setTextSize(1);
    tft.setTextColor(TFT_DARKGREY, TFT_BLACK);
    const char* hint = "Open in browser to configure";
    tw = tft.textWidth(hint);
    tft.setCursor((SCREEN_W - tw) / 2, 122);
    tft.print(hint);
    delay(2500);
  }

  // Load all config from NVS (or defaults)
  loadConfig();
  bot = new UniversalTelegramBot(cfgBotToken, secureClient);

  secureClient.setInsecure();

  // Start web config server
  setupWebServer();
  setupOTA();

  // NTP sync
  drawSplash("Syncing time (NTP)...");
  configTime(NTP_UTC_OFFSET, NTP_DST_OFFSET, NTP_SERVER);
  unsigned long t0 = millis();
  while (!isTimeValid() && millis() - t0 < 5000) delay(200);
  ntpSynced = isTimeValid();
  Serial.printf("[NTP] %s\n", ntpSynced ? "OK" : "failed");

  // Telegram state restore
  drawSplash("Fetching last state...");
  lastSentText = fetchLastBotMessage();
  Serial.printf("[Init] Last TG msg: \"%s\"\n", lastSentText.c_str());

  toggleState = (lastSentText == stateToMsg(true));

  // Restore elapsed time from NVS wall-clock timestamp
  prefs.begin("tgstate", /*readOnly=*/true);
  uint32_t savedToggleTime = prefs.getUInt("toggleTime", 0);
  prefs.end();
  if (ntpSynced && savedToggleTime > 0 && (uint32_t)time(nullptr) >= savedToggleTime) {
    uint32_t elapsedSec = (uint32_t)time(nullptr) - savedToggleTime;
    toggledAt = (millis() > elapsedSec * 1000UL) ? millis() - elapsedSec * 1000UL : 1;
    Serial.printf("[Init] Restored toggledAt from NVS: %u s ago\n", elapsedSec);
  } else {
    toggledAt = 0;  // unknown
    Serial.println("[Init] toggledAt unknown (no NVS entry or NTP not synced)");
  }

  String curMsg = stateToMsg(toggleState);
  if (lastSentText != curMsg) {
    Serial.println("[Init] Out of sync, updating Telegram...");
    sendTelegram(curMsg);
  }

  setLED(toggleState);
  drawFrame(toggleState);
  invalidateLiveCache();
  updateLiveZones();
  lastActivityAt = millis();
}

// ─────────────────────────────────────────────
// loop()
// ─────────────────────────────────────────────

uint32_t get_value()
{
  return cfgBlDimMs;
}

void loop() {
  ArduinoOTA.handle();
  // Handle incoming HTTP config requests
  webServer.handleClient();

  if (otaInProgress) return;

  // 1-second clock/elapsed tick
  static unsigned long lastTick = 0;
  if (millis() - lastTick >= 1000) {
    lastTick = millis();
    updateLiveZones();
  }

  // Periodic Cloudflare Queue publish
  static unsigned long lastCfSend = 0;
  unsigned long cfIntervalMs = (unsigned long)cfgCfQueueSec * 1000UL;
  if (strlen(cfgCfQueueUrl) > 0 && millis() - lastCfSend >= cfIntervalMs) {
    lastCfSend = millis();
    sendToCloudflareQueue(toggleState);
  }

  // Backlight dimming after inactivity
  static bool dimmed = false;
  if (!dimmed && millis() - lastActivityAt >= get_value()) {
    Serial.printf("[DIM] Dimming backlight after %lu ms of inactivity\n", millis() - lastActivityAt);
    ledcWrite(BL_PWM_CHANNEL, cfgBlDim);
    dimmed = true;
  }

  // Wake screen on touch
  if (dimmed && ts.touched()) {
    Serial.println("[DIM] Waking backlight (touch)");
    ledcWrite(BL_PWM_CHANNEL, cfgBlFull);
    dimmed = false;
    lastActivityAt = millis();
  }

  // Physical latching switch on cfgBtnPin: LOW = OPEN, HIGH = CLOSED
  static bool lastPinVal = HIGH;
  static unsigned long lastChange = 0;

  bool pinVal = digitalRead(cfgBtnPin);

  // Debounce: only act after the pin has been stable for 50 ms
  if (pinVal != lastPinVal) {
    lastChange  = millis();
    lastPinVal  = pinVal;
  }
  if (millis() - lastChange < 50) return;

  bool newState = (pinVal == LOW);   // LOW → OPEN, HIGH → CLOSED
  if (newState == toggleState) return;   // no change, nothing to do

  // Wake screen if dimmed; the switch position change wakes it
  if (dimmed) {
    Serial.println("[DIM] Waking backlight (button activity)");
    ledcWrite(BL_PWM_CHANNEL, cfgBlFull);
    dimmed = false;
    lastActivityAt = millis();
  }
  lastActivityAt = millis();

  Serial.printf("[BTN] pin=%s → state=%s\n", pinVal == LOW ? "LOW" : "HIGH",
                newState ? "OPEN" : "CLOSED");

  toggleState = newState;
  toggledAt   = millis();

  setLED(toggleState);
  drawFrame(toggleState);
  invalidateLiveCache();
  updateLiveZones();

  String newMsg = stateToMsg(toggleState);
  if (newMsg != lastSentText) {
    showNotification("Sending to Telegram...", TFT_NAVY);
    bool ok = sendTelegram(newMsg);
    delay(600);
    showNotification(ok ? "Sent!" : "Send failed!", ok ? 0x0340 : TFT_MAROON);
    delay(900);
    drawFrame(toggleState);
    invalidateLiveCache();
    updateLiveZones();
  }

  // Publish new state to Cloudflare Queue immediately on toggle
  if (strlen(cfgCfQueueUrl) > 0) {
    sendToCloudflareQueue(toggleState);
    lastCfSend = millis();  // reset periodic timer so we don't double-send
  }
}

/*
 * ─────────────────────────────────────────────────────────────────────
 * TFT_eSPI  User_Setup.h  for ESP32-2432S028R (CYD)
 * ─────────────────────────────────────────────────────────────────────
 * Replace ALL content in  <Arduino libraries>/TFT_eSPI/User_Setup.h
 *
 * #define ILI9341_DRIVER
 * #define TFT_BL             21
 * #define TFT_BACKLIGHT_ON   HIGH
 * #define TFT_MISO           12
 * #define TFT_MOSI           13
 * #define TFT_SCLK           14
 * #define TFT_CS             15
 * #define TFT_DC              2
 * #define TFT_RST            -1
 * #define LOAD_GLCD
 * #define LOAD_FONT2
 * #define LOAD_FONT4
 * #define LOAD_FONT6
 * #define LOAD_FONT7
 * #define LOAD_FONT8
 * #define LOAD_GFXFF
 * #define SMOOTH_FONT
 * #define SPI_FREQUENCY       55000000
 * #define SPI_READ_FREQUENCY  20000000
 * #define SPI_TOUCH_FREQUENCY  2500000
 * ─────────────────────────────────────────────────────────────────────
 */
