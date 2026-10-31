//! `--install-desktop-entry`: explicit, idempotent per-user install of the launcher and icons.
use crate::IconChoice;
use std::{fs, io, path::Path};

const APP_ID: &str = "net.meskatech.gnobbi";
const DESKTOP: &str = include_str!("../../../packaging/linux/net.meskatech.gnobbi.desktop");

/// Writes the entry and icons under `home/.local/share`; `exec` is the absolute AppImage path;
/// `icon` is the saved choice (shown by the shell at its next start or bar restart).
pub fn install_desktop_entry(home: &Path, exec: &str, icon: IconChoice) -> io::Result<()> {
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
    for (size, png) in [("256x256", icon.theme_png_256()), ("512x512", icon.runtime_png())] {
        let dir = share.join("icons/hicolor").join(size).join("apps");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{APP_ID}.png")), png)?;
    }
    Ok(())
}
