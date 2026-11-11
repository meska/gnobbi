#![cfg(unix)] // POSIX path literals / Linux desktop entry: is_absolute is host-specific.
//! `--install-desktop-entry` writes the launcher and icons under a temporary HOME, only on request.
#![cfg(feature = "desktop")]
use matrix_desktop::IconChoice;
use std::{fs, path::Path, process::Command};

fn assert_icons(home: &Path, icon: IconChoice) {
    for (size, png) in [("256", icon.theme_png_256()), ("512", icon.runtime_png())] {
        let file = home.join(format!(".local/share/icons/hicolor/{size}x{size}/apps/{ID}.png"));
        assert_eq!(fs::read(file).unwrap(), png);
    }
}

const ID: &str = "net.meskatech.gnobbi";

/// A real AppImage already under `~/Applications` (with a space in its path): used as is, not copied.
fn appimage(home: &Path) -> std::path::PathBuf {
    let app = home.join("Applications/my apps/Gnobbi.AppImage");
    fs::create_dir_all(app.parent().unwrap()).unwrap();
    fs::write(&app, "x").unwrap();
    app
}

fn run(home: &Path, args: &[&str]) -> std::process::ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_gnobbi"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env("APPIMAGE", appimage(home))
        .status()
        .unwrap()
}

#[test]
fn installs_idempotently_and_only_on_request() {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desktop-entry-home");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();

    // Not a UI launch path: use an unknown-free flag set; without the flag nothing may be written.
    let desktop = home.join(format!(".local/share/applications/{ID}.desktop"));
    assert!(!desktop.exists());

    assert!(run(&home, &["--install-desktop-entry"]).success());
    let first = fs::read_to_string(&desktop).unwrap();
    for line in [
        format!("Exec=\"{}\" %u", home.join("Applications/my apps/Gnobbi.AppImage").display()),
        "MimeType=x-scheme-handler/matrix;".to_string(),
        format!("Icon={ID}"),
        format!("StartupWMClass={ID}"),
    ] {
        assert!(first.lines().any(|l| l == line), "missing {line}");
    }
    assert_icons(&home, IconChoice::MinimalMark);

    assert!(run(&home, &["--install-desktop-entry"]).success());
    assert_eq!(fs::read_to_string(&desktop).unwrap(), first);
}

#[test]
fn library_writes_nothing_unless_called() {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desktop-entry-empty");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    assert!(matrix_desktop::desktop_entry::install_desktop_entry(&home, "relative", IconChoice::default()).is_err());
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
}

#[test]
fn installs_the_saved_icon_and_falls_back_to_the_default() {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desktop-entry-icon");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".config/gnobbi")).unwrap();
    let settings = home.join(".config/gnobbi/settings.json");
    for (json, expected) in [
        (r#"{"icon":"gnomo"}"#, IconChoice::Gnome),
        (r#"{"icon":"mostriciattolo"}"#, IconChoice::Monster),
        (r#"{"icon":"nonsense"}"#, IconChoice::MinimalMark),
    ] {
        fs::write(&settings, json).unwrap();
        assert!(run(&home, &["--install-desktop-entry"]).success());
        assert_icons(&home, expected);
    }
    fs::remove_file(&settings).unwrap();
    assert!(run(&home, &["--install-desktop-entry"]).success());
    assert_icons(&home, IconChoice::MinimalMark);
}

#[test]
fn runs_user_cache_commands_and_writes_only_under_home() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desktop-entry-caches");
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let share = home.join(".local/share");
    let mut calls = Vec::new();
    matrix_desktop::desktop_entry::install_desktop_entry_with(&share, "/opt/G.AppImage", IconChoice::default(), &mut |p, a| {
        calls.push((p.to_string(), a.iter().map(|x| x.to_path_buf()).collect::<Vec<_>>()))
    })
    .unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].0, "update-desktop-database");
    assert_eq!(calls[0].1, [home.join(".local/share/applications")]);
    assert_eq!(calls[1].0, "gtk-update-icon-cache");
    assert_eq!(calls[1].1[2], home.join(".local/share/icons/hicolor"));
    assert_eq!(calls[2].0, "xdg-mime");
    assert_eq!(calls[2].1[0], Path::new("default"));
    assert_eq!(calls[2].1[2], Path::new("x-scheme-handler/matrix"));
    // A stale Exec is rewritten.
    matrix_desktop::desktop_entry::install_desktop_entry_with(&share, "/opt/New.AppImage", IconChoice::default(), &mut |_, _| {}).unwrap();
    let d = fs::read_to_string(home.join(format!(".local/share/applications/{ID}.desktop"))).unwrap();
    assert!(d.contains("Exec=\"/opt/New.AppImage\""));
    // Everything lives under HOME; the sandbox root holds nothing else.
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
}
