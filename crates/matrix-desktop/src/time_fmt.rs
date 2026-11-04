//! The one place that turns a Matrix timestamp (ms since epoch) into the «HH:MM» shown in the UI.
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};

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

// Provisional (Telegram Desktop screenshot not received yet): change formats and threshold here only.
const LIST_WEEKDAYS: [&str; 7] = ["lun", "mar", "mer", "gio", "ven", "sab", "dom"];
const LIST_DATE_FORMAT: &str = "%d/%m/%Y";
/// Days back (inclusive) that still show the weekday name.
const LIST_WEEKDAY_MAX_DAYS: i64 = 6;

/// Conversation-list stamp of the last message: «HH:MM» on the local day of `now_ms`, the short weekday from
/// yesterday to `LIST_WEEKDAY_MAX_DAYS` ago, else «31/10/2026». `None` when absent (or out of range): show nothing.
pub fn last_message_stamp_in<Tz: TimeZone>(timestamp_ms: Option<u64>, now_ms: u64, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let local = |ms: u64| i64::try_from(ms).ok().and_then(DateTime::from_timestamp_millis).map(|t| t.with_timezone(tz));
    let ts = timestamp_ms?;
    let (t, now) = (local(ts)?, local(now_ms)?);
    // Calendar-day difference, so 23 h / 25 h DST days still count as one day.
    Some(match now.date_naive().signed_duration_since(t.date_naive()).num_days() {
        ..=0 => clock_time_in(ts, tz),
        1..=LIST_WEEKDAY_MAX_DAYS => LIST_WEEKDAYS[t.weekday().num_days_from_monday() as usize].to_owned(),
        _ => t.format(LIST_DATE_FORMAT).to_string(),
    })
}

/// Same, now and in the system zone.
pub fn last_message_stamp(timestamp_ms: Option<u64>) -> Option<String> {
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
    last_message_stamp_in(timestamp_ms, now, &Local)
}

/// Epoch ms of the next local midnight after `now_ms` (when every stamp may change): the UI sleeps until then.
/// Skipped-midnight zones fall back to the first valid local hour.
pub fn next_local_midnight_ms_in<Tz: TimeZone>(now_ms: u64, tz: &Tz) -> Option<u64> {
    let now = DateTime::from_timestamp_millis(i64::try_from(now_ms).ok()?)?.with_timezone(tz);
    let day: NaiveDate = now.date_naive().succ_opt()?;
    (0..24)
        .find_map(|h| tz.from_local_datetime(&day.and_hms_opt(0, 0, 0)?.checked_add_signed(Duration::hours(h))?).earliest())
        .and_then(|t| u64::try_from(t.timestamp_millis()).ok())
}

