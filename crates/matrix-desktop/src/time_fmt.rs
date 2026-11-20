//! The ONE place that turns a Matrix timestamp (ms since epoch) into the day / date / time text shown in the UI.
//! No other file may contain a date or time format: change the rules here and they change everywhere.
//!
//! Rules (day = calendar day in the zone, so midnight and DST switches follow the wall clock, never 24 h blocks):
//! - today: the time only; yesterday up to 6 days back: abbreviated weekday + time; older: short date + time.
//! - separator between days (`day_label`): «Today» / «Yesterday» / abbreviated weekday / short date (no time).
//! - weekday, short date order/separators (year always 2 digits), 12/24 h and AM/PM text come from the system
//!   locale, read once through the platform formatter (macOS CFDateFormatter, Windows GetLocaleInfoEx,
//!   Linux nl_langinfo) as an ICU-style pattern: d/dd, M/MM, y (always printed with 2 digits), H/HH, h/hh, m/mm,
//!   a (or t) for AM/PM, 'quoted text'. «Today»/«Yesterday» are UI strings (`i18n::t`), not formats.
//! - Fallback when the locale cannot be read (Linux with LANG unset/C/POSIX or not installed, a failing API call,
//!   unsupported OS): `Patterns::fallback()` = the reference it_IT look «Mer» / «GG/MM/AA» / «HH:mm».
//! - Locale and zone are injected (`Patterns`, `tz`, `now_ms`) so tests do not depend on the machine.
//!
//! shortcut: the system patterns are read once per run, a 12/24 h or locale change needs a restart.
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use std::sync::OnceLock;

/// What the locale decides. `weekdays` start on Monday.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patterns {
    pub weekdays: [String; 7],
    pub date: String,
    pub time: String,
    pub am: String,
    pub pm: String,
}

impl Patterns {
    /// Reference it_IT look, used when the system locale is unreadable.
    pub fn fallback() -> Self {
        let s = |v: &str| v.to_owned();
        Self {
            weekdays: ["Lun", "Mar", "Mer", "Gio", "Ven", "Sab", "Dom"].map(s),
            date: s("dd/MM/yy"),
            time: s("HH:mm"),
            am: s("AM"),
            pm: s("PM"),
        }
    }
}

/// System patterns, read once.
pub fn system() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| native().unwrap_or_else(Patterns::fallback))
}

fn render<Tz: TimeZone>(pattern: &str, t: &DateTime<Tz>, p: &Patterns) -> String {
    let ch: Vec<char> = pattern.chars().collect();
    let (mut out, mut i) = (String::new(), 0);
    while i < ch.len() {
        let c = ch[i];
        if c == '\'' {
            if ch.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
            } else {
                i += 1;
                while i < ch.len() && ch[i] != '\'' {
                    out.push(ch[i]);
                    i += 1;
                }
                i += 1;
            }
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = ch[i..].iter().take_while(|&&x| x == c).count();
        i += n;
        let h = t.hour();
        let num = |v: u32| {
            if n == 1 {
                v.to_string()
            } else {
                format!("{v:02}")
            }
        };
        match c {
            'y' | 'Y' | 'u' => out.push_str(&format!("{:02}", t.year().rem_euclid(100))),
            'M' | 'L' => out.push_str(&num(t.month())),
            'd' => out.push_str(&num(t.day())),
            'H' | 'k' => out.push_str(&num(h)),
            'h' | 'K' => out.push_str(&num(if h.is_multiple_of(12) { 12 } else { h % 12 })),
            'm' => out.push_str(&num(t.minute())),
            's' => out.push_str(&num(t.second())),
            'a' | 't' => out.push_str(if h < 12 { &p.am } else { &p.pm }),
            _ => out.extend(std::iter::repeat_n(c, n)),
        }
    }
    out
}

fn local<Tz: TimeZone>(ms: u64, tz: &Tz) -> Option<DateTime<Tz>> {
    i64::try_from(ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|t| t.with_timezone(tz))
}

/// Calendar days from `t` to `now` in the zone (<= 0: today, or a clock-skewed future).
fn days_ago<Tz: TimeZone>(t: &DateTime<Tz>, now: &DateTime<Tz>) -> i64 {
    now.date_naive()
        .signed_duration_since(t.date_naive())
        .num_days()
}

