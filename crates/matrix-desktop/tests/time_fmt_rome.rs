//! QA: system-zone clock around midnight and the two DST switches. Run with TZ=Europe/Rome.
use chrono::{TimeZone, Utc};
use matrix_desktop::time_fmt::clock_time;
use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, RoomId, RoomScope};

fn rome() -> bool {
    std::env::var("TZ").as_deref() == Ok("Europe/Rome")
}
fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> String {
    clock_time(Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap().timestamp_millis() as u64)
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
        let t = clock_time(ms);
        assert_eq!(t.len(), 5, "{t}");
        assert!(!t.to_uppercase().contains("UTC"));
    }
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
        sender: HistorySender::new("@anna:test", None), video: None, attachment_name: None, attachment_size: None,
        styles: Vec::new(), thread: None, thread_root: None, status: false,
    }]);
    let d = HistoryNodes::default().project(&rows, 0, true).messages[0].description.clone();
    assert!(d.starts_with("00:30") && !d.to_uppercase().contains("UTC"), "{d}");
}
