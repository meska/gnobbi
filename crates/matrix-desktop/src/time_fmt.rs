//! The one place that turns a Matrix timestamp (ms since epoch) into the «HH:MM» shown in the UI.
use chrono::{DateTime, Datelike, Local, TimeZone};

/// «HH:MM» (24 h) of `timestamp_ms` in `tz`; the zone is injected so tests do not depend on the machine.
pub fn clock_time_in<Tz: TimeZone>(timestamp_ms: u64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    i64::try_from(timestamp_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map_or_else(
            || "--:--".to_owned(),
            |t| t.with_timezone(tz).format("%H:%M").to_string(),
        )
}

/// Same, in the system zone (DST included).
pub fn clock_time(timestamp_ms: u64) -> String {
    clock_time_in(timestamp_ms, &Local)
}

const WEEKDAYS: [&str; 7] = ["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"];
const MONTHS: [&str; 12] = ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"];

/// «17:11» today, the weekday within the last 6 days, «25 ott» before; `now_ms` and the zone are injected for tests.
pub fn day_or_time_in<Tz: TimeZone>(timestamp_ms: u64, now_ms: u64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let local = |ms: u64| i64::try_from(ms).ok().and_then(DateTime::from_timestamp_millis).map(|t| t.with_timezone(tz));
    let (Some(t), Some(now)) = (local(timestamp_ms), local(now_ms)) else {
        return clock_time_in(timestamp_ms, tz);
    };
    match now.date_naive().signed_duration_since(t.date_naive()).num_days() {
        ..=0 => clock_time_in(timestamp_ms, tz),
        1..=6 => WEEKDAYS[t.weekday().num_days_from_monday() as usize].to_owned(),
        _ => format!("{} {}", t.day(), MONTHS[t.month0() as usize]),
    }
}

/// Same, now and in the system zone.
pub fn day_or_time(timestamp_ms: u64) -> String {
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
    day_or_time_in(timestamp_ms, now, &Local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const CET: i32 = 3600;
    const CEST: i32 = 7200;
    // 2026-01-15 and 2026-07-15, 23:30 at offset 0.
    const WINTER_2330: u64 = 1_768_519_800_000;
    const SUMMER_2330: u64 = 1_784_158_200_000;

    fn at(secs: i32, ms: u64) -> String {
        clock_time_in(ms, &FixedOffset::east_opt(secs).unwrap())
    }

    #[test]
    fn rome_winter_and_summer_offsets() {
        assert_eq!(at(CET, WINTER_2330), "00:30");
        assert_eq!(at(CEST, SUMMER_2330), "01:30");
        assert_eq!(at(0, WINTER_2330), "23:30");
    }

    #[test]
    fn pads_and_is_24_hours() {
        assert_eq!(at(CET, 0), "01:00");
        assert_eq!(at(0, 3_661_000), "01:01");
        assert_eq!(at(0, 13 * 3_600_000 + 5 * 60_000), "13:05");
    }

    #[test]
    fn out_of_range_does_not_panic() {
        assert_eq!(at(0, u64::MAX), "--:--");
    }

    #[test]
    fn today_weekday_or_date() {
        const DAY: u64 = 86_400_000;
        // WINTER_2330 is Thursday 2026-01-15 23:30 UTC.
        let z = |now| day_or_time_in(WINTER_2330, now, &FixedOffset::east_opt(0).unwrap());
        assert_eq!(z(WINTER_2330 + 20 * 60_000), "23:30");
        assert_eq!(z(WINTER_2330 + DAY), "giovedì");
        assert_eq!(z(WINTER_2330 + 6 * DAY), "giovedì");
        assert_eq!(z(WINTER_2330 + 7 * DAY), "15 gen");
        // Same instant, local midnight crossed in CET: "today" follows the zone, not UTC.
        let cet = FixedOffset::east_opt(CET).unwrap();
        assert_eq!(day_or_time_in(WINTER_2330, WINTER_2330 + 3_600_000, &cet), "00:30");
        assert_eq!(day_or_time_in(WINTER_2330, WINTER_2330 + 3_600_000 + DAY, &cet), "venerdì");
    }
}
