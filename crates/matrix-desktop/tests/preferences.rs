use matrix_desktop::{AppDirs, Preferences, Theme};
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
    let d = dirs(&tmp("missing"));
    assert_eq!(Preferences::load(&d), Preferences::default());
}

#[test]
fn roundtrip_keeps_values() {
    let d = dirs(&tmp("roundtrip"));
    let p = Preferences {
        theme: Some(Theme::Night),
        debug: true,
        read_receipts: false,
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
