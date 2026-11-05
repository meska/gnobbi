use matrix_desktop::{Accent, AppDirs, AutoNight, FontChoice, Preferences, Theme};
use std::path::PathBuf;

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
    let d = dirs(&tmp("missing"));
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn roundtrip_keeps_values() {
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
        close_to_tray: true,
        desktop_entry_dont_ask: true,
    };
    p.save(&d).unwrap();
    assert_eq!(Preferences::load(&d), p);
}

#[test]
fn corrupt_file_gives_defaults() {
    let d = dirs(&tmp("corrupt"));
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("settings.json"), b"{not json").unwrap();
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn unknown_keys_ignored_and_invalid_values_default_per_field() {
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
    let d = dirs(&tmp("perm"));
    Preferences::default().save(&d).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&d.config), 0o700);
    assert_eq!(mode(&d.config.join("settings.json")), 0o600);
}

#[test]
fn thumbnail_cache_dir_is_under_cache_and_created() {
    let d = dirs(&tmp("thumbs"));
    let t = d.thumbnails().unwrap();
    assert_eq!(t, d.cache.join("thumbnails"));
    assert!(t.is_dir());
}

#[test]
fn read_receipts_default_on_and_invalid_value_falls_back_to_on() {
    assert!(Preferences::default().read_receipts);
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
        ["Segno minimo", "Gnomo", "Mostriciattolo"]
    );
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
