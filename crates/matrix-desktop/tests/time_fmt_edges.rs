//! QA edge cases of the shared time module: exact midnight to the millisecond, 6/7-day edge on a leap day and a
//! year boundary, negative offsets, next-midnight on DST days (run with TZ=Europe/Rome and TZ=America/Sao_Paulo), spoken form.
use chrono::{FixedOffset, Local, TimeZone, Utc};
use matrix_desktop::time_fmt::*;

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, milli: u32) -> u64 {
    (Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap().timestamp_millis() as u64) + u64::from(milli)
}
fn zone(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).unwrap()
}
fn tz_is(name: &str) -> bool {
    std::env::var("TZ").as_deref() == Ok(name)
}

#[test]
fn exact_midnight_to_the_millisecond() {
    let p = Patterns::fallback();
    let now = ms(2026, 11, 6, 12, 0, 0, 0);
    for z in [zone(0), zone(1), zone(-5), zone(9)] {
        let off = i64::from(z.local_minus_utc()) * 1000;
        let local_midnight = (ms(2026, 11, 6, 0, 0, 0, 0) as i64 - off) as u64;
        // `now` must stay the same local day as the probes in every zone: use a now just after the local midnight.
        let now = local_midnight + 3_600_000;
        assert_eq!(day_label_in(local_midnight - 1, now, &z, &p), "Ieri", "{z}");
        assert_eq!(day_label_in(local_midnight, now, &z, &p), "Oggi", "{z}");
        assert_eq!(list_stamp_in(local_midnight, now, &z, &p), "00:00", "{z}");
        assert_eq!(list_stamp_in(local_midnight - 1, now, &z, &p), "Gio", "{z}");
        assert!(starts_day_in(Some(local_midnight - 1), local_midnight, &z), "{z}");
        assert!(!starts_day_in(Some(local_midnight), local_midnight + 86_399_999, &z), "{z}");
        assert!(starts_day_in(Some(local_midnight), local_midnight + 86_400_000, &z), "{z}");
    }
    let _ = now;
}

#[test]
fn six_and_seven_days_on_leap_day_and_year_boundary() {
    let p = Patterns::fallback();
    let z = zone(0);
    // 2024 is a leap year: 29 Feb (Thu) -> 6 Mar is 6 days, 28 Feb -> 6 Mar is 7.
    let now = ms(2024, 3, 6, 12, 0, 0, 0);
    assert_eq!(list_stamp_in(ms(2024, 2, 29, 9, 0, 0, 0), now, &z, &p), "Gio");
    assert_eq!(list_stamp_in(ms(2024, 2, 28, 9, 0, 0, 0), now, &z, &p), "28/02/24");
    // Year boundary: 28 Dec 2026 (Mon) -> 3 Jan 2027 is 6 days; 27 Dec is 7 and prints the old year.
    let now = ms(2027, 1, 3, 12, 0, 0, 0);
    assert_eq!(day_label_in(ms(2026, 12, 28, 9, 0, 0, 0), now, &z, &p), "Lun");
    assert_eq!(day_label_in(ms(2026, 12, 27, 9, 0, 0, 0), now, &z, &p), "27/12/26");
    // A 6-day-old message sent at 00:00 and a 7-day-old one at 23:59 stay on their side of the line.
    let now = ms(2026, 11, 6, 0, 0, 0, 0);
    assert_eq!(list_stamp_in(ms(2026, 10, 31, 0, 0, 0, 0), now, &z, &p), "Sab");
    assert_eq!(list_stamp_in(ms(2026, 10, 30, 23, 59, 59, 999), now, &z, &p), "30/10/26");
}

#[test]
fn day_label_and_stamp_agree_on_the_same_day_in_negative_offsets() {
    let p = Patterns::fallback();
    let z = zone(-5); // UTC 03:00 on the 6th is 22:00 on the 5th
    let now = ms(2026, 11, 6, 3, 0, 0, 0);
    assert_eq!(day_label_in(ms(2026, 11, 6, 2, 0, 0, 0), now, &z, &p), "Oggi"); // 21:00 on the 5th
    assert_eq!(list_stamp_in(ms(2026, 11, 6, 2, 0, 0, 0), now, &z, &p), "21:00");
    assert_eq!(day_label_in(ms(2026, 11, 5, 4, 59, 0, 0), now, &z, &p), "Ieri"); // 23:59 on the 4th
}

