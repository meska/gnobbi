use matrix_desktop::{AppDirs, Preferences, Theme, app_dirs, Platform};
use matrix_desktop::directory_servers::SavedServers;
use matrix_desktop::update::startup;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("gnobbi-migration-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn dirs(&self) -> AppDirs {
        AppDirs { config: self.0.join("gnobbi"), data: self.0.join("data"), cache: self.0.join("cache") }
    }
    fn legacy(&self) -> PathBuf { self.0.join("dime/settings.json") }
    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn settings(d: &AppDirs) -> PathBuf { d.config.join("settings.json") }
fn read(d: &AppDirs) -> Value { serde_json::from_slice(&std::fs::read(settings(d)).unwrap()).unwrap() }

#[test]
fn schema_zero_upgrade_preserves_choices_unknown_and_rollback_state() {
    let f = Fixture::new("upgrade");
    let d = f.dirs();
    Fixture::write(&settings(&d), br#"{"notifications":true,"theme":"night","read_receipts":false,"future":{"x":9},"max_build_seen":812,"bad_builds":[7]}"#);
    assert!(Preferences::load(&d).notifications);
    Preferences::update(&d, |p| p.close_to_tray = true).unwrap();
    let p = Preferences::load(&d);
    assert!(p.notifications && p.close_to_tray);
    assert_eq!(p.theme, Some(Theme::Night));
    assert!(!p.read_receipts);
    let value = read(&d);
    assert_eq!(value["settings_version"], 1);
    assert_eq!(value["future"], json!({"x":9}));
    assert_eq!(value["max_build_seen"], 812);
    assert_eq!(value["bad_builds"], json!([7]));
}

#[test]
fn schema_one_missing_field_only_defaults_that_field() {
    let f = Fixture::new("missing-field");
    let d = f.dirs();
    Fixture::write(&settings(&d), br#"{"settings_version":1,"theme":"night","read_receipts":false,"future":42}"#);
    let p = Preferences::load(&d);
    assert!(!p.notifications);
    assert_eq!(p.theme, Some(Theme::Night));
    assert!(!p.read_receipts);
    Preferences::update(&d, |p| p.notifications = true).unwrap();
    assert!(Preferences::load(&d).notifications);
    assert_eq!(read(&d)["future"], 42);
}

#[test]
fn missing_canonical_imports_legacy_without_touching_source() {
    let f = Fixture::new("legacy");
    let d = f.dirs();
    let original = br#"{"notifications":true,"theme":"night","future":{"x":9}}"#;
    Fixture::write(&f.legacy(), original);
    let p = Preferences::load(&d);
    assert!(p.notifications);
    assert_eq!(p.theme, Some(Theme::Night));
    assert_eq!(std::fs::read(f.legacy()).unwrap(), original);
    let v = read(&d);
    assert_eq!(v["settings_version"], 1);
    assert_eq!(v["future"], json!({"x":9}));
}

#[test]
fn canonical_precedes_legacy_and_absent_files_stay_absent_until_write() {
    let f = Fixture::new("precedence");
    let d = f.dirs();
    assert!(!Preferences::load(&d).notifications);
    assert!(!settings(&d).exists());
    Preferences::update(&d, |p| p.notifications = false).unwrap();
    Fixture::write(&f.legacy(), br#"{"notifications":true,"obsolete":99}"#);
    assert!(!Preferences::load(&d).notifications);
    Preferences::update(&d, |p| p.debug = true).unwrap();
    assert!(read(&d).get("obsolete").is_none());
}

#[test]
fn rejected_files_survive_all_shared_writers_byte_for_byte() {
    let f = Fixture::new("reject");
    let d = f.dirs();
    let mut failures = Vec::new();
    for (case, body) in [("corrupt", r#"{invalid SECRET"#), ("array", "[]"), ("future", r#"{"settings_version":2,"notifications":true}"#), ("string", r#"{"settings_version":"1","notifications":true}"#), ("negative", r#"{"settings_version":-1}"#)] {
        for writer in ["preferences", "servers", "startup"] {
            Fixture::write(&settings(&d), body.as_bytes());
            let result = match writer {
                "preferences" => Preferences::update(&d, |p| p.notifications = false).map(|_| ()),
                "servers" => { let mut s = SavedServers::default(); s.add(1, "example.org"); s.save(&d) },
                _ => startup::add_bad_build(&d, 42),
            };
            if result.is_ok() || std::fs::read(settings(&d)).unwrap() != body.as_bytes() {
                failures.push(format!("{case}/{writer}"));
            }
        }
    }
    assert!(failures.is_empty(), "writers accepted or changed invalid settings: {failures:?}");
}

#[test]
fn unreadable_legacy_never_creates_a_new_default_file() {
    let f = Fixture::new("bad-legacy");
    let d = f.dirs();
    let original = b"{broken legacy";
    Fixture::write(&f.legacy(), original);
    assert!(!Preferences::load(&d).notifications);
    assert!(Preferences::update(&d, |p| p.notifications = true).is_err());
    assert_eq!(std::fs::read(f.legacy()).unwrap(), original);
    assert!(!settings(&d).exists());
}

#[test]
fn settings_directory_is_not_replaced() {
    let f = Fixture::new("directory");
    let d = f.dirs();
    std::fs::create_dir_all(settings(&d)).unwrap();
    let marker = settings(&d).join("marker");
    std::fs::write(&marker, b"untouched").unwrap();
    assert!(Preferences::update(&d, |p| p.notifications = true).is_err());
    assert_eq!(std::fs::read(marker).unwrap(), b"untouched");
}

#[test]
fn server_keys_for_two_persistent_ids_are_distinct_and_stable() {
    let f = Fixture::new("accounts");
    let d = f.dirs();
    let mut servers = SavedServers::default();
    servers.add(7, "one.example");
    servers.add(42, "two.example");
    servers.save(&d).unwrap();
    let restored = SavedServers::load(&d);
    assert_eq!(restored.for_account(7), ["one.example"]);
    assert_eq!(restored.for_account(42), ["two.example"]);
    Preferences::update(&d, |p| p.notifications = true).unwrap();
    let v = read(&d);
    assert_eq!(v["directory_servers"]["7"], json!(["one.example"]));
    assert_eq!(v["directory_servers"]["42"], json!(["two.example"]));
    assert_eq!(v["notifications"], true);
}

#[test]
fn config_path_is_independent_of_account_appimage_and_version() {
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let resolve = |account: &str, appimage: &str, version: &str| {
            app_dirs(platform, |key| match key {
                "HOME" => Some("/home/test".into()),
                "APPDATA" => Some("C:\\Users\\test\\AppData\\Roaming".into()),
                "LOCALAPPDATA" => Some("C:\\Users\\test\\AppData\\Local".into()),
                "ACCOUNT" => Some(account.into()), "APPIMAGE" => Some(appimage.into()), "VERSION" => Some(version.into()),
                _ => None,
            }).unwrap().config.join("settings.json")
        };
        assert_eq!(resolve("7", "/opt/a", "1"), resolve("42", "/other/b", "2"));
    }
}

#[cfg(unix)]
#[test]
fn eacces_settings_are_neither_overwritten_nor_hidden_by_a_write() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("eacces");
    let d = f.dirs();
    let path = settings(&d);
    Fixture::write(&path, br#"{"notifications":true}"#);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() { return; } // running as root: EACCES not reproducible
    let _ = Preferences::load(&d);
    assert!(Preferences::update(&d, |p| p.notifications = false).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), br#"{"notifications":true}"#);
}

#[test]
fn invalid_update_field_does_not_reset_notifications_on_read() {
    let f = Fixture::new("badupdate");
    let d = f.dirs();
    Fixture::write(&settings(&d), br#"{"settings_version":1,"notifications":true,"update_check_enabled":"x"}"#);
    assert!(Preferences::load(&d).notifications);
}
