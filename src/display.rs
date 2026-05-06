use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_6X10},
        MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{
        Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle,
        CornerRadii,
    },
    text::{Baseline, Text},
};
use profont::PROFONT_24_POINT;

pub const SCREEN_W: i32 = 320;
pub const SCREEN_H: i32 = 240;

const Y_TOPBAR_H: i32 = 38;
const Y_BADGE_TOP: i32 = 44;
const Y_BADGE_H: i32 = 60;
const Y_ELAPSED_TOP: i32 = 112;

// Colours as RGB565 raw values, converted to embedded-graphics Rgb565
fn c(raw: u16) -> Rgb565 {
    use embedded_graphics::pixelcolor::raw::RawU16;
    Rgb565::from(RawU16::new(raw))
}

const COL_BG: u16 = 0x0000;       // TFT_BLACK
const COL_TOPBAR: u16 = 0x0014;   // very dark navy
const COL_ON_BADGE: u16 = 0x0460; // dark green-teal
const COL_OFF_BADGE: u16 = 0x2104;// dark charcoal
const COL_ELAPSED: u16 = 0x8C71;  // medium grey
const COL_WHITE: u16 = 0xFFFF;
const COL_CYAN: u16 = 0x07FF;
const COL_GREEN: u16 = 0x07E0;
const COL_DARKGREY: u16 = 0x7BEF;
const COL_LIGHTGREY: u16 = 0xC618;
const COL_YELLOW: u16 = 0xFFE0;
const COL_DIM_BLUE: u16 = 0x3186;

pub trait DrawTarget565: DrawTarget<Color = Rgb565, Error: core::fmt::Debug> {}
impl<T: DrawTarget<Color = Rgb565, Error: core::fmt::Debug>> DrawTarget565 for T {}

fn fill_rect<D: DrawTarget<Color = Rgb565>>(
    d: &mut D, x: i32, y: i32, w: i32, h: i32, col: Rgb565,
) {
    let _ = Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32))
        .into_styled(PrimitiveStyle::with_fill(col))
        .draw(d);
}

fn fill_rounded_rect<D: DrawTarget<Color = Rgb565>>(
    d: &mut D, x: i32, y: i32, w: i32, h: i32, radius: u32, fill: Rgb565,
) {
    let _ = RoundedRectangle::new(
        Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)),
        CornerRadii::new(Size::new(radius, radius)),
    )
    .into_styled(PrimitiveStyle::with_fill(fill))
    .draw(d);
}

fn stroke_rounded_rect<D: DrawTarget<Color = Rgb565>>(
    d: &mut D, x: i32, y: i32, w: i32, h: i32, radius: u32, stroke: Rgb565,
) {
    let _ = RoundedRectangle::new(
        Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)),
        CornerRadii::new(Size::new(radius, radius)),
    )
    .into_styled(PrimitiveStyle::with_stroke(stroke, 1))
    .draw(d);
}

fn hline<D: DrawTarget<Color = Rgb565>>(d: &mut D, x: i32, y: i32, w: i32, col: Rgb565) {
    let _ = Line::new(Point::new(x, y), Point::new(x + w - 1, y))
        .into_styled(PrimitiveStyle::with_stroke(col, 1))
        .draw(d);
}

fn text_sm<D: DrawTarget<Color = Rgb565>>(d: &mut D, s: &str, x: i32, y: i32, fg: u16, bg: u16) {
    let style = MonoTextStyle::new(&FONT_6X10, c(fg));
    // No per-character background fill with MonoTextStyle — fill rect before drawing
    fill_rect(d, x, y, (s.len() * FONT_6X10.character_size.width) as i32, FONT_6X10.character_size.height as i32, c(bg));
    let _ = Text::with_baseline(s, Point::new(x, y), style, Baseline::Top).draw(d);
}

fn text_md<D: DrawTarget<Color = Rgb565>>(d: &mut D, s: &str, x: i32, y: i32, fg: u16, bg: u16) {
    fill_rect(d, x, y, (s.len() * FONT_10X20.character_size.width) as i32, FONT_10X20.character_size.height as i32, c(bg));
    let style = MonoTextStyle::new(&FONT_10X20, c(fg));
    let _ = Text::with_baseline(s, Point::new(x, y), style, Baseline::Top).draw(d);
}

