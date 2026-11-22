//! The process shows up as `gnobbi` (ps/top take the name from the executable file) and the
//! desktop entry shipped in the AppDir matches the window app id.
#![cfg(not(any(target_os = "ios", target_os = "android")))]
use std::path::Path;

const APP_ID: &str = "net.meskatech.gnobbi";

#[test]
fn executable_is_named_gnobbi() {
    let exe = Path::new(env!("CARGO_BIN_EXE_gnobbi"));
    assert_eq!(exe.file_stem().unwrap(), "gnobbi");
}

#[test]
fn appdir_desktop_entry_matches_app_id() {
    let linux = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/linux");
    let desktop = std::fs::read_to_string(linux.join(format!("{APP_ID}.desktop"))).unwrap();
    for line in [
        format!("StartupWMClass={APP_ID}"),
        format!("Icon={APP_ID}"),
        "Exec=gnobbi %u".into(),
        "MimeType=x-scheme-handler/matrix;".into(),
    ] {
        assert!(desktop.lines().any(|l| l == line), "missing {line}");
    }
    // The AppDir is filled from that file, with the binary installed as usr/bin/gnobbi.
    let script = std::fs::read_to_string(linux.join("build-appimage.sh")).unwrap();
    assert!(script.contains("dist/gnobbi\"") && script.contains("usr/bin/gnobbi"));
    assert!(script.contains("cp \"$src/$APP_ID.desktop\" \"$d/$APP_ID.desktop\""));
}
