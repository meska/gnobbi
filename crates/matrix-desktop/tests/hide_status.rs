use matrix_desktop::{DemoHistory, RoomStatusChoices, StatusChoice};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

const DAY: u64 = 86_400_000;

fn ev(id: &str, sender: &str, ts: u64, status: bool, mine: bool) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted: false,
        event_id: EventId::new(id),
        body: id.into(),
        outgoing: false,
        timestamp_ms: ts,
        sender: HistorySender::new(sender, Some(sender)),
        video: None,
        audio: None,
        styles: Vec::new(),
        mentions: Vec::new(),
        highlighted: mine,
        thread: None,
        thread_root: None,
        reply_to: None,
        status,
        utd: false,
        attachment_name: None,
        attachment_size: None,
        media: None,
    }
}

/// anna, [status], anna on the same day; [status] then bruno the next day; an invitation for me; anna again.
fn history() -> DemoHistory {
    let mut h = DemoHistory::new(RoomScope { account_id: AccountId::new(7), room_id: RoomId::new("!g:test") }, 0);
    h.prepend_server(vec![
        ev("$1", "@anna:test", 10 * DAY + 1_000, false, false),
        ev("$s1", "@bob:test", 10 * DAY + 2_000, true, false),
        ev("$2", "@anna:test", 10 * DAY + 3_000, false, false),
        ev("$s2", "@bob:test", 11 * DAY + 500, true, false),
        ev("$3", "@bruno:test", 11 * DAY + 4_000, false, false),
        ev("$inv", "@me:test", 11 * DAY + 5_000, true, true),
        ev("$4", "@anna:test", 11 * DAY + 6_000, false, false),
    ]);
    h
}

fn ids(h: &DemoHistory) -> Vec<String> {
    h.events().iter().map(|e| e.text.clone()).collect()
}

#[test]
fn hiding_drops_status_rows_but_keeps_my_invitation_and_restores_in_place() {
    let mut h = history();
    assert_eq!(ids(&h).len(), 7);
    assert!(h.set_hide_status(true));
    assert_eq!(ids(&h), ["$1", "$2", "$3", "$inv", "$4"]);
    assert_eq!(h.hidden_status_count(), 2);
    assert!(!h.set_hide_status(true), "no change, no reprojection");
    assert!(h.set_hide_status(false));
    assert_eq!(ids(&h), ["$1", "$s1", "$2", "$s2", "$3", "$inv", "$4"]);
    assert_eq!(h.hidden_status_count(), 0);
}

#[test]
fn series_and_day_separators_follow_the_visible_rows() {
    let mut h = history();
    // Visible: status row splits anna's series.
    assert!(!h.continues_series(2));
    h.set_hide_status(true);
    assert!(h.continues_series(1), "$2 follows $1 once the status row between them is hidden");
    let tz = chrono::Utc;
    let p = matrix_desktop::time_fmt::system();
    let seps: Vec<bool> = (0..h.events().len()).map(|i| h.day_separator_in(i, &tz, p, 12 * DAY).is_some()).collect();
    assert_eq!(seps, [true, false, true, false, false], "one separator per day, none orphaned on a hidden row");
}

#[test]
fn room_choice_wins_over_the_global_setting() {
    use StatusChoice::*;
    for (choice, global_show, hides) in [(Inherit, true, false), (Inherit, false, true), (Show, false, false), (Hide, true, true)] {
        assert_eq!(choice.hides(global_show), hides, "{choice:?} global_show={global_show}");
    }
}

#[test]
fn room_choices_persist_per_profile_and_ignore_bad_values() {
    let dir = std::env::temp_dir().join(format!("room-status-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(RoomStatusChoices::load(&dir).get("!a:x"), StatusChoice::Inherit, "no file");
    let mut c = RoomStatusChoices::load(&dir);
    c.set("!a:x", StatusChoice::Hide);
    c.set("!b:x", StatusChoice::Show);
    c.set("!c:x", StatusChoice::Hide);
    c.set("!c:x", StatusChoice::Inherit);
    let back = RoomStatusChoices::load(&dir);
    assert_eq!((back.get("!a:x"), back.get("!b:x"), back.get("!c:x")), (StatusChoice::Hide, StatusChoice::Show, StatusChoice::Inherit));
    std::fs::write(dir.join("room_status.json"), br#"{"!a:x":"hide","!b:x":"maybe","!c:x":3}"#).unwrap();
    assert_eq!(RoomStatusChoices::load(&dir).get("!a:x"), StatusChoice::Hide);
    std::fs::write(dir.join("room_status.json"), b"not json").unwrap();
    assert_eq!(RoomStatusChoices::load(&dir).get("!a:x"), StatusChoice::Inherit, "corrupt file");
    std::fs::remove_dir_all(dir).unwrap();
}
