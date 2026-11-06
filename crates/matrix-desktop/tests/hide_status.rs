use matrix_desktop::{AppDirs, DemoHistory, RoomPrefs};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn scope(room: &str) -> RoomScope {
    RoomScope { account_id: AccountId::new(7), room_id: RoomId::new(room) }
}

fn ev(id: &str, ts: u64, status: bool, redacted: bool) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted,
        event_id: EventId::new(id),
        body: id.into(),
        outgoing: false,
        timestamp_ms: ts,
        sender: HistorySender::new("@a:test", Some("a")),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status,
        attachment_name: None,
        attachment_size: None,
    }
}

fn texts(h: &DemoHistory) -> Vec<String> {
    h.events().iter().map(|e| e.text.clone()).collect()
}

fn history() -> DemoHistory {
    let mut h = DemoHistory::new(scope("!a:test"), 0);
    // $1 = room creation status row, $3 = redacted message.
    h.prepend_server(vec![ev("$1", 1, true, false), ev("$2", 2, false, false), ev("$3", 3, false, true), ev("$4", 4, true, false), ev("$5", 5, false, false)]);
    h
}

#[test]
fn status_rows_hide_all_including_room_creation_and_come_back_in_place() {
    let mut h = history();
    assert_eq!(texts(&h).len(), 5, "shown by default");
    assert!(h.set_hide_status(true));
    assert_eq!(texts(&h), ["$2", "$3", "$5"]);
    assert!(!h.set_hide_status(true), "idempotent");
    assert!(h.set_hide_status(false));
    assert_eq!(texts(&h), ["$1", "$2", "$3", "$4", "$5"]);
}

#[test]
fn status_and_redacted_hiding_are_independent() {
    let mut h = history();
    h.set_hide_status(true);
    h.set_hide_redacted(true);
    assert_eq!(texts(&h), ["$2", "$5"]);
    h.set_hide_status(false);
    assert_eq!(texts(&h), ["$1", "$2", "$4", "$5"], "redacted row stays hidden");
    h.set_hide_status(true);
    h.set_hide_redacted(false);
    assert_eq!(texts(&h), ["$2", "$3", "$5"], "status rows stay hidden");
}

#[test]
fn rows_arriving_while_hidden_stay_hidden() {
    let mut h = history();
    h.set_hide_status(true);
    h.prepend_server(vec![ev("$0", 0, true, false)]);
    assert_eq!(texts(&h), ["$2", "$3", "$5"]);
}

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("room-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs { config: root.join("cfg"), data: root.join("data"), cache: root.join("cache") }
}

#[test]
fn preference_is_per_room_and_survives_restart() {
    let d = dirs("per-room");
    let (a, b) = (scope("!a:test"), scope("!b:test"));
    let mut p = RoomPrefs::load(&d);
    assert!(!p.hide_status(&a), "default: shown");
    p.toggle_hide_status(&a);
    p.save(&d).unwrap();
    let p = RoomPrefs::load(&d);
    assert!(p.hide_status(&a) && !p.hide_status(&b));
    let other_account = RoomScope { account_id: AccountId::new(8), room_id: a.room_id.clone() };
    assert!(!p.hide_status(&other_account));
    let mut p = p;
    p.toggle_hide_status(&a);
    assert!(!p.hide_status(&a));
}

#[test]
fn corrupt_file_means_shown() {
    let d = dirs("corrupt");
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("room-prefs.json"), b"{\"hide_status\":\"x\"}").unwrap();
    assert!(!RoomPrefs::load(&d).hide_status(&scope("!a:test")));
}

#[test]
fn hide_redacted_is_tri_state_per_room_and_survives_restart() {
    let d = dirs("redacted");
    let (a, b) = (scope("!a:test"), scope("!b:test"));
    let mut p = RoomPrefs::load(&d);
    assert_eq!(p.hide_redacted(&a), None, "no choice yet");
    p.set_hide_redacted(&a, false);
    p.set_hide_redacted(&b, true);
    p.save(&d).unwrap();
    let p = RoomPrefs::load(&d);
    assert_eq!((p.hide_redacted(&a), p.hide_redacted(&b)), (Some(false), Some(true)));
    assert_eq!(p.hide_redacted(&scope("!c:test")), None);
}