#[test]
fn next_midnight_on_the_dst_days_of_rome() {
    if !tz_is("Europe/Rome") {
        return;
    }
    let next = |t| next_local_midnight_ms_in(t, &Local).unwrap();
    // 28 Mar 12:00 UTC -> 29 Mar 00:00 CET (= 28 Mar 23:00 UTC); on the 23 h day the next midnight is 22:00 UTC.
    assert_eq!(next(ms(2026, 3, 28, 12, 0, 0, 0)), ms(2026, 3, 28, 23, 0, 0, 0));
    assert_eq!(next(ms(2026, 3, 29, 12, 0, 0, 0)), ms(2026, 3, 29, 22, 0, 0, 0));
    // 24 Oct 12:00 UTC -> 25 Oct 00:00 CEST (22:00 UTC); on the 25 h day the next midnight is 23:00 UTC the same evening.
    assert_eq!(next(ms(2026, 10, 24, 12, 0, 0, 0)), ms(2026, 10, 24, 22, 0, 0, 0));
    assert_eq!(next(ms(2026, 10, 25, 12, 0, 0, 0)), ms(2026, 10, 25, 23, 0, 0, 0));
    // Exactly at midnight the next one is a full day away, never `now` itself.
    let m = ms(2026, 11, 5, 23, 0, 0, 0);
    assert_eq!(next(m), m + 86_400_000);
    // Right after the autumn repeat of 02:00 the label still says "Oggi" for the first pass.
    let p = Patterns::fallback();
    let first = ms(2026, 10, 25, 0, 30, 0, 0); // 02:30 CEST
    let second = ms(2026, 10, 25, 1, 30, 0, 0); // 02:30 CET
    assert_eq!(clock_time_in(first, &Local, &p), clock_time_in(second, &Local, &p));
    assert!(!starts_day_in(Some(first), second, &Local));
}

/// Brazil moved the clocks at midnight until 2018: 4 Nov 2018 00:00 does not exist, the day starts at 01:00 (-02).
#[test]
fn next_midnight_when_midnight_does_not_exist() {
    if !tz_is("America/Sao_Paulo") {
        return;
    }
    let got = next_local_midnight_ms_in(ms(2018, 11, 3, 15, 0, 0, 0), &Local).unwrap();
    assert_eq!(got, ms(2018, 11, 4, 3, 0, 0, 0)); // 01:00 -02
}

#[test]
fn spoken_form_of_list_stamps() {
    assert_eq!(last_message_spoken("14:27"), "ultimo messaggio alle 14:27");
    assert_eq!(last_message_spoken("2:27 PM"), "ultimo messaggio alle 2:27 PM");
    assert_eq!(last_message_spoken("30.10.26"), "ultimo messaggio il 30.10.26");
    assert_eq!(last_message_spoken("26/10/30"), "ultimo messaggio il 26/10/30");
}

/// ja_JP on Linux: glibc's T_FMT «%H時%M分%S秒» becomes «HH時mm分», a time with no colon: the spoken form follows the value.
#[test]
fn spoken_form_of_a_time_without_colon() {
    let p = Patterns { time: "HH時mm分".into(), ..Patterns::fallback() };
    let stamp = list_stamp_in(ms(2026, 11, 6, 14, 5, 0, 0), ms(2026, 11, 6, 18, 0, 0, 0), &zone(0), &p);
    assert_eq!(stamp, "14時05分");
    let (at, now) = (ms(2026, 11, 6, 14, 5, 0, 0), ms(2026, 11, 6, 18, 0, 0, 0));
    assert_eq!(last_message_spoken_in(at, now, &zone(0), &p), "ultimo messaggio alle 14時05分");
    assert_eq!(last_message_spoken_in(at - 8 * 86_400_000, now, &zone(0), &p), "ultimo messaggio il 29/10/26");
}