fn text_lg<D: DrawTarget<Color = Rgb565>>(d: &mut D, s: &str, x: i32, y: i32, fg: u16) {
    let style = MonoTextStyle::new(&PROFONT_24_POINT, c(fg));
    let _ = Text::with_baseline(s, Point::new(x, y), style, Baseline::Top).draw(d);
}

fn text_lg_width(s: &str) -> i32 {
    (s.len() * PROFONT_24_POINT.character_size.width) as i32
}

fn text_md_width(s: &str) -> i32 {
    (s.len() * FONT_10X20.character_size.width) as i32
}

fn text_sm_width(s: &str) -> i32 {
    (s.len() * FONT_6X10.character_size.width) as i32
}

// Full static frame — call on boot or after state change
pub fn draw_frame<D: DrawTarget<Color = Rgb565>>(d: &mut D, state: bool, device_ip: &str) {
    // Clear screen
    fill_rect(d, 0, 0, SCREEN_W, SCREEN_H, c(COL_BG));

    // Top bar
    fill_rect(d, 0, 0, SCREEN_W, Y_TOPBAR_H, c(COL_TOPBAR));
    hline(d, 0, Y_TOPBAR_H - 1, SCREEN_W, c(COL_DARKGREY));

    // Separator above elapsed
    hline(d, 10, Y_ELAPSED_TOP - 4, SCREEN_W - 20, c(COL_OFF_BADGE));

    // State badge
    let badge_col = if state { COL_ON_BADGE } else { COL_OFF_BADGE };
    let border_col = if state { COL_GREEN } else { COL_DARKGREY };
    fill_rounded_rect(d, 10, Y_BADGE_TOP, SCREEN_W - 20, Y_BADGE_H, 10, c(badge_col));
    stroke_rounded_rect(d, 10, Y_BADGE_TOP, SCREEN_W - 20, Y_BADGE_H, 10, c(border_col));

    let label = if state { "OPEN" } else { "CLOSE" };
    let tw = text_lg_width(label);
    let tx = (SCREEN_W - tw) / 2;
    // Badge Y center: Y_BADGE_TOP + (Y_BADGE_H - 24pt_height) / 2
    let ty = Y_BADGE_TOP + (Y_BADGE_H - PROFONT_24_POINT.character_size.height as i32) / 2;
    text_lg(d, label, tx, ty, COL_WHITE);

    // Elapsed label (static prefix)
    text_sm(d, "Since last toggle:", 10, Y_ELAPSED_TOP, COL_DARKGREY, COL_BG);

    // IP address at bottom
    if !device_ip.is_empty() {
        let ip_label = format!("Config: http://{}/", device_ip);
        let iw = text_sm_width(&ip_label);
        let ix = (SCREEN_W - iw) / 2;
        text_sm(d, &ip_label, ix, SCREEN_H - 14, COL_DIM_BLUE, COL_BG);
    }
}

// Cached strings for partial redraws
#[derive(Default)]
pub struct LiveCache {
    prev_time:    String,
    prev_date:    String,
    prev_elapsed: String,
}

impl LiveCache {
    pub fn invalidate(&mut self) {
        self.prev_time.clear();
        self.prev_date.clear();
        self.prev_elapsed.clear();
    }
}

// Partial live-zone update (clock, date, elapsed) — call every second
pub fn update_live_zones<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    cache: &mut LiveCache,
    ntp_synced: bool,
    toggled_at_ms: Option<u64>, // millis since epoch of last toggle
    now_ms: u64,                // current millis
) {
    // ── Clock ──────────────────────────────────
    let time_str = if ntp_synced {
        format_time_from_ms(now_ms)
    } else {
        "--:--:--".to_string()
    };

    if time_str != cache.prev_time {
        cache.prev_time = time_str.clone();
        fill_rect(d, 6, 7, 125, 22, c(COL_TOPBAR));
        text_md(d, &time_str, 6, 9, COL_CYAN, COL_TOPBAR);
    }

    // ── Date ───────────────────────────────────
    let date_str = if ntp_synced {
        format_date_from_ms(now_ms)
    } else {
        String::new()
    };

    if date_str != cache.prev_date {
        cache.prev_date = date_str.clone();
        fill_rect(d, 150, 7, SCREEN_W - 156, 22, c(COL_TOPBAR));
        if !date_str.is_empty() {
            let dw = text_md_width(&date_str);
            text_md(d, &date_str, SCREEN_W - dw - 6, 9, COL_LIGHTGREY, COL_TOPBAR);
        }
    }

    // ── Elapsed ────────────────────────────────
    let elapsed_str = match toggled_at_ms {
        None => "unknown".to_string(),
        Some(at) => {
            let secs = now_ms.saturating_sub(at) / 1000;
            format_elapsed(secs)
        }
    };

    if elapsed_str != cache.prev_elapsed {
        cache.prev_elapsed = elapsed_str.clone();
        // Clear the elapsed value area (after "Since last toggle:" label)
        let label_w = text_sm_width("Since last toggle:") as i32;
        fill_rect(d, 10 + label_w, Y_ELAPSED_TOP, SCREEN_W - 10 - label_w - 4, 10, c(COL_BG));
        text_sm(d, &elapsed_str, 10 + label_w + 2, Y_ELAPSED_TOP, COL_ELAPSED, COL_BG);
    }
}

