use matrix_desktop::{AccountCatalog, AppDirs};
use std::path::PathBuf;

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-catalog-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

fn legacy_install(d: &AppDirs) {
    std::fs::create_dir_all(d.data.join("profile/account-1")).unwrap();
    std::fs::write(d.data.join("profile/account-1/session.sealed"), b"x").unwrap();
}

#[test]
fn create_save_and_read_back() {
    let d = dirs("create");
    let mut c = AccountCatalog::load(&d);
    assert!(c.accounts().is_empty() && !c.is_damaged());
    let a = c.add("@a:example.org", "https://example.org", "Casa");
    let b = c.add("@b:example.org", "https://example.org", "Lavoro");
    assert_ne!(a, b);
    c.save(&d).unwrap();
    let r = AccountCatalog::load(&d);
    assert_eq!(r, c);
    assert_eq!(r.get(b).unwrap().label, "Lavoro");
    // separate dirs per account
    r.ensure_dirs(&d, a).unwrap();
    r.ensure_dirs(&d, b).unwrap();
    assert_ne!(r.data_dir(&d, a), r.data_dir(&d, b));
    assert_ne!(r.cache_dir(&d, a), r.cache_dir(&d, b));
    assert!(r.data_dir(&d, a).unwrap().is_dir() && r.cache_dir(&d, b).unwrap().is_dir());
}

#[test]
fn ids_are_never_reused_after_reload() {
    let d = dirs("ids");
    let mut c = AccountCatalog::load(&d);
    let first = c.add("@a:x", "https://x", "");
    c.save(&d).unwrap();
    let mut c = AccountCatalog::load(&d);
    let second = c.add("@b:x", "https://x", "");
    assert_ne!(first, second);
}

#[test]
fn legacy_single_account_install_is_adopted_in_place() {
    let d = dirs("legacy");
    legacy_install(&d);
    let mut c = AccountCatalog::load(&d);
    assert!(c.migrate_legacy(&d));
    c.save(&d).unwrap();
    let r = AccountCatalog::load(&d);
    let id = r.accounts()[0].id;
    assert_eq!(r.data_dir(&d, id), Some(d.data.join("profile")));
    assert!(d.data.join("profile/account-1/session.sealed").is_file());
    let mut again = r.clone();
    assert!(!again.migrate_legacy(&d), "second start changes nothing");
    // a new account gets its own dir and the legacy one keeps its id
    let new = again.add("@n:x", "https://x", "");
    assert_eq!(
        again.data_dir(&d, new),
        Some(d.data.join("accounts").join(new.as_u64().to_string()))
    );
    assert_eq!(again.accounts()[0].id, id);
}

#[test]
fn fresh_install_and_empty_profile_are_not_migrated() {
    let d = dirs("fresh");
    let mut c = AccountCatalog::load(&d);
    assert!(!c.migrate_legacy(&d));
    std::fs::create_dir_all(d.data.join("profile")).unwrap();
    assert!(!c.migrate_legacy(&d));
}

#[test]
fn failed_write_keeps_previous_file() {
    let d = dirs("atomic");
    let mut c = AccountCatalog::load(&d);
    c.add("@a:x", "https://x", "uno");
    c.save(&d).unwrap();
    let before = std::fs::read(d.config.join("accounts.json")).unwrap();
    // a directory in the temp file's place makes the write fail before the rename
    std::fs::create_dir(d.config.join("accounts.json.tmp")).unwrap();
    c.add("@b:x", "https://x", "due");
    assert!(c.save(&d).is_err());
    assert_eq!(
        std::fs::read(d.config.join("accounts.json")).unwrap(),
        before
    );
    std::fs::remove_dir(d.config.join("accounts.json.tmp")).unwrap();
    c.save(&d).unwrap();
    assert!(!d.config.join("accounts.json.tmp").exists());
    assert_eq!(AccountCatalog::load(&d).accounts().len(), 2);
}

#[test]
fn corrupt_or_unknown_file_is_empty_untouched_and_not_overwritten() {
    for (i, body) in [
        &b"{not json"[..],
        br#"{"version":2,"next_id":1,"accounts":[]}"#,
        br#"{"version":1,"next_id":1,"accounts":[{"id":5,"dir":"p"}]}"#,
        br#"{"version":1,"next_id":9,"accounts":[{"id":1,"dir":"../out"}]}"#,
        br#"{"version":1,"next_id":9,"accounts":[{"id":1,"dir":"/abs"}]}"#,
        br#"{"version":1,"next_id":9,"accounts":[{"id":1,"dir":"a"},{"id":1,"dir":"b"}]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let d = dirs(&format!("bad{i}"));
        legacy_install(&d);
        std::fs::create_dir_all(&d.config).unwrap();
        let file: PathBuf = d.config.join("accounts.json");
        std::fs::write(&file, body).unwrap();
        let mut c = AccountCatalog::load(&d);
        assert!(c.accounts().is_empty() && c.is_damaged(), "case {i}");
        assert!(!c.migrate_legacy(&d), "case {i}");
        c.add("@a:x", "https://x", "");
        assert!(c.save(&d).is_err(), "case {i}");
        assert_eq!(std::fs::read(&file).unwrap(), body, "case {i}");
    }
}

/// The shown account must be found even when its folder is spelled differently from the
/// catalog's (symlinked HOME) or is not listed at all but its user is: otherwise a switch is refused.
#[test]
fn shown_account_is_found_by_resolved_folder_or_user() {
    let d = dirs("shown");
    let mut c = AccountCatalog::load(&d);
    let id = c.add("@a:example.org", "https://example.org", "A");
    c.ensure_dirs(&d, id).unwrap();
    let real = c.data_dir(&d, id).unwrap();
    assert_eq!(c.id_for_root(&d, &real), Some(id));
    let link = d.data.parent().unwrap().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(c.id_for_root(&d, &link), Some(id));
    assert_eq!(c.id_for_root(&d, &d.data.join("elsewhere")), None);
    assert_eq!(c.id_for_user("@a:example.org"), Some(id));
    assert_eq!(c.id_for_user("@z:example.org"), None);
}

#[test]
fn avatar_mxc_is_saved_read_back_and_optional_in_old_files() {
    let d = dirs("avatar");
    let mut c = AccountCatalog::load(&d);
    let a = c.add("@a:x", "https://x", "A");
    let b = c.add("@b:x", "https://x", "B");
    assert!(c.record_avatar(a, Some("mxc://x/a")));
    assert!(!c.record_avatar(a, Some("mxc://x/a")));
    c.save(&d).unwrap();
    let r = AccountCatalog::load(&d);
    assert_eq!(r.get(a).unwrap().avatar.as_deref(), Some("mxc://x/a"));
    assert_eq!(r.get(b).unwrap().avatar, None);
    // a catalog written before the field existed still loads
    let old = std::fs::read_to_string(d.config.join("accounts.json")).unwrap().replace("\"avatar\":\"mxc://x/a\",", "").replace("\"avatar\":null,", "");
    assert!(!old.contains("avatar"));
    std::fs::write(d.config.join("accounts.json"), old).unwrap();
    let r = AccountCatalog::load(&d);
    assert!(!r.is_damaged() && r.get(a).unwrap().avatar.is_none());
}
