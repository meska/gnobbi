//! Last open room per account in `room-prefs.json`: save, read, isolation, validation, legacy file, clear.
use matrix_desktop::{AppDirs, RoomPrefs};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn scope(acc: u64, room: &str) -> RoomScope {
    RoomScope { account_id: AccountId::new(acc), room_id: RoomId::new(room) }
}

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("last-open-room-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs { config: root.join("cfg"), data: root.join("data"), cache: root.join("cache") }
}

#[test]
fn save_read_and_skip_redundant_write() {
    let d = dirs("save");
    let mut p = RoomPrefs::default();
    assert_eq!(p.last_open_room(AccountId::new(1)), None);
    assert!(p.set_last_open_room(&scope(1, "!a:srv")));
    assert!(!p.set_last_open_room(&scope(1, "!a:srv")), "same room: no change, no write");
    p.save(&d).unwrap();
    assert_eq!(RoomPrefs::load(&d).last_open_room(AccountId::new(1)), Some(RoomId::new("!a:srv")));
}

#[test]
fn accounts_are_isolated() {
    let d = dirs("iso");
    let mut p = RoomPrefs::default();
    p.set_last_open_room(&scope(1, "!a:srv"));
    p.set_last_open_room(&scope(2, "!b:srv"));
    p.save(&d).unwrap();
    let l = RoomPrefs::load(&d);
    assert_eq!(l.last_open_room(AccountId::new(1)), Some(RoomId::new("!a:srv")));
    assert_eq!(l.last_open_room(AccountId::new(2)), Some(RoomId::new("!b:srv")));
    assert_eq!(l.last_open_room(AccountId::new(3)), None);
}

#[test]
fn invalid_ids_are_rejected_without_panic() {
    let mut p = RoomPrefs::default();
    for bad in ["", "!", "a:srv", "!a", "!:srv", "!a:", "!a b:srv", "#alias:srv", "!a:srv\n"] {
        assert!(!p.set_last_open_room(&scope(1, bad)), "{bad:?}");
    }
    assert_eq!(p.last_open_room(AccountId::new(1)), None);
}

#[test]
fn invalid_stored_values_are_dropped_on_load() {
    let d = dirs("badfile");
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("room-prefs.json"), br#"{"last_open_room":{"1":"junk","x":"!a:s","2":"!b:s","3":5}}"#).unwrap();
    let l = RoomPrefs::load(&d);
    assert_eq!(l.last_open_room(AccountId::new(1)), None);
    assert_eq!(l.last_open_room(AccountId::new(2)), Some(RoomId::new("!b:s")));
}

#[test]
fn legacy_file_without_field_loads() {
    let d = dirs("legacy");
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("room-prefs.json"), br#"{"hide_status":["1|!a:srv"],"hide_redacted":{"1|!a:srv":true}}"#).unwrap();
    let l = RoomPrefs::load(&d);
    assert!(l.hide_status(&scope(1, "!a:srv")));
    assert_eq!(l.hide_redacted(&scope(1, "!a:srv")), Some(true));
    assert_eq!(l.last_open_room(AccountId::new(1)), None);
}

#[test]
fn clear_removes_only_that_account() {
    let d = dirs("clear");
    let mut p = RoomPrefs::default();
    p.set_last_open_room(&scope(1, "!a:srv"));
    p.set_last_open_room(&scope(2, "!b:srv"));
    assert!(p.clear_last_open_room(AccountId::new(1)));
    assert!(!p.clear_last_open_room(AccountId::new(1)));
    p.save(&d).unwrap();
    let l = RoomPrefs::load(&d);
    assert_eq!(l.last_open_room(AccountId::new(1)), None);
    assert_eq!(l.last_open_room(AccountId::new(2)), Some(RoomId::new("!b:srv")));
}