pub fn draw_splash<D: DrawTarget<Color = Rgb565>>(d: &mut D, msg: &str) {
    fill_rect(d, 0, 0, SCREEN_W, SCREEN_H, c(0x0000));
    let tw = text_md_width(msg);
    let tx = (SCREEN_W - tw) / 2;
    text_md(d, msg, tx, 108, COL_CYAN, 0x0000);
}

pub fn show_notification<D: DrawTarget<Color = Rgb565>>(d: &mut D, msg: &str, col: u16) {
    let ny = SCREEN_H - 22;
    fill_rounded_rect(d, 12, ny, SCREEN_W - 24, 18, 4, c(col));
    let tw = text_sm_width(msg);
    let tx = (SCREEN_W - tw) / 2;
    text_sm(d, msg, tx, ny + 5, COL_WHITE, col);
}

pub fn draw_ap_splash<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    ap_ssid: &str,
    ap_pass: &str,
    ap_ip: &str,
) {
    fill_rect(d, 0, 0, SCREEN_W, SCREEN_H, c(0x0000));

    // Title bar
    fill_rect(d, 0, 0, SCREEN_W, 30, c(0x0014));
    let title = "WiFi Setup Mode";
    let tw = text_md_width(title);
    text_md(d, title, (SCREEN_W - tw) / 2, 5, COL_CYAN, 0x0014);

    // Instruction
    let instr = "Connect to network, then open:";
    let iw = text_sm_width(instr);
    text_sm(d, instr, (SCREEN_W - iw) / 2, 38, COL_LIGHTGREY, 0x0000);

    // SSID row
    fill_rounded_rect(d, 6, 54, SCREEN_W - 12, 34, 5, c(0x1082));
    text_sm(d, "Network (SSID)", 14, 59, COL_DARKGREY, 0x1082);
    text_md(d, ap_ssid, 14, 68, COL_WHITE, 0x1082);

    // Password row
    fill_rounded_rect(d, 6, 96, SCREEN_W - 12, 34, 5, c(0x1082));
    text_sm(d, "Password", 14, 101, COL_DARKGREY, 0x1082);
    text_md(d, ap_pass, 14, 110, COL_YELLOW, 0x1082);

    // URL row
    fill_rounded_rect(d, 6, 138, SCREEN_W - 12, 34, 5, c(0x1082));
    text_sm(d, "Config URL", 14, 143, COL_DARKGREY, 0x1082);
    let url = format!("http://{}/", ap_ip);
    text_md(d, &url, 14, 152, COL_CYAN, 0x1082);

    // Footer
    let hint = "Device restarts after saving";
    let hw = text_sm_width(hint);
    text_sm(d, hint, (SCREEN_W - hw) / 2, SCREEN_H - 12, 0x4208, 0x0000);
}

// ── Time formatting helpers ──────────────────────────────────────

fn format_elapsed(total_secs: u64) -> String {
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    let d = total_secs / 86400;

    if d > 0 {
        format!("{}d {:02}h {:02}m {:02}s", d, h, m, s)
    } else if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else {
        format!("{:02}m {:02}s", m, s)
    }
}

fn format_time_from_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn format_date_from_ms(ms: u64) -> String {
    // Compute calendar date from Unix timestamp (seconds)
    let unix_secs = ms / 1000;
    let (day, month, year) = unix_to_date(unix_secs);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{:02} {} {:04}", day, MONTHS[(month - 1) as usize], year)
}

fn unix_to_date(secs: u64) -> (u32, u32, u32) {
    // Proleptic Gregorian calendar
    let days = secs / 86400;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (d as u32, m as u32, y as u32)
}
