// Pure-Rust logic: no ESP-IDF dependencies.
// Compiled for both the embedded target (as part of the library crate) and
// the host (x86_64) for `cargo test --target x86_64-unknown-linux-gnu --lib`.

// ── Elapsed / clock / date formatting ──────────────────────────

pub fn format_elapsed(total_secs: u64) -> String {
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

pub fn format_time_from_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn format_date_from_ms(ms: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (day, month, year) = unix_to_date(ms / 1000);
    format!("{:02} {} {:04}", day, MONTHS[(month - 1) as usize], year)
}

/// Proleptic Gregorian calendar. Returns (day, month, year), all 1-based.
pub fn unix_to_date(secs: u64) -> (u32, u32, u32) {
    let days = secs / 86400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (d as u32, m as u32, y as u32)
}

// ── URL / form decoding ─────────────────────────────────────────

/// Decode `application/x-www-form-urlencoded` bytes into key-value pairs.
/// Malformed percent-sequences are passed through unchanged.
/// Multi-byte UTF-8 sequences (e.g. emoji) are decoded correctly.
pub fn parse_form_urlencoded(body: &[u8]) -> Vec<(String, String)> {
    let s = String::from_utf8_lossy(body);
    s.split('&')
        .filter_map(|part| {
            let mut it = part.splitn(2, '=');
            let key = url_decode(it.next()?);
            if key.is_empty() {
                return None;
            }
            let val = url_decode(it.next().unwrap_or(""));
            Some((key, val))
        })
        .collect()
}

pub fn url_decode(s: &str) -> String {
    // Collect raw bytes first, then interpret as UTF-8 so multi-byte
    // sequences (emoji, accented chars) round-trip correctly.
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let src = s.as_bytes();
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < src.len() => {
                if let (Some(h), Some(l)) = (hex_val(src[i + 1]), hex_val(src[i + 2])) {
                    bytes.push(h << 4 | l);
                    i += 3;
                } else {
                    bytes.push(b'%');
                    i += 1;
                }
            }
            b => {
                bytes.push(b);
                i += 1;
            }
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

// ── Telegram JSON helpers ───────────────────────────────────────

pub fn build_telegram_json(chat_id: &str, text: &str) -> String {
    format!(r#"{{"chat_id":"{}","text":"{}"}}"#, chat_id, json_escape(text))
}

pub fn json_escape(s: &str) -> String {
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

// ── Tests (run on host with: cargo test --target x86_64-unknown-linux-gnu --lib) ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_elapsed ─────────────────────────────────────────

    #[test]
    fn elapsed_zero() {
        assert_eq!(format_elapsed(0), "00m 00s");
    }

    #[test]
    fn elapsed_seconds_only() {
        assert_eq!(format_elapsed(45), "00m 45s");
    }

    #[test]
    fn elapsed_one_minute_thirty() {
        assert_eq!(format_elapsed(90), "01m 30s");
    }

    #[test]
    fn elapsed_exact_hour() {
        assert_eq!(format_elapsed(3600), "1h 00m 00s");
    }

    #[test]
    fn elapsed_hours_minutes_seconds() {
        assert_eq!(format_elapsed(3661), "1h 01m 01s");
    }

    #[test]
    fn elapsed_exact_day() {
        assert_eq!(format_elapsed(86400), "1d 00h 00m 00s");
    }

    #[test]
    fn elapsed_days_hours_minutes_seconds() {
        assert_eq!(format_elapsed(90_061), "1d 01h 01m 01s");
    }

    #[test]
    fn elapsed_multiple_days() {
        // 2 days + 3h + 4m + 5s
        assert_eq!(format_elapsed(2 * 86400 + 3 * 3600 + 4 * 60 + 5), "2d 03h 04m 05s");
    }

    // ── format_time_from_ms ────────────────────────────────────

    #[test]
    fn time_midnight() {
        assert_eq!(format_time_from_ms(0), "00:00:00");
    }

    #[test]
    fn time_one_second() {
        assert_eq!(format_time_from_ms(1_000), "00:00:01");
    }

    #[test]
    fn time_one_hour_one_minute_one_second() {
        assert_eq!(format_time_from_ms(3_661_000), "01:01:01");
    }

    #[test]
    fn time_wraps_at_24h() {
        // 25 hours should wrap: 25 % 24 = 1
        assert_eq!(format_time_from_ms(25 * 3_600_000), "01:00:00");
    }

    // ── unix_to_date ───────────────────────────────────────────

    #[test]
    fn date_epoch() {
        assert_eq!(unix_to_date(0), (1, 1, 1970));
    }

    #[test]
    fn date_one_day_after_epoch() {
        assert_eq!(unix_to_date(86_400), (2, 1, 1970));
    }

    #[test]
    fn date_last_day_of_jan_1970() {
        assert_eq!(unix_to_date(30 * 86_400), (31, 1, 1970));
    }

    #[test]
    fn date_first_feb_1970() {
        assert_eq!(unix_to_date(31 * 86_400), (1, 2, 1970));
    }

    #[test]
    fn date_leap_day_2000() {
        // 2000-02-29 exists (year 2000 is a leap year)
        // Days from epoch to 2000-02-29:
        //   30 years, various leap years.  Known good: 2000-02-29 = unix 951_782_400
        assert_eq!(unix_to_date(951_782_400), (29, 2, 2000));
    }

    #[test]
    fn date_known_timestamp() {
        // 2023-11-14 22:13:20 UTC = 1_700_000_000
        assert_eq!(unix_to_date(1_700_000_000), (14, 11, 2023));
    }

    #[test]
    fn date_new_year_2025() {
        // 2025-01-01 00:00:00 UTC = 1_735_689_600
        assert_eq!(unix_to_date(1_735_689_600), (1, 1, 2025));
    }

    // ── format_date_from_ms ────────────────────────────────────

    #[test]
    fn date_str_epoch() {
        assert_eq!(format_date_from_ms(0), "01 Jan 1970");
    }

    #[test]
    fn date_str_known() {
        assert_eq!(format_date_from_ms(1_700_000_000 * 1_000), "14 Nov 2023");
    }

    // ── url_decode ─────────────────────────────────────────────

    #[test]
    fn url_decode_plain() {
        assert_eq!(url_decode("hello"), "hello");
    }

    #[test]
    fn url_decode_plus_as_space() {
        assert_eq!(url_decode("hello+world"), "hello world");
    }

    #[test]
    fn url_decode_percent_space() {
        assert_eq!(url_decode("hello%20world"), "hello world");
    }

    #[test]
    fn url_decode_exclamation() {
        assert_eq!(url_decode("secret%21"), "secret!");
    }

    #[test]
    fn url_decode_mixed() {
        assert_eq!(url_decode("My+Network%21"), "My Network!");
    }

    #[test]
    fn url_decode_utf8_emoji() {
        // 🟢 = U+1F7E2 = UTF-8 F0 9F 9F A2
        assert_eq!(url_decode("%F0%9F%9F%A2"), "🟢");
    }

    #[test]
    fn url_decode_bad_percent_passthrough() {
        // %ZZ is not valid hex — percent sign passes through unchanged
        assert_eq!(url_decode("%ZZfoo"), "%ZZfoo");
    }

    #[test]
    fn url_decode_truncated_percent() {
        // % at end of string — passes through
        assert_eq!(url_decode("foo%"), "foo%");
    }

    // ── parse_form_urlencoded ──────────────────────────────────

    #[test]
    fn form_single_pair() {
        let pairs = parse_form_urlencoded(b"key=value");
        assert_eq!(pairs, vec![("key".into(), "value".into())]);
    }

    #[test]
    fn form_two_pairs() {
        let pairs = parse_form_urlencoded(b"a=1&b=2");
        assert_eq!(
            pairs,
            vec![("a".into(), "1".into()), ("b".into(), "2".into())]
        );
    }

    #[test]
    fn form_decoded_values() {
        let pairs = parse_form_urlencoded(b"ssid=My+Network&pass=secret%21");
        assert_eq!(
            pairs,
            vec![
                ("ssid".into(), "My Network".into()),
                ("pass".into(), "secret!".into()),
            ]
        );
    }

    #[test]
    fn form_empty_value() {
        let pairs = parse_form_urlencoded(b"key=");
        assert_eq!(pairs, vec![("key".into(), "".into())]);
    }

    #[test]
    fn form_missing_equals_skipped() {
        // A part with no '=' has an empty key and is filtered out
        let pairs = parse_form_urlencoded(b"noequals&b=2");
        // "noequals" decodes to key="noequals", val="" — actually kept (non-empty key)
        assert_eq!(
            pairs,
            vec![
                ("noequals".into(), "".into()),
                ("b".into(), "2".into()),
            ]
        );
    }

    #[test]
    fn form_emoji_in_value() {
        // Emoji UTF-8 percent-encoded in the value
        let pairs = parse_form_urlencoded(b"msg=%F0%9F%9F%A2+open");
        assert_eq!(pairs, vec![("msg".into(), "🟢 open".into())]);
    }

    #[test]
    fn form_empty_body() {
        assert!(parse_form_urlencoded(b"").is_empty());
    }

    // ── json_escape ────────────────────────────────────────────

    #[test]
    fn escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn escape_double_quote() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(json_escape(r"back\slash"), r"back\\slash");
    }

    #[test]
    fn escape_newline() {
        assert_eq!(json_escape("line\nbreak"), r"line\nbreak");
    }

    #[test]
    fn escape_tab() {
        assert_eq!(json_escape("col\there"), r"col\there");
    }

    #[test]
    fn escape_carriage_return() {
        assert_eq!(json_escape("a\rb"), r"a\rb");
    }

    #[test]
    fn escape_emoji_unchanged() {
        // Emoji are valid UTF-8 and need no escaping in JSON
        assert_eq!(json_escape("open 🟢"), "open 🟢");
    }

    #[test]
    fn escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    // ── build_telegram_json ────────────────────────────────────

    #[test]
    fn telegram_json_simple() {
        assert_eq!(
            build_telegram_json("12345", "hello world"),
            r#"{"chat_id":"12345","text":"hello world"}"#
        );
    }

    #[test]
    fn telegram_json_escaped_text() {
        assert_eq!(
            build_telegram_json("99", r#"say "hi""#),
            r#"{"chat_id":"99","text":"say \"hi\""}"#
        );
    }

    #[test]
    fn telegram_json_emoji() {
        let j = build_telegram_json("-100123", "Place is now open! 🟢");
        assert!(j.contains("Place is now open! 🟢"));
    }
}