/// Abbreviated weekday within the last 6 days, short date before.
fn weekday_or_date<Tz: TimeZone>(t: &DateTime<Tz>, days: i64, p: &Patterns) -> String {
    if days <= 6 {
        let w = &p.weekdays[t.weekday().num_days_from_monday() as usize];
        let mut c = w.chars();
        c.next()
            .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
    } else {
        render(&p.date, t, p)
    }
}

/// True when `ms` falls on another calendar day than `prev_ms` in `tz` (the first message, `None`, always starts a day).
pub fn starts_day_in<Tz: TimeZone>(prev_ms: Option<u64>, ms: u64, tz: &Tz) -> bool {
    match (prev_ms.and_then(|p| local(p, tz)), local(ms, tz)) {
        (Some(p), Some(t)) => p.date_naive() != t.date_naive(),
        (None, Some(_)) => true,
        _ => false,
    }
}

/// Time of `ms` in `tz`; «--:--» when out of range.
pub fn clock_time_in<Tz: TimeZone>(ms: u64, tz: &Tz, p: &Patterns) -> String {
    local(ms, tz).map_or_else(|| "--:--".to_owned(), |t| render(&p.time, &t, p))
}

/// Day separator: «Today», «Yesterday», abbreviated weekday (2..=6 days back) or short date; empty when out of range.
pub fn day_label_in<Tz: TimeZone>(ms: u64, now_ms: u64, tz: &Tz, p: &Patterns) -> String {
    let (Some(t), Some(now)) = (local(ms, tz), local(now_ms, tz)) else {
        return String::new();
    };
    match days_ago(&t, &now) {
        ..=0 => crate::i18n::t("Today").to_owned(),
        1 => crate::i18n::t("Yesterday").to_owned(),
        d => weekday_or_date(&t, d, p),
    }
}

/// Time today; weekday (yesterday..6 days) or short date, then the time, before.
pub fn day_or_time_in<Tz: TimeZone>(ms: u64, now_ms: u64, tz: &Tz, p: &Patterns) -> String {
    let (Some(t), Some(now)) = (local(ms, tz), local(now_ms, tz)) else {
        return clock_time_in(ms, tz, p);
    };
    match days_ago(&t, &now) {
        ..=0 => render(&p.time, &t, p),
        d => format!("{} {}", weekday_or_date(&t, d, p), render(&p.time, &t, p)),
    }
}

/// Conversation-list stamp (Telegram): time today, abbreviated weekday from yesterday to 6 days back, short date before.
pub fn list_stamp_in<Tz: TimeZone>(ms: u64, now_ms: u64, tz: &Tz, p: &Patterns) -> String {
    let (Some(t), Some(now)) = (local(ms, tz), local(now_ms, tz)) else {
        return clock_time_in(ms, tz, p);
    };
    match days_ago(&t, &now) {
        ..=0 => render(&p.time, &t, p),
        d => weekday_or_date(&t, d, p),
    }
}

/// Screen-reader form of a list stamp; `is_time` (known from the value, not the text: ja_JP «14時05分» has no colon) picks «alle» vs «il».
fn spoken(stamp: &str, is_time: bool) -> String {
    let when = if is_time {
        format!("alle {stamp}")
    } else if stamp.chars().any(|c| c.is_ascii_digit()) {
        format!("il {stamp}")
    } else {
        stamp.to_owned()
    };
    format!("ultimo messaggio {when}")
}

/// Spoken form of the stamp `list_stamp_in` gives for the same arguments.
pub fn last_message_spoken_in<Tz: TimeZone>(ms: u64, now_ms: u64, tz: &Tz, p: &Patterns) -> String {
    let today = matches!((local(ms, tz), local(now_ms, tz)), (Some(t), Some(now)) if days_ago(&t, &now) <= 0);
    spoken(&list_stamp_in(ms, now_ms, tz, p), today)
}

/// Spoken form of a stamp known only as text (offline fixtures): a time is taken to contain «:».
pub fn last_message_spoken(stamp: &str) -> String {
    spoken(stamp, stamp.contains(':'))
}

/// Epoch ms of the next local midnight after `now_ms` (when every stamp may change): the UI sleeps until then.
/// Skipped-midnight zones fall back to the first valid local hour.
pub fn next_local_midnight_ms_in<Tz: TimeZone>(now_ms: u64, tz: &Tz) -> Option<u64> {
    let now = DateTime::from_timestamp_millis(i64::try_from(now_ms).ok()?)?.with_timezone(tz);
    let day = now.date_naive().succ_opt()?;
    (0..24)
        .find_map(|h| tz.from_local_datetime(&day.and_hms_opt(h, 0, 0)?).earliest())
        .and_then(|t| u64::try_from(t.timestamp_millis()).ok())
}

