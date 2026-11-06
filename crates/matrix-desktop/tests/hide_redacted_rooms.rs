//! Per-room «messaggi eliminati» choice: resolution room → global, isolation between rooms,
//! persistence together with the status-row preference, and the resulting projection.
use matrix_desktop::{AppDirs, DemoHistory, Preferences, RoomPrefs};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender, ThreadInfo};

fn scope(room: &str) -> RoomScope {
    RoomScope { account_id: AccountId::new(7), room_id: RoomId::new(room) }
}

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("hide-redacted-rooms-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs { config: root.join("cfg"), data: root.join("data"), cache: root.join("cache") }
}

/// Same expression as main.rs (room choice, else global); kept in sync by `resolve_matches_main`.
fn resolved(rooms: &RoomPrefs, global: &Preferences, s: &RoomScope) -> bool {
    rooms.hide_redacted(s).unwrap_or(global.hide_redacted)
}

fn global(hide: bool) -> Preferences {
    Preferences { hide_redacted: hide, ..Preferences::default() }
}

fn ev(id: &str, ts: u64, redacted: bool, status: bool) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted,
        event_id: EventId::new(id),
        body: if redacted { "[Messaggio eliminato]".into() } else { id.into() },
        outgoing: false,
        timestamp_ms: ts,
        sender: HistorySender::new("@a:test", Some("a")),
        video: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None,
        thread_root: None,
        status,
        attachment_name: None,
        attachment_size: None,
    }
}

fn history(room: &str, hide: bool) -> DemoHistory {
    let mut h = DemoHistory::new(scope(room), 0);
    h.set_hide_redacted(hide);
    h.prepend_server(vec![ev("$1", 1, false, false), ev("$2", 2, true, false), ev("$3", 3, false, false), ev("$s", 4, false, true)]);
    h
}

fn texts(h: &DemoHistory) -> Vec<String> {
    h.events().iter().map(|e| e.text.clone()).collect()
}

#[test]
fn resolve_matches_main() {
    let main = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    assert!(main.contains("self.room_prefs.hide_redacted(&self.active_scope).unwrap_or(self.prefs.hide_redacted)"));
}

#[test]
fn default_hides_deleted_messages_for_a_room_without_a_choice() {
    let (rooms, a) = (RoomPrefs::default(), scope("!a:test"));
    assert!(resolved(&rooms, &Preferences::default(), &a));
    let h = history("!a:test", resolved(&rooms, &Preferences::default(), &a));
    assert_eq!(texts(&h), ["$1", "$3", "$s"]);
}

#[test]
fn room_a_shows_while_room_b_without_choice_hides_and_neither_leaks() {
    let (a, b) = (scope("!a:test"), scope("!b:test"));
    let mut rooms = RoomPrefs::default();
    rooms.set_hide_redacted(&a, false);
    let g = Preferences::default();
    assert!(!resolved(&rooms, &g, &a));
    assert!(resolved(&rooms, &g, &b));
    assert_eq!(texts(&history("!a:test", resolved(&rooms, &g, &a))).len(), 4);
    assert_eq!(texts(&history("!b:test", resolved(&rooms, &g, &b))).len(), 3);
    // B explicit «nascondi» does not touch A, and A keeps its own choice.
    rooms.set_hide_redacted(&b, true);
    assert_eq!((rooms.hide_redacted(&a), rooms.hide_redacted(&b)), (Some(false), Some(true)));
    // Same room id on another account is a different room.
    let other = RoomScope { account_id: AccountId::new(8), room_id: a.room_id.clone() };
    assert_eq!(rooms.hide_redacted(&other), None);
}

#[test]
fn global_applies_only_where_the_room_has_no_choice() {
    let (a, b, c) = (scope("!a:test"), scope("!b:test"), scope("!c:test"));
    let mut rooms = RoomPrefs::default();
    rooms.set_hide_redacted(&b, true);
    rooms.set_hide_redacted(&c, false);
    let show = global(false);
    assert!(!resolved(&rooms, &show, &a), "global show + no choice → show");
    assert!(resolved(&rooms, &show, &b), "room hide beats global show");
    assert!(!resolved(&rooms, &show, &c));
    let hide = global(true);
    assert!(resolved(&rooms, &hide, &a));
    assert!(!resolved(&rooms, &hide, &c), "room show beats global hide");
}

