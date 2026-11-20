use matrix_desktop::{Accent, AppDirs, AutoNight, FontChoice, Preferences, Theme};
use std::path::PathBuf;

/// A child spawned while another test holds a settings lock inherits that fd until exec, so the
/// lock reads as held (macOS and Linux). Every test touching settings holds this shared (`no_spawn`),
/// the spawning test exclusively (`spawn_gate`, polled: a queued writer would deadlock a nested reader).
static SPAWN_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());

fn no_spawn() -> std::sync::RwLockReadGuard<'static, ()> {
    SPAWN_GATE.read().unwrap_or_else(|e| e.into_inner())
}

fn spawn_gate() -> std::sync::RwLockWriteGuard<'static, ()> {
    loop {
        match SPAWN_GATE.try_write() {
            Ok(g) => return g,
            Err(std::sync::TryLockError::Poisoned(e)) => return e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

fn dirs(root: &std::path::Path) -> AppDirs {
    AppDirs {
        config: root.join("cfg"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("dime-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn missing_file_gives_defaults() {
    assert!(!Preferences::default().close_to_tray, "closing keeps quitting unless asked");
    let _gate = no_spawn();
    let d = dirs(&tmp("missing"));
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn notifications_default_off_and_survive_restart() {
    let _gate = no_spawn();
    assert!(!Preferences::default().notifications);
    let d = dirs(&tmp("notif"));
    assert!(!Preferences::load(&d).notifications, "missing file");
    Preferences::update(&d, |p| p.notifications = true).unwrap();
    assert!(Preferences::load(&d).notifications, "written then re-read");
    Preferences::update(&d, |p| p.notifications = false).unwrap();
    assert!(!Preferences::load(&d).notifications);
}

#[test]
fn notifications_corrupt_file_or_value_is_off() {
    let _gate = no_spawn();
    let d = dirs(&tmp("notif-bad"));
    std::fs::create_dir_all(&d.config).unwrap();
    for body in ["{not json", "[]", r#"{"notifications":"yes"}"#, r#"{"notifications":1}"#] {
        std::fs::write(d.config.join("settings.json"), body).unwrap();
        assert!(!Preferences::load(&d).notifications, "{body}");
    }
}

#[test]
fn roundtrip_keeps_values() {
    let _gate = no_spawn();
    let d = dirs(&tmp("roundtrip"));
    let p = Preferences {
        theme: Some(Theme::Night),
        accent: Some(Accent::Purple),
        debug: true,
        read_receipts: false,
        auto_night: AutoNight::Schedule,
        font: FontChoice::Serif,
        icon: matrix_desktop::IconChoice::Monster,
        hide_redacted: false,
        show_status: false,
        close_to_tray: true,
        notifications: true,
        desktop_entry_dont_ask: true,
        message_preview: false,
        update_check_enabled: false,
        last_check: Some(1_700_000_000),
        max_build_seen: 812,
        no_recovery_offer_accounts: Default::default(),
        bad_builds: [7, 3].into(),
        update_base: Some("http://lan/dl/".into()),
        list_width: Some(412),
        thread_width: Some(500),
        count_preview_accounts: Default::default(),
        language: matrix_desktop::i18n::Preference::Italian,
        text_size: matrix_desktop::TextSize::Large,
        density: matrix_desktop::Density::Compact,
    };
    p.save(&d).unwrap();
    assert_eq!(Preferences::load(&d), p);
    // «Doppio clic»: back to the default clears the saved width; other settings stay.
    Preferences::update(&d, |p| p.list_width = None).unwrap();
    let back = Preferences::load(&d);
    assert_eq!((back.list_width, back.font), (None, FontChoice::Serif));
}

#[test]
fn language_choice_persists_and_applies() {
    use matrix_desktop::i18n::{self, Preference};
    let _gate = no_spawn();
    let d = dirs(&tmp("language"));
    assert_eq!(Preferences::load(&d).language, Preference::Auto);
    Preferences::update(&d, |p| p.language = Preference::Italian).unwrap();
    let saved = Preferences::load(&d).language;
    assert_eq!(saved, Preference::Italian);
    assert_eq!(i18n::apply(saved, None), "it");
    assert_eq!(i18n::t("Settings"), "Impostazioni");
    assert_eq!(i18n::apply(Preference::English, None), "en");
    assert_eq!(i18n::t("Settings"), "Settings");
}

#[test]
fn transaction_excludes_a_second_thread_but_not_another_directory() {
    use std::sync::mpsc;
    use std::time::Duration;
    let _gate = no_spawn();
    let d = dirs(&tmp("thread-lock"));
    let other = dirs(&tmp("thread-other"));
    let thread_dirs = d.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        Preferences::update(&thread_dirs, |p| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            p.max_build_seen = 19;
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let contended = Preferences::update(&d, |_| {}).unwrap_err().kind();
    let independent = Preferences::update(&other, |p| p.debug = true);
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    assert_eq!(contended, std::io::ErrorKind::WouldBlock);
    assert!(independent.unwrap().debug);
    assert_eq!(Preferences::load(&d).max_build_seen, 19);
}

#[test]
fn lock_process_helper() {
    use std::io::{Read, Write};
    let Some(path) = std::env::var_os("GNOBBI_TEST_LOCK_PATH") else { return };
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).unwrap();
    lock.try_lock().unwrap();
    println!("LOCK_HELD");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte);
}

#[test]
fn process_death_releases_the_lock() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::time::Duration;
    let _gate = spawn_gate();
    let d = dirs(&tmp("process-lock"));
    std::fs::create_dir_all(&d.config).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_process_helper", "--nocapture"])
        .env("GNOBBI_TEST_LOCK_PATH", d.config.join("settings.json.lock"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if line.contains("LOCK_HELD") { let _ = tx.send(()); break; }
        }
    });
    if rx.recv_timeout(Duration::from_secs(10)).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child did not acquire lock");
    }
    assert_eq!(Preferences::update(&d, |_| {}).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    child.kill().unwrap();
    child.wait().unwrap();
    Preferences::update(&d, |p| p.debug = true).unwrap();
}

#[cfg(unix)]
#[test]
fn settings_and_lock_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let _gate = no_spawn();
    let d = dirs(&tmp("symlinks"));
    std::fs::create_dir_all(&d.config).unwrap();
    let victim = d.config.join("victim");
    std::fs::write(&victim, b"{}").unwrap();
    for name in ["settings.json", "settings.json.lock"] {
        let path = d.config.join(name);
        symlink(&victim, &path).unwrap();
        assert!(Preferences::update(&d, |p| p.debug = true).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"{}");
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn settings_and_lock_hard_links_are_rejected() {
    let _gate = no_spawn();
    let d = dirs(&tmp("hardlinks"));
    std::fs::create_dir_all(&d.config).unwrap();
    let victim = d.config.join("victim");
    std::fs::write(&victim, b"{}").unwrap();
    for name in ["settings.json", "settings.json.lock"] {
        let path = d.config.join(name);
        std::fs::hard_link(&victim, &path).unwrap();
        assert!(Preferences::update(&d, |p| p.debug = true).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"{}");
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn directory_servers_and_outcome_survive_preference_update() {
    use matrix_desktop::directory_servers::SavedServers;
    let _gate = no_spawn();
    let d = dirs(&tmp("shared-stores"));
    std::fs::create_dir_all(&d.config).unwrap();
    let path = d.config.join("settings.json");
    std::fs::write(&path, br#"{"update_outcome":{"result":"updated","build":31},"future":{"x":1}}"#).unwrap();
    let mut servers = SavedServers::default();
    servers.add(1, "matrix.example");
    servers.save(&d).unwrap();
    Preferences::update(&d, |p| p.debug = true).unwrap();
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["update_outcome"]["build"], 31);
    assert_eq!(data["future"]["x"], 1);
    assert_eq!(matrix_desktop::update::startup::update_outcome(&d), Some(("updated".into(), 31)));
    assert_eq!(SavedServers::load(&d).for_account(1), ["matrix.example"]);
}

#[test]
fn non_object_settings_are_never_overwritten() {
    let _gate = no_spawn();
    let d = dirs(&tmp("non-object"));
    std::fs::create_dir_all(&d.config).unwrap();
    let path = d.config.join("settings.json");
    std::fs::write(&path, b"[]").unwrap();
    assert!(Preferences::update(&d, |_| {}).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"[]");
}

#[test]
fn update_reads_current_values_and_preserves_unknown_keys() {
    let _gate = no_spawn();
    let d = dirs(&tmp("update-current"));
    std::fs::create_dir_all(&d.config).unwrap();
    let path = d.config.join("settings.json");
    std::fs::write(&path, br#"{"future":{"value":9},"max_build_seen":42}"#).unwrap();
    let result = Preferences::update(&d, |p| p.debug = true).unwrap();
    assert_eq!(result.max_build_seen, 42);
    assert!(result.debug);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(json["future"]["value"], 9);
}

#[test]
fn save_returns_would_block_if_another_handle_holds_the_lock() {
    let _gate = no_spawn();
    let d = dirs(&tmp("contended"));
    std::fs::create_dir_all(&d.config).unwrap();
    let lock = std::fs::File::create(d.config.join("settings.json.lock")).unwrap();
    lock.try_lock().unwrap();
    assert_eq!(Preferences::default().save(&d).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    drop(lock);
    Preferences::default().save(&d).unwrap();
}

#[test]
fn corrupt_settings_are_not_overwritten_by_save() {
    let _gate = no_spawn();
    let d = dirs(&tmp("corrupt-save"));
    std::fs::create_dir_all(&d.config).unwrap();
    let path = d.config.join("settings.json");
    std::fs::write(&path, b"{broken").unwrap();
    assert!(Preferences::default().save(&d).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"{broken");
}

#[test]
fn corrupt_file_gives_defaults() {
    let _gate = no_spawn();
    let d = dirs(&tmp("corrupt"));
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("settings.json"), b"{not json").unwrap();
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn unknown_keys_ignored_and_invalid_values_default_per_field() {
    let _gate = no_spawn();
    let d = dirs(&tmp("unknown"));
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(
        d.config.join("settings.json"),
        br#"{"future":1,"theme":"neon","debug":true}"#,
    )
    .unwrap();
    let p = Preferences::load(&d);
    assert_eq!(p.theme, None);
    assert!(p.debug);
}

#[cfg(unix)]
#[test]
fn save_creates_private_dir_and_file() {
    use std::os::unix::fs::PermissionsExt;
    let _gate = no_spawn();
    let d = dirs(&tmp("perm"));
    Preferences::default().save(&d).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&d.config), 0o700);
    assert_eq!(mode(&d.config.join("settings.json")), 0o600);
}

#[test]
fn thumbnail_cache_dir_is_under_cache_and_created() {
    let _gate = no_spawn();
    let d = dirs(&tmp("thumbs"));
    let t = d.thumbnails().unwrap();
    assert_eq!(t, d.cache.join("thumbnails"));
    assert!(t.is_dir());
}

#[test]
fn read_receipts_default_on_and_invalid_value_falls_back_to_on() {
    assert!(Preferences::default().read_receipts);
    let _gate = no_spawn();
    let d = dirs(&tmp("receipts"));
    std::fs::create_dir_all(&d.config).unwrap();
    for body in [
        &br#"{"read_receipts":"no"}"#[..],
        br#"{"read_receipts":null}"#,
        br#"{"debug":true}"#,
    ] {
        std::fs::write(d.config.join("settings.json"), body).unwrap();
        assert!(
            Preferences::load(&d).read_receipts,
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    std::fs::write(
        d.config.join("settings.json"),
        br#"{"read_receipts":false}"#,
    )
    .unwrap();
    assert!(!Preferences::load(&d).read_receipts);
}

#[test]
fn theme_saved_values_map_to_the_four_themes_and_unknown_follows_system() {
    let _gate = no_spawn();
    let d = dirs(&tmp("badtheme"));
    std::fs::create_dir_all(&d.config).unwrap();
    for (saved, want) in [
        (r#""light""#, Some(Theme::Classic)),
        (r#""dark""#, Some(Theme::Dark)),
        (r#""day""#, Some(Theme::Day)),
        (r#""night""#, Some(Theme::Night)),
        (r#""system""#, None),
        (r#""sepia""#, None),
        (r#""""#, None),
        (r#""Light""#, None),
        ("42", None),
    ] {
        std::fs::write(
            d.config.join("settings.json"),
            format!(r#"{{"theme":{saved}}}"#),
        )
        .unwrap();
        assert_eq!(Preferences::load(&d).theme, want, "{saved}");
    }
    for t in Theme::ALL {
        Preferences {
            theme: Some(t),
            ..Preferences::default()
        }
        .save(&d)
        .unwrap();
        assert_eq!(Preferences::load(&d).theme, Some(t));
    }
    assert!(
        Theme::Dark.is_dark()
            && Theme::Night.is_dark()
            && !Theme::Classic.is_dark()
            && !Theme::Day.is_dark()
    );
}

#[test]
fn accent_roundtrips_system_included_and_unknown_means_blue() {
    let _gate = no_spawn();
    let d = dirs(&tmp("accent"));
    for accent in Accent::ALL.map(Some).into_iter().chain([None]) {
        Preferences { accent, ..Preferences::default() }.save(&d).unwrap();
        assert_eq!(Preferences::load(&d).accent, accent);
    }
    std::fs::write(d.config.join("settings.json"), r#"{"accent":"mauve"}"#).unwrap();
    assert_eq!(Preferences::load(&d).accent, Some(Accent::Blue));
    std::fs::write(d.config.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
    assert_eq!(Preferences::load(&d).accent, Some(Accent::Blue));
}

#[test]
fn os_accent_names_map_onto_the_palette() {
    assert_eq!(Accent::parse("teal"), Some(Accent::Teal));
    assert_eq!(Accent::parse("yellow"), Some(Accent::Orange));
    assert_eq!(Accent::parse("slate"), Some(Accent::Blue));
    assert_eq!(Accent::parse("nope"), None);
}

#[test]
fn night_hours_are_20_to_07() {
    let night: Vec<u32> = (0..24).filter(|h| matrix_desktop::is_night_hour(*h)).collect();
    assert_eq!(night, [0, 1, 2, 3, 4, 5, 6, 20, 21, 22, 23]);
}

#[test]
fn auto_night_picks_night_by_clock_or_system_else_the_saved_theme() {
    let p = |auto_night, theme| Preferences { auto_night, theme, ..Preferences::default() };
    // Off: never night, the OS only decides when nothing is saved.
    assert_eq!(p(AutoNight::Off, Some(Theme::Day)).effective_theme(true, 23), Theme::Day);
    assert_eq!(p(AutoNight::Off, None).effective_theme(true, 12), Theme::Dark);
    assert_eq!(p(AutoNight::Off, None).effective_theme(false, 12), Theme::Classic);
    // Schedule: clock only, the OS appearance is ignored.
    let s = p(AutoNight::Schedule, Some(Theme::Day));
    assert_eq!(s.effective_theme(false, 20), Theme::Night);
    assert_eq!(s.effective_theme(false, 6), Theme::Night);
    assert_eq!(s.effective_theme(true, 7), Theme::Day);
    assert_eq!(s.effective_theme(true, 19), Theme::Day);
    // System: OS only, the clock is ignored.
    let y = p(AutoNight::System, Some(Theme::Day));
    assert_eq!(y.effective_theme(true, 12), Theme::Night);
    assert_eq!(y.effective_theme(false, 23), Theme::Day);
}

#[test]
fn auto_night_and_font_default_safely_when_invalid_or_wrong_type() {
    let _gate = no_spawn();
    let d = dirs(&tmp("autofont"));
    std::fs::create_dir_all(&d.config).unwrap();
    for body in [
        r#"{"auto_night":"sunset","font":"comic"}"#,
        r#"{"auto_night":3,"font":null}"#,
        r#"{"auto_night":["schedule"],"font":{"a":1}}"#,
        r#"[1,2]"#,
    ] {
        std::fs::write(d.config.join("settings.json"), body).unwrap();
        let p = Preferences::load(&d);
        assert_eq!((p.auto_night, p.font), (AutoNight::Off, FontChoice::Sans), "{body}");
    }
    // Old files without the new keys keep working.
    std::fs::write(d.config.join("settings.json"), r#"{"theme":"dark","debug":true}"#).unwrap();
    let p = Preferences::load(&d);
    assert_eq!((p.theme, p.debug, p.auto_night, p.font), (Some(Theme::Dark), true, AutoNight::Off, FontChoice::Sans));
}

#[test]
fn restart_restores_theme_accent_auto_night_and_font() {
    let _gate = no_spawn();
    let d = dirs(&tmp("restart"));
    for auto_night in AutoNight::ALL {
        for font in FontChoice::ALL {
            let saved = Preferences {
                theme: Some(Theme::Day),
                accent: Some(Accent::Teal),
                auto_night,
                font,
                ..Preferences::default()
            };
            saved.save(&d).unwrap();
            // A new process reads the file and applies it before the first paint.
            let loaded = Preferences::load(&d);
            assert_eq!(loaded, saved);
            matrix_desktop::set_font(loaded.font);
            assert_eq!(matrix_desktop::ui_font(), font.family());
        }
    }
    matrix_desktop::set_font(FontChoice::Sans);
}

#[test]
fn unreadable_or_hostile_settings_files_load_as_defaults_without_panicking() {
    let _gate = no_spawn();
    let d = dirs(&tmp("hostile"));
    std::fs::create_dir_all(&d.config).unwrap();
    let file = d.config.join("settings.json");
    let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
    let big = format!(r#"{{"theme":"{}"}}"#, "x".repeat(4 << 20));
    let cases: [&[u8]; 9] = [
        b"", b"\xff\xfe\x00{", b"\0\0\0", b"null", b"\"dark\"", b"{\"theme\":", b"{\"theme\":\"dark\"", deep.as_bytes(), big.as_bytes(),
    ];
    for body in cases {
        std::fs::write(&file, body).unwrap();
        assert_eq!(Preferences::load(&d), Preferences::default(), "{} bytes", body.len());
    }
    // A directory where the file should be (unreadable) is also just defaults.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn icon_defaults_to_minimal_mark_saves_and_invalid_value_falls_back() {
    use matrix_desktop::IconChoice;
    assert_eq!(Preferences::default().icon, IconChoice::MinimalMark);
    assert_eq!(
        IconChoice::ALL.map(IconChoice::label),
        ["Minimal mark", "Gnome", "Little monster"]
    );
    let _gate = no_spawn();
    let d = dirs(&tmp("icon"));
    for icon in IconChoice::ALL {
        let p = Preferences { icon, ..Preferences::default() };
        p.save(&d).unwrap();
        assert_eq!(Preferences::load(&d).icon, icon);
    }
    for bad in [r#""drago""#, r#""""#, "7", "null", r#""Gnomo""#] {
        std::fs::write(d.config.join("settings.json"), format!(r#"{{"icon":{bad}}}"#)).unwrap();
        assert_eq!(Preferences::load(&d).icon, IconChoice::MinimalMark, "{bad}");
    }
}

#[test]
fn hide_redacted_defaults_on_invalid_falls_back_and_persists() {
    assert!(Preferences::default().hide_redacted);
    let _gate = no_spawn();
    let d = dirs(&tmp("hide-redacted"));
    std::fs::create_dir_all(&d.config).unwrap();
    for body in [
        &br#"{"hide_deleted":"yes"}"#[..],
        br#"{"hide_deleted":null}"#,
        br#"{"debug":true}"#,
        // Old key (always saved false): ignored, reset once to the new default.
        br#"{"hide_redacted":false}"#,
    ] {
        std::fs::write(d.config.join("settings.json"), body).unwrap();
        assert!(Preferences::load(&d).hide_redacted);
    }
    let p = Preferences {
        hide_redacted: false,
        ..Preferences::default()
    };
    p.save(&d).unwrap();
    assert!(!Preferences::load(&d).hide_redacted, "survives a restart");
}

#[test]
fn update_fields_old_file_defaults_and_full_roundtrip() {
    let _gate = no_spawn();
    let d = dirs(&tmp("update_fields"));
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("settings.json"), r#"{"theme":"dark","debug":true}"#).unwrap();
    let old = Preferences::load(&d);
    assert!(old.update_check_enabled && old.debug);
    assert_eq!((old.last_check, old.max_build_seen, old.update_base), (None, 0, None));
    assert!(old.bad_builds.is_empty());
    let full = Preferences {
        update_check_enabled: false,
        last_check: Some(1_700_000_000),
        max_build_seen: 812,
        bad_builds: [7, 3, 7].into(),
        update_base: Some("http://lan/dl/".into()),
        ..Preferences::default()
    };
    full.save(&d).unwrap();
    assert_eq!(Preferences::load(&d), full);
    assert_eq!(full.bad_builds.len(), 2);
}

/// Saved list width: survives a restart as is; a narrower window only clamps what is painted, the saved value stays.
#[test]
fn list_width_survives_restart_and_odd_values_fall_back_to_default() {
    let _gate = no_spawn();
    let d = dirs(&tmp("list-width-restart"));
    assert_eq!(Preferences::load(&d).list_width, None, "first run: default");
    Preferences::update(&d, |p| p.list_width = Some(480)).unwrap();
    assert_eq!(Preferences::load(&d).list_width, Some(480), "restart 1");
    assert_eq!(Preferences::load(&d).list_width, Some(480), "restart 2: loading does not rewrite or drop it");
    Preferences::update(&d, |p| p.font = FontChoice::Serif).unwrap();
    assert_eq!(Preferences::load(&d).list_width, Some(480), "other settings do not touch the width");
    // Written by hand / by another version: anything that is not a plain non-negative integer is «not saved».
    let file = d.config.join("settings.json");
    for (raw, want) in [("\"wide\"", None), ("-5", None), ("320.5", None), ("null", None), ("99999999999", None), ("0", Some(0)), ("1000000", Some(1_000_000))] {
        std::fs::write(&file, format!("{{\"list_width\": {raw}}}")).unwrap();
        assert_eq!(Preferences::load(&d).list_width, want, "list_width = {raw}");
    }
}

#[test]
fn message_preview_is_on_by_default_also_for_files_saved_before_it_existed() {
    let _gate = no_spawn();
    assert!(Preferences::default().message_preview);
    let d = dirs(&tmp("message-preview-legacy"));
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("settings.json"), r#"{"read_receipts": false, "message_preview": "x"}"#).unwrap();
    assert!(Preferences::load(&d).message_preview, "missing or non-boolean value means on");
    let saved = Preferences::update(&d, |p| p.message_preview = false).unwrap();
    assert!(!saved.message_preview && !Preferences::load(&d).message_preview, "off survives a reload");
}

#[test]
fn show_status_is_on_by_default_also_for_files_saved_before_it_existed() {
    let _gate = no_spawn();
    let d = dirs(&tmp("show-status"));
    assert!(Preferences::default().show_status);
    std::fs::create_dir_all(&d.config).unwrap();
    for body in [r#"{"read_receipts": false}"#, r#"{"show_status": "x"}"#, r#"{"show_status": null}"#] {
        std::fs::write(d.config.join("settings.json"), body).unwrap();
        assert!(Preferences::load(&d).show_status, "{body}");
    }
    let saved = Preferences::update(&d, |p| p.show_status = false).unwrap();
    assert!(!saved.show_status && !Preferences::load(&d).show_status, "off survives a reload");
}