pub fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// System zone and locale.
pub fn clock_time(ms: u64) -> String {
    clock_time_in(ms, &Local, system())
}
pub fn starts_day(prev_ms: Option<u64>, ms: u64) -> bool {
    starts_day_in(prev_ms, ms, &Local)
}
pub fn day_label(ms: u64) -> String {
    day_label_in(ms, now_ms(), &Local, system())
}
pub fn day_or_time(ms: u64) -> String {
    day_or_time_in(ms, now_ms(), &Local, system())
}

pub fn list_stamp(ms: u64) -> String {
    list_stamp_in(ms, now_ms(), &Local, system())
}
/// `None` (no timestamp known) shows nothing.
pub fn last_message_stamp(ms: Option<u64>) -> Option<String> {
    ms.map(list_stamp)
}
pub fn last_message_spoken_ms(ms: u64) -> String {
    last_message_spoken_in(ms, now_ms(), &Local, system())
}
pub fn next_local_midnight_ms() -> Option<u64> {
    next_local_midnight_ms_in(now_ms(), &Local)
}

/// macOS: CFDateFormatter. `locale_id` None = the user's current locale (honours the 24-hour setting through the «jm» template).
#[cfg(target_os = "macos")]
pub fn native_for(locale_id: Option<&str>) -> Option<Patterns> {
    use objc2_core_foundation::{
        CFDateFormatter, CFDateFormatterStyle, CFLocale, CFString, CFTimeZone,
        kCFDateFormatterTimeZone,
    };
    let loc = match locale_id {
        Some(id) => CFLocale::new(None, Some(&CFString::from_str(id)))?,
        None => CFLocale::current()?,
    };
    // SAFETY: plain CoreFoundation constructors with valid (or documented-nullable) arguments.
    let (df, time) = unsafe {
        let df = CFDateFormatter::new(
            None,
            Some(&loc),
            CFDateFormatterStyle::ShortStyle,
            CFDateFormatterStyle::NoStyle,
        )?;
        let tpl = CFString::from_str("jm");
        (
            df,
            CFDateFormatter::new_date_format_from_template(None, Some(&tpl), 0, Some(&loc))?,
        )
    };
    let date = df.format()?.to_string();
    let gmt = CFTimeZone::with_time_interval_from_gmt(None, 0.0)?;
    // SAFETY: the key is a CF constant, the value a live CFTimeZone.
    unsafe { df.set_property(kCFDateFormatterTimeZone, Some(gmt.as_ref())) };
    // 2001-01-01 (CF absolute time 0) is a Monday; format noon of day `d` / 01:00 / 13:00 with a one-field pattern.
    let fmt = |pattern: &str, secs: f64| -> Option<String> {
        // SAFETY: `df` is a live formatter.
        unsafe {
            df.set_format(Some(&CFString::from_str(pattern)));
            CFDateFormatter::new_string_with_absolute_time(None, Some(&df), secs)
        }
        .map(|s| s.to_string())
    };
    let mut weekdays: [String; 7] = Default::default();
    for (d, w) in weekdays.iter_mut().enumerate() {
        *w = fmt("ccc", d as f64 * 86_400.0 + 43_200.0)?;
    }
    Some(Patterns {
        weekdays,
        date,
        time: time.to_string(),
        am: fmt("a", 3_600.0)?,
        pm: fmt("a", 13.0 * 3_600.0)?,
    })
}

#[cfg(target_os = "macos")]
fn native() -> Option<Patterns> {
    native_for(None)
}

