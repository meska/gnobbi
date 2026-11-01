//! The one place that turns a Matrix timestamp (ms since epoch) into the «HH:MM» shown in the UI.
use chrono::{DateTime, Local, TimeZone};

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
}
