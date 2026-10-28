//! `--install-desktop-entry`: explicit, idempotent per-user install of the launcher and icons.
use std::{fs, io, path::Path};

const APP_ID: &str = "net.meskatech.gnobbi";
const DESKTOP: &str = include_str!("../../../packaging/linux/net.meskatech.gnobbi.desktop");
const ICONS: [(&str, &[u8]); 2] = [
    (
        "256x256",
        include_bytes!("../../../packaging/linux/icon-256.png"),
    ),
    (
        "512x512",
        include_bytes!("../../../packaging/linux/icon-512.png"),
    ),
];

/// Writes the entry and icons under `home/.local/share`; `exec` is the absolute AppImage path.
pub fn install_desktop_entry(home: &Path, exec: &str) -> io::Result<()> {
    if !Path::new(exec).is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Exec path must be absolute",
        ));
    }
    // Desktop Entry spec: quote the Exec argument and escape its special characters.
    let quoted = exec
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    let desktop = DESKTOP.replace("Exec=gnobbi", &format!("Exec=\"{quoted}\""));
    let share = home.join(".local/share");
    let apps = share.join("applications");
    fs::create_dir_all(&apps)?;
    fs::write(apps.join(format!("{APP_ID}.desktop")), desktop)?;
    for (size, png) in ICONS {
        let dir = share.join("icons/hicolor").join(size).join("apps");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{APP_ID}.png")), png)?;
    }
    Ok(())
}