/// Windows: the user's locale (Region settings, overrides included) via GetLocaleInfoEx; its patterns use d/M/y/H/h/m/tt like ICU.
#[cfg(windows)]
fn native() -> Option<Patterns> {
    use windows::Win32::Globalization::{
        GetLocaleInfoEx, LOCALE_S1159, LOCALE_S2359, LOCALE_SABBREVDAYNAME1, LOCALE_SSHORTDATE,
        LOCALE_SSHORTTIME,
    };
    use windows::core::PCWSTR;
    let info = |kind: u32| -> Option<String> {
        let mut b = [0u16; 80];
        // SAFETY: `b` outlives the call; the API writes at most b.len() units including the NUL.
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut b)) };
        (n > 1).then(|| String::from_utf16_lossy(&b[..n as usize - 1]))
    };
    let mut weekdays: [String; 7] = Default::default();
    for (d, w) in weekdays.iter_mut().enumerate() {
        *w = info(LOCALE_SABBREVDAYNAME1 + d as u32)?; // DAYNAME1 = Monday
    }
    Some(Patterns {
        weekdays,
        date: info(LOCALE_SSHORTDATE)?,
        time: info(LOCALE_SSHORTTIME)?,
        am: info(LOCALE_S1159)?,
        pm: info(LOCALE_S2359)?,
    })
}

/// glibc strftime format (D_FMT / T_FMT) -> our pattern. Seconds are dropped with their separator; unknown conversions are dropped.
#[cfg(any(target_os = "linux", test))]
fn strftime_to_pattern(f: &str) -> String {
    let f = f
        .replace(":%S", "")
        .replace(".%S", "")
        .replace("%S秒", "")
        .replace("%S", "");
    let (mut out, mut quoted) = (String::new(), false);
    let mut it = f.chars();
    while let Some(c) = it.next() {
        let piece = if c == '%' {
            match it.next() {
                Some('d') => "dd",
                Some('e') => "d",
                Some('m') => "MM",
                Some('y' | 'Y') => "yy",
                Some('H') => "HH",
                Some('k') => "H",
                Some('I') => "hh",
                Some('l') => "h",
                Some('M') => "mm",
                Some('p' | 'P') => "a",
                Some('T' | 'R') => "HH:mm",
                Some('r') => "hh:mm a",
                Some('D') => "MM/dd/yy",
                Some('F') => "yy-MM-dd",
                Some('%') => "%",
                _ => "",
            }
            .to_owned()
        } else if c == '\'' {
            "''".to_owned()
        } else if c.is_ascii_alphabetic() {
            if !quoted {
                out.push('\'');
                quoted = true;
            }
            out.push(c);
            continue;
        } else {
            c.to_string()
        };
        if quoted {
            out.push('\'');
            quoted = false;
        }
        out.push_str(&piece);
    }
    if quoted {
        out.push('\'');
    }
    out
}

