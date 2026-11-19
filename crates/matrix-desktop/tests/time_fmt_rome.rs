//! QA: system-zone clock around midnight and the two DST switches. Run with TZ=Europe/Rome.
use chrono::{TimeZone, Utc};
use matrix_desktop::time_fmt::{clock_time, clock_time_in, day_label_in, Patterns};
use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, RoomId, RoomScope};

fn rome() -> bool {
    std::env::var("TZ").as_deref() == Ok("Europe/Rome")
}
fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> String {
    clock_time_in(Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap().timestamp_millis() as u64, &chrono::Local, &Patterns::fallback())
}

#[test]
fn midnight_summer_and_winter() {
    if !rome() { return; }
    assert_eq!(at(2026, 6, 30, 21, 59, 59), "23:59");
    assert_eq!(at(2026, 6, 30, 22, 0, 0), "00:00");
    assert_eq!(at(2026, 12, 31, 22, 59, 59), "23:59");
    assert_eq!(at(2026, 12, 31, 23, 0, 0), "00:00");
}

#[test]
fn spring_forward_skips_0200_to_0300() {
    if !rome() { return; }
    assert_eq!(at(2026, 3, 29, 0, 59, 59), "01:59");
    assert_eq!(at(2026, 3, 29, 1, 0, 0), "03:00");
}

#[test]
fn autumn_back_repeats_the_0200_hour() {
    if !rome() { return; }
    assert_eq!(at(2026, 10, 25, 0, 59, 59), "02:59");
    assert_eq!(at(2026, 10, 25, 1, 0, 0), "02:00");
    assert_eq!(at(2026, 10, 25, 1, 59, 59), "02:59");
    assert_eq!(at(2026, 10, 25, 2, 0, 0), "03:00");
}

#[test]
fn never_prints_utc_and_always_five_chars() {
    for ms in [0u64, 1, 86_399_999, 1_784_158_200_000, u64::MAX] {
        let t = clock_time_in(ms, &chrono::Local, &Patterns::fallback());
        assert_eq!(t.len(), 5, "{t}");
        let _ = clock_time(ms); // system patterns never panic
        assert!(!t.to_uppercase().contains("UTC"));
    }
}

fn label(ts: (i32, u32, u32, u32, u32), now: (i32, u32, u32, u32, u32)) -> String {
    let ms = |(y, mo, d, h, mi): (i32, u32, u32, u32, u32)| Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis() as u64;
    day_label_in(ms(ts), ms(now), &chrono::Local, &Patterns::fallback())
}

/// Calendar days, not 24 h blocks: the 23 h and 25 h days of the DST switches must not shift the label.
#[test]
fn day_labels_across_the_dst_switches() {
    if !rome() { return; }
    // Sat 28 Mar 23:30 CET -> Mon 30 Mar 10:00 CEST: 34.5 h elapsed but two calendar days.
    assert_eq!(label((2026, 3, 28, 22, 30), (2026, 3, 30, 8, 0)), "Sab");
    // Sat 24 Oct 23:30 CEST -> Mon 26 Oct 00:10 CET: 25 h 40 elapsed but two calendar days.
    assert_eq!(label((2026, 10, 24, 21, 30), (2026, 10, 25, 23, 10)), "Sab");
    // Sun 25 Oct (25 h day) 12:00 -> Mon 26 Oct 00:10: yesterday.
    assert_eq!(label((2026, 10, 25, 11, 0), (2026, 10, 25, 23, 10)), "Ieri");
    // Sun 29 Mar 00:30 CET and Sun 29 Mar 12:00 CEST (the 23 h day): same day.
    assert_eq!(label((2026, 3, 28, 23, 30), (2026, 3, 29, 10, 0)), "Oggi");
}

#[test]
fn demo_row_time_matches_its_timestamp() {
    let rows = DemoHistory::new(RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!d:fixture.invalid") }, 130);
    for e in rows.events() {
        assert_eq!(e.time, clock_time(e.timestamp_ms), "demo row {}", e.text);
    }
}

/// Accessible node (AT-SPI projection) of a real server row carries the local time, not UTC.
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[test]
fn accessible_node_has_local_time() {
    use matrix_desktop::accessibility::HistoryNodes;
    use matrix_engine::history::{HistoryEvent, HistorySender};
    if !rome() { return; }
    let ts = Utc.with_ymd_and_hms(2026, 7, 15, 22, 30, 0).unwrap().timestamp_millis() as u64; // 00:30 CEST on the 16th
    let mut rows = DemoHistory::new(RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:fixture.invalid") }, 0);
    rows.prepend_server(vec![HistoryEvent {
        poll: None,
        event_id: matrix_domain::EventId::new("$a"), body: "ciao".into(), outgoing: false, redacted: false, timestamp_ms: ts,
        sender: HistorySender::new("@anna:test", None), video: None, audio: None, attachment_name: None, attachment_size: None, media: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false, thread: None, thread_root: None, reply_to: None, status: false, utd: false,
    }]);
    let d = HistoryNodes::default().project(&rows, 0, true).messages[0].description.clone();
    assert!(d.starts_with("00:30") && !d.to_uppercase().contains("UTC"), "{d}");
}