#[test]
fn choices_survive_restart_together_with_status_preference() {
    let d = dirs("persist");
    let (a, b, c) = (scope("!a:test"), scope("!b:test"), scope("!c:test"));
    let mut p = RoomPrefs::load(&d);
    p.set_hide_redacted(&a, false);
    p.set_hide_redacted(&b, true);
    p.toggle_hide_status(&a);
    p.save(&d).unwrap();
    let mut p = RoomPrefs::load(&d);
    assert_eq!((p.hide_redacted(&a), p.hide_redacted(&b), p.hide_redacted(&c)), (Some(false), Some(true), None));
    assert!(p.hide_status(&a) && !p.hide_status(&b));
    // Changing one preference and saving again must not drop the other.
    p.toggle_hide_status(&a);
    p.save(&d).unwrap();
    let p = RoomPrefs::load(&d);
    assert!(!p.hide_status(&a));
    assert_eq!(p.hide_redacted(&a), Some(false));
    assert_eq!(p, {
        let mut q = RoomPrefs::default();
        q.set_hide_redacted(&a, false);
        q.set_hide_redacted(&b, true);
        q
    });
}

#[test]
fn malformed_room_entries_are_ignored_not_fatal() {
    let d = dirs("malformed");
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("room-prefs.json"), br#"{"hide_redacted":{"7|!a:test":"yes","7|!b:test":true,"7|!c:test":null},"hide_status":["7|!a:test"]}"#).unwrap();
    let p = RoomPrefs::load(&d);
    assert_eq!(p.hide_redacted(&scope("!a:test")), None);
    assert_eq!(p.hide_redacted(&scope("!b:test")), Some(true));
    assert_eq!(p.hide_redacted(&scope("!c:test")), None);
    assert!(p.hide_status(&scope("!a:test")), "valid sibling key still read");
    std::fs::write(d.config.join("room-prefs.json"), br#"{"hide_redacted":[1,2]}"#).unwrap();
    assert_eq!(RoomPrefs::load(&d).hide_redacted(&scope("!b:test")), None);
}

#[test]
fn hot_toggle_keeps_each_room_projection_separate() {
    let (mut a, mut b) = (history("!a:test", true), history("!b:test", true));
    assert!(a.set_hide_redacted(false));
    assert_eq!((a.events().len(), b.events().len()), (4, 3));
    assert!(b.set_hide_redacted(false) && a.set_hide_redacted(true));
    assert_eq!((a.events().len(), b.events().len()), (3, 4));
}

#[test]
fn reply_to_a_deleted_message_stays_visible_when_the_parent_is_hidden() {
    let mut reply = ev("$r", 3, false, false);
    reply.thread_root = Some(EventId::new("$2"));
    let mut h = DemoHistory::new(scope("!a:test"), 0);
    h.set_hide_redacted(true);
    h.prepend_server(vec![ev("$2", 2, true, false), reply]);
    assert_eq!(texts(&h), ["$r"]);
    h.set_hide_redacted(false);
    assert_eq!(texts(&h), ["[Messaggio eliminato]", "$r"]);
}

#[test]
fn deleted_thread_root_with_replies_stays_visible() {
    let mut root = ev("$2", 2, true, false);
    root.thread = Some(ThreadInfo { num_replies: 1, latest_sender: None, unread: 0 });
    let mut h = DemoHistory::new(scope("!a:test"), 0);
    h.set_hide_redacted(true);
    h.prepend_server(vec![root]);
    assert_eq!(texts(&h), ["[Messaggio eliminato]"]);
}

#[test]
fn deleted_text_never_reaches_projection_when_hidden_even_if_redacted_live() {
    let mut h = history("!a:test", true);
    h.refresh_server(vec![ev("$3", 3, true, false)]);
    assert_eq!(texts(&h), ["$1", "$s"]);
    assert!(!texts(&h).iter().any(|t| t.contains("eliminato")));
}