/// Linux: LC_ALL / LC_TIME / LANG (first non-empty) through glibc newlocale + nl_langinfo_l (no process-wide setlocale).
/// There is no separate 12/24 h setting on Linux: it follows the locale's own time format.
#[cfg(target_os = "linux")]
fn native() -> Option<Patterns> {
    use std::ffi::{CStr, CString};
    let name = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))?;
    if name == "C" || name == "POSIX" || name.starts_with("C.") {
        return None;
    }
    let name = CString::new(name).ok()?;
    // SAFETY: newlocale/nl_langinfo_l with a valid NUL-terminated name; the returned strings are copied before freelocale.
    unsafe {
        let loc = libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            return None; // locale not installed
        }
        let s = |item| {
            CStr::from_ptr(libc::nl_langinfo_l(item, loc))
                .to_string_lossy()
                .into_owned()
        };
        let weekdays = [
            libc::ABDAY_2,
            libc::ABDAY_3,
            libc::ABDAY_4,
            libc::ABDAY_5,
            libc::ABDAY_6,
            libc::ABDAY_7,
            libc::ABDAY_1,
        ]
        .map(s);
        let p = Patterns {
            weekdays,
            date: strftime_to_pattern(&s(libc::D_FMT)),
            time: strftime_to_pattern(&s(libc::T_FMT)),
            am: s(libc::AM_STR),
            pm: s(libc::PM_STR),
        };
        libc::freelocale(loc);
        Some(p)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn native() -> Option<Patterns> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    #[test]
    fn list_stamp_is_time_weekday_or_date() {
        let (p, z, now) = (Patterns::fallback(), utc(), ms(2026, 11, 4, 12, 0)); // Wednesday
        let at = |t| list_stamp_in(t, now, &z, &p);
        assert_eq!(at(ms(2026, 11, 4, 0, 5)), "00:05");
        assert_eq!(at(ms(2026, 11, 3, 23, 59)), "Mar");
        assert_eq!(at(ms(2026, 10, 29, 9, 0)), "Gio"); // 6 days
        assert_eq!(at(ms(2026, 10, 28, 9, 0)), "28/10/26"); // 7 days
        assert_eq!(last_message_stamp(None), None);
        assert_eq!(last_message_spoken("14:27"), "ultimo messaggio alle 14:27");
        assert_eq!(last_message_spoken("28/10/26"), "ultimo messaggio il 28/10/26");
    }

    #[test]
    fn next_midnight_is_the_following_local_day() {
        let z = FixedOffset::east_opt(3600).unwrap();
        assert_eq!(next_local_midnight_ms_in(ms(2026, 11, 4, 12, 0), &z), Some(ms(2026, 11, 4, 23, 0)));
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }
    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis() as u64
    }
    /// CLDR-like fixtures for the injected locales (weekdays lower case as the platforms return them).
    fn loc(name: &str) -> Patterns {
        let (wd, date, time, am, pm): ([&str; 7], &str, &str, &str, &str) = match name {
            "it_IT" => (
                ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
                "dd/MM/yy",
                "HH:mm",
                "AM",
                "PM",
            ),
            "en_US" => (
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
                "M/d/yy",
                "h:mm a",
                "AM",
                "PM",
            ),
            "en_GB" => (
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
                "dd/MM/y",
                "HH:mm",
                "am",
                "pm",
            ),
            "de_DE" => (
                ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."],
                "dd.MM.yy",
                "HH:mm",
                "AM",
                "PM",
            ),
            "ja_JP" => (
                ["月", "火", "水", "木", "金", "土", "日"],
                "y/MM/dd",
                "H:mm",
                "午前",
                "午後",
            ),
            _ => unreachable!(),
        };
        Patterns {
            weekdays: wd.map(str::to_owned),
            date: date.into(),
            time: time.into(),
            am: am.into(),
            pm: pm.into(),
        }
    }

    #[test]
    fn locales_today_yesterday_six_seven_days_other_year() {
        // now = Friday 2026-11-06 18:00 UTC; every probe is at 14:05.
        let now = ms(2026, 11, 6, 18, 0);
        let probes = [
            ms(2026, 11, 6, 14, 5),
            ms(2026, 11, 5, 14, 5),
            ms(2026, 10, 31, 14, 5),
            ms(2026, 10, 30, 14, 5),
            ms(2025, 12, 31, 14, 5),
        ];
        let expected = [
            (
                "it_IT",
                [
                    "14:05",
                    "Gio 14:05",
                    "Sab 14:05",
                    "30/10/26 14:05",
                    "31/12/25 14:05",
                ],
                ["Today", "Yesterday", "Sab", "30/10/26", "31/12/25"],
            ),
            (
                "en_US",
                [
                    "2:05 PM",
                    "Thu 2:05 PM",
                    "Sat 2:05 PM",
                    "10/30/26 2:05 PM",
                    "12/31/25 2:05 PM",
                ],
                ["Today", "Yesterday", "Sat", "10/30/26", "12/31/25"],
            ),
            (
                "en_GB",
                [
                    "14:05",
                    "Thu 14:05",
                    "Sat 14:05",
                    "30/10/26 14:05",
                    "31/12/25 14:05",
                ],
                ["Today", "Yesterday", "Sat", "30/10/26", "31/12/25"],
            ),
            (
                "de_DE",
                [
                    "14:05",
                    "Do. 14:05",
                    "Sa. 14:05",
                    "30.10.26 14:05",
                    "31.12.25 14:05",
                ],
                ["Today", "Yesterday", "Sa.", "30.10.26", "31.12.25"],
            ),
            (
                "ja_JP",
                [
                    "14:05",
                    "木 14:05",
                    "土 14:05",
                    "26/10/30 14:05",
                    "25/12/31 14:05",
                ],
                ["Today", "Yesterday", "土", "26/10/30", "25/12/31"],
            ),
        ];
        for (name, times, labels) in expected {
            let p = loc(name);
            for (i, &t) in probes.iter().enumerate() {
                assert_eq!(day_or_time_in(t, now, &utc(), &p), times[i], "{name} #{i}");
                assert_eq!(
                    day_label_in(t, now, &utc(), &p),
                    labels[i],
                    "{name} label #{i}"
                );
            }
        }
    }

    #[test]
    fn twelve_and_twenty_four_hours_with_midnight_and_noon() {
        let (mut p, now) = (Patterns::fallback(), ms(2026, 11, 6, 18, 0));
        let at = |p: &Patterns, h, m| clock_time_in(ms(2026, 11, 6, h, m), &utc(), p);
        assert_eq!(
            (at(&p, 0, 5), at(&p, 12, 0), at(&p, 23, 59)),
            ("00:05".into(), "12:00".into(), "23:59".into())
        );
        p.time = "h:mm a".into();
        assert_eq!(
            (at(&p, 0, 5), at(&p, 12, 0), at(&p, 23, 59)),
            ("12:05 AM".into(), "12:00 PM".into(), "11:59 PM".into())
        );
        p.time = "a hh:mm".into();
        assert_eq!(at(&p, 9, 7), "AM 09:07");
        assert_eq!(
            day_or_time_in(ms(2026, 11, 5, 0, 0), now, &utc(), &p),
            "Gio AM 12:00"
        );
    }

    #[test]
    fn midnight_follows_the_zone_not_utc() {
        let cet = FixedOffset::east_opt(3600).unwrap();
        let p = Patterns::fallback();
        let t = ms(2026, 1, 15, 23, 30); // 00:30 on the 16th in CET
        assert_eq!(day_or_time_in(t, t + 3_600_000, &cet, &p), "00:30");
        assert_eq!(day_label_in(t, t + 3_600_000, &cet, &p), "Today");
        assert_eq!(
            day_label_in(t, t + 3_600_000 + 86_400_000, &cet, &p),
            "Yesterday"
        );
        assert_eq!(
            day_label_in(t, t + 3_600_000 + 7 * 86_400_000, &cet, &p),
            "16/01/26"
        );
        // In UTC the message is on the 15th: ten minutes later still today, one hour later (the 16th) yesterday.
        assert_eq!(day_label_in(t, t + 600_000, &utc(), &p), "Today");
        assert_eq!(day_label_in(t, t + 3_600_000, &utc(), &p), "Yesterday");
    }

    #[test]
    fn out_of_range_and_future_do_not_panic() {
        let p = Patterns::fallback();
        assert_eq!(clock_time_in(u64::MAX, &utc(), &p), "--:--");
        assert_eq!(day_label_in(u64::MAX, 0, &utc(), &p), "");
        assert_eq!(day_or_time_in(u64::MAX, 0, &utc(), &p), "--:--");
        assert_eq!(
            day_label_in(ms(2026, 11, 7, 1, 0), ms(2026, 11, 6, 1, 0), &utc(), &p),
            "Today"
        ); // clock skew
    }

    #[test]
    fn quotes_and_literals() {
        let mut p = Patterns::fallback();
        p.time = "HH 'Uhr' mm''".into();
        assert_eq!(
            clock_time_in(ms(2026, 1, 1, 9, 5), &utc(), &p),
            "09 Uhr 05'"
        );
    }

    #[test]
    fn glibc_formats_become_patterns() {
        assert_eq!(strftime_to_pattern("%d/%m/%Y"), "dd/MM/yy");
        assert_eq!(strftime_to_pattern("%m/%d/%Y"), "MM/dd/yy");
        assert_eq!(strftime_to_pattern("%H:%M:%S"), "HH:mm");
        assert_eq!(strftime_to_pattern("%r"), "hh:mm a");
        assert_eq!(strftime_to_pattern("%I:%M:%S %p"), "hh:mm a");
        assert_eq!(strftime_to_pattern("%H時%M分%S秒"), "HH時mm分");
        assert_eq!(strftime_to_pattern("%H:%M Uhr"), "HH:mm 'Uhr'");
    }

    /// The real macOS formatter for injected locales (no machine setting involved).
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_formatter_for_injected_locales() {
        let now = ms(2026, 11, 6, 18, 0);
        let yesterday = ms(2026, 11, 5, 14, 5);
        for (id, expected) in [("it_IT", "Gio"), ("en_GB", "Thu"), ("ja_JP", "木")] {
            let p = native_for(Some(id)).unwrap();
            assert!(
                day_or_time_in(yesterday, now, &utc(), &p).starts_with(expected),
                "{id}: {p:?}"
            );
        }
        let it = native_for(Some("it_IT")).unwrap();
        assert_eq!(
            day_or_time_in(ms(2026, 10, 30, 14, 5), now, &utc(), &it),
            "30/10/26 14:05"
        );
        let us = native_for(Some("en_US")).unwrap();
        assert!(
            clock_time_in(ms(2026, 11, 6, 14, 5), &utc(), &us).starts_with("2:05"),
            "{us:?}"
        );
    }
}
