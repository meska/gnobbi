//! `--install-desktop-entry` writes the launcher and icons under a temporary HOME, only on request.
#![cfg(feature = "desktop")]
use std::{fs, path::Path, process::Command};

const ID: &str = "net.meskatech.gnobbi";

fn run(home: &Path, args: &[&str]) -> std::process::ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_gnobbi"))
        .args(args)
        .env("HOME", home)
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
    let linux = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/linux");
    for size in ["256", "512"] {
        let icon = home.join(format!(".local/share/icons/hicolor/{size}x{size}/apps/{ID}.png"));
        assert_eq!(fs::read(icon).unwrap(), fs::read(linux.join(format!("icon-{size}.png"))).unwrap());
    }

    assert!(run(&home, &["--install-desktop-entry"]).success());
    assert_eq!(fs::read_to_string(&desktop).unwrap(), first);
}

#[test]
fn library_writes_nothing_unless_called() {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desktop-entry-empty");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    assert!(matrix_desktop::desktop_entry::install_desktop_entry(&home, "relative").is_err());
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
}