/// Same, from the system clock and zone.
pub fn next_local_midnight_ms() -> Option<u64> {
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).ok()?;
    next_local_midnight_ms_in(now, &Local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use chrono::NaiveDate;

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

    // Test-only Europe/Rome for 2026: +1 h, +2 h between 2026-03-29 01:00 UTC and 2026-10-25 01:00 UTC.
    const DST_START: i64 = 1_774_746_000;
    const DST_END: i64 = 1_792_890_000;
    #[derive(Clone, Copy)]
    struct Rome;
    impl TimeZone for Rome {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Rome
        }
        fn offset_from_local_date(&self, d: &NaiveDate) -> chrono::LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, l: &chrono::NaiveDateTime) -> chrono::LocalResult<FixedOffset> {
            let (cet, cest) = (FixedOffset::east_opt(3600).unwrap(), FixedOffset::east_opt(7200).unwrap());
            let (a, b) = (l.and_utc().timestamp() - 3600, l.and_utc().timestamp() - 7200); // as CET / as CEST
            let (a_ok, b_ok) = ((..DST_START).contains(&a) || a >= DST_END, (DST_START..DST_END).contains(&b));
            match (a_ok, b_ok) {
                (true, true) => chrono::LocalResult::Ambiguous(cest, cet),
                (true, false) => chrono::LocalResult::Single(cet),
                (false, true) => chrono::LocalResult::Single(cest),
                _ => chrono::LocalResult::None,
            }
        }
        fn offset_from_utc_date(&self, d: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, u: &chrono::NaiveDateTime) -> FixedOffset {
            let s = if (DST_START..DST_END).contains(&u.and_utc().timestamp()) { 7200 } else { 3600 };
            FixedOffset::east_opt(s).unwrap()
        }
    }

    /// UTC ms of a Rome wall-clock time (unambiguous cases only).
    fn rome(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        let t = Rome.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap();
        u64::try_from(t.timestamp_millis()).unwrap()
    }
    fn stamp(ts: u64, now: u64) -> Option<String> {
        last_message_stamp_in(Some(ts), now, &Rome)
    }

    #[test]
    fn stamp_same_day_yesterday_and_week_boundary() {
        let now = rome(2026, 11, 4, 12, 0); // Wednesday
        assert_eq!(stamp(rome(2026, 11, 4, 0, 0), now).unwrap(), "00:00");
        assert_eq!(stamp(rome(2026, 11, 3, 23, 59), now).unwrap(), "mar");
        assert_eq!(stamp(rome(2026, 10, 29, 8, 0), now).unwrap(), "gio"); // 6 days
        assert_eq!(stamp(rome(2026, 10, 28, 8, 0), now).unwrap(), "28/10/2026"); // 7 days
        assert_eq!(stamp(rome(2025, 12, 31, 8, 0), now).unwrap(), "31/12/2025");
    }

    #[test]
    fn stamp_midnight_follows_local_day_not_utc() {
        // 23:59 and 00:01 Rome are on different local days (both are about 23:00 UTC).
        let (late, early) = (rome(2026, 1, 15, 23, 59), rome(2026, 1, 16, 0, 1));
        assert_eq!(stamp(late, late + 30_000).unwrap(), "23:59");
        assert_eq!(stamp(late, early).unwrap(), "gio");
        assert_eq!(stamp(early, early).unwrap(), "00:01");
    }

    #[test]
    fn stamp_across_dst_days() {
        // 2026-03-29 has 23 h: Sunday 00:30 is still «yesterday» on Monday 00:10 (only 22 h 40 min earlier).
        let (s, m) = (rome(2026, 3, 29, 0, 30), rome(2026, 3, 30, 0, 10));
        assert_eq!(stamp(s, m).unwrap(), "dom");
        // 2026-10-25 has 25 h: Saturday 23:30 and Sunday 23:30 are still different days.
        assert_eq!(stamp(rome(2026, 10, 24, 23, 30), rome(2026, 10, 25, 23, 30)).unwrap(), "sab");
        assert_eq!(stamp(rome(2026, 10, 25, 0, 30), rome(2026, 10, 25, 23, 30)).unwrap(), "00:30");
        // Week boundary over the DST change: 7 calendar days back is a date, even if only 6 d 23 h elapsed.
        assert_eq!(stamp(rome(2026, 3, 25, 0, 30), rome(2026, 4, 1, 0, 10)).unwrap(), "25/03/2026");
    }

    #[test]
    fn stamp_missing_or_out_of_range_is_none() {
        assert_eq!(last_message_stamp_in(None, 0, &Rome), None);
        assert_eq!(last_message_stamp_in(Some(u64::MAX), 0, &Rome), None);
    }

    #[test]
    fn stamp_other_zone_than_utc() {
        let tokyo = FixedOffset::east_opt(9 * 3600).unwrap();
        // 2026-01-15 23:30 UTC is already Friday 08:30 in Tokyo.
        assert_eq!(last_message_stamp_in(Some(WINTER_2330), WINTER_2330, &tokyo).unwrap(), "08:30");
        assert_eq!(last_message_stamp_in(Some(WINTER_2330), WINTER_2330 + 86_400_000, &tokyo).unwrap(), "ven");
    }

    #[test]
    fn next_midnight_normal_and_dst_days() {
        assert_eq!(next_local_midnight_ms_in(rome(2026, 11, 4, 12, 0), &Rome), Some(rome(2026, 11, 5, 0, 0)));
        assert_eq!(next_local_midnight_ms_in(rome(2026, 11, 4, 23, 59), &Rome), Some(rome(2026, 11, 5, 0, 0)));
        // From Saturday evening the next midnight is 25 h-day Sunday's start; from Sunday it is 25 h later.
        let sun = rome(2026, 10, 25, 0, 0);
        assert_eq!(next_local_midnight_ms_in(rome(2026, 10, 24, 20, 0), &Rome), Some(sun));
        assert_eq!(next_local_midnight_ms_in(sun, &Rome).unwrap() - sun, 25 * 3_600_000);
        let mar = rome(2026, 3, 29, 0, 0);
        assert_eq!(next_local_midnight_ms_in(mar, &Rome).unwrap() - mar, 23 * 3_600_000);
    }
}
