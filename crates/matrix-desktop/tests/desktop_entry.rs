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

fn run(home: &Path, args: &[&str]) -> std::process::ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_gnobbi"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env("APPIMAGE", "/opt/my apps/Gnobbi.AppImage")
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
        "Exec=\"/opt/my apps/Gnobbi.AppImage\"".to_string(),
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
