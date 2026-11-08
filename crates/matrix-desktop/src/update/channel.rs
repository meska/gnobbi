//! Update channel detection (AUTO_UPDATE.md §0.1-0.2). Pure: all facts come from [`Env`].
//! Only `AppImage` and `WindowsPortable` are updated in-app; every other channel does nothing.

use std::path::{Path, PathBuf};

/// How this binary was installed, hence who updates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// Linux AppImage; `target` is the file to swap.
    AppImage { target: PathBuf },
    /// Windows `gnobbi.exe` in a writable folder.
    WindowsPortable { exe: PathBuf },
    /// Mac App Store / TestFlight (Apple updates it).
    MacStore,
    Flatpak,
    /// Distro package, `/usr`, `/opt` or Snap.
    SystemPackage,
    /// Dev build, macOS outside the store, read-only Windows folder, anything else.
    Unknown,
}

impl Channel {
    /// True when the app may check for and download updates itself.
    pub fn self_updates(&self) -> bool {
        matches!(self, Channel::AppImage { .. } | Channel::WindowsPortable { .. })
    }
}

/// Snapshot of the facts the detection needs; build it with [`Env::from_process`] or fake it in tests.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `std::env::consts::OS`.
    pub os: String,
    pub home: Option<PathBuf>,
    /// `$APPIMAGE`, empty treated as unset.
    pub appimage: Option<PathBuf>,
    pub flatpak_id: bool,
    /// `/.flatpak-info` exists.
    pub flatpak_info: bool,
    pub snap: bool,
    pub current_exe: Option<PathBuf>,
    /// `~/Applications/gnobbi.AppImage` exists.
    pub stable_copy_exists: bool,
    /// macOS: `<bundle>/Contents/_MASReceipt/receipt` exists.
    pub mas_receipt: bool,
    /// Windows: the folder of `gnobbi.exe` is writable.
    pub exe_dir_writable: bool,
}

impl Env {
    /// Reads the real process environment (read-only: no file is created or modified).
    pub fn from_process() -> Env {
        let var = |k| std::env::var_os(k).filter(|v| !v.is_empty());
        let home = var("HOME").or_else(|| var("USERPROFILE")).map(PathBuf::from);
        let current_exe = std::env::current_exe().ok();
        let dir = current_exe.as_deref().and_then(Path::parent);
        Env {
            os: std::env::consts::OS.into(),
            stable_copy_exists: home
                .as_ref()
                .is_some_and(|h| h.join("Applications/gnobbi.AppImage").is_file()),
            appimage: var("APPIMAGE").map(PathBuf::from),
            flatpak_id: var("FLATPAK_ID").is_some(),
            flatpak_info: Path::new("/.flatpak-info").exists(),
            snap: var("SNAP").is_some(),
            // exe is <bundle>/Contents/MacOS/<bin>
            mas_receipt: dir
                .and_then(Path::parent)
                .is_some_and(|c| c.join("_MASReceipt/receipt").exists()),
            // shortcut: readonly flag + Program Files name; a real ACL check needs a write probe, which this module must not do.
            exe_dir_writable: dir.is_some_and(|d| {
                !d.to_string_lossy().contains("Program Files")
                    && std::fs::metadata(d).is_ok_and(|m| !m.permissions().readonly())
            }),
            home,
            current_exe,
        }
    }
}

/// Detects the channel. Linux order: Flatpak, `$APPIMAGE`, Snap, system folders, `target/`.
/// Flatpak goes first so a stray `$APPIMAGE` inside a sandbox never proposes an update.
pub fn detect_channel(env: &Env) -> Channel {
    match env.os.as_str() {
        "linux" => detect_linux(env),
        "macos" if env.mas_receipt => Channel::MacStore,
        "windows" => match &env.current_exe {
            Some(exe) if env.exe_dir_writable && !in_target(exe) => {
                Channel::WindowsPortable { exe: exe.clone() }
            }
            _ => Channel::Unknown,
        },
        _ => Channel::Unknown,
    }
}

fn detect_linux(env: &Env) -> Channel {
    if env.flatpak_info || env.flatpak_id {
        return Channel::Flatpak;
    }
    if let Some(image) = &env.appimage {
        let stable = env.home.as_ref().map(|h| h.join("Applications/gnobbi.AppImage"));
        let target = match stable {
            Some(s) if env.stable_copy_exists => s,
            _ => image.clone(),
        };
        return Channel::AppImage { target };
    }
    if env.snap {
        return Channel::SystemPackage;
    }
    match &env.current_exe {
        Some(e) if in_target(e) => Channel::Unknown,
        Some(e) if ["/usr", "/opt"].iter().any(|p| e.starts_with(p)) => Channel::SystemPackage,
        _ => Channel::Unknown,
    }
}

// `/usr/local` is covered by `/usr`.
fn in_target(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux() -> Env {
        Env {
            os: "linux".into(),
            home: Some("/home/u".into()),
            current_exe: Some("/tmp/.mount_gnobbiXX/usr/bin/gnobbi".into()),
            ..Env::default()
        }
    }

    #[test]
    fn appimage_uses_env_path_not_current_exe() {
        let env = Env { appimage: Some("/home/u/Downloads/g.AppImage".into()), ..linux() };
        assert_eq!(
            detect_channel(&env),
            Channel::AppImage { target: "/home/u/Downloads/g.AppImage".into() }
        );
    }

    #[test]
    fn appimage_from_downloads_updates_applications_copy() {
        let env = Env {
            appimage: Some("/home/u/Downloads/g.AppImage".into()),
            stable_copy_exists: true,
            ..linux()
        };
        assert_eq!(
            detect_channel(&env),
            Channel::AppImage { target: "/home/u/Applications/gnobbi.AppImage".into() }
        );
    }

    #[test]
    fn flatpak_wins_over_spurious_appimage() {
        for (info, id) in [(true, false), (false, true)] {
            let env = Env {
                appimage: Some("/x.AppImage".into()),
                flatpak_info: info,
                flatpak_id: id,
                ..linux()
            };
            assert_eq!(detect_channel(&env), Channel::Flatpak);
        }
    }

    #[test]
    fn snap_and_system_folders_are_system_package() {
        assert_eq!(detect_channel(&Env { snap: true, ..linux() }), Channel::SystemPackage);
        for p in ["/usr/bin/gnobbi", "/usr/local/bin/gnobbi", "/opt/gnobbi/gnobbi"] {
            let env = Env { current_exe: Some(p.into()), ..linux() };
            assert_eq!(detect_channel(&env), Channel::SystemPackage, "{p}");
        }
    }

    #[test]
    fn dev_build_and_container_only_are_unknown() {
        for p in ["/home/u/gnobbi/target/debug/gnobbi", "/home/u/bin/gnobbi"] {
            let env = Env { current_exe: Some(p.into()), ..linux() };
            assert_eq!(detect_channel(&env), Channel::Unknown, "{p}");
        }
    }

    #[test]
    fn macos_receipt_means_store_else_unknown() {
        let mac = Env { os: "macos".into(), ..Env::default() };
        assert_eq!(detect_channel(&Env { mas_receipt: true, ..mac.clone() }), Channel::MacStore);
        assert_eq!(detect_channel(&mac), Channel::Unknown);
    }

    #[test]
    fn windows_needs_writable_folder_and_not_target() {
        let win = Env {
            os: "windows".into(),
            current_exe: Some(r"C:\Gnobbi\gnobbi.exe".into()),
            exe_dir_writable: true,
            ..Env::default()
        };
        assert_eq!(
            detect_channel(&win),
            Channel::WindowsPortable { exe: r"C:\Gnobbi\gnobbi.exe".into() }
        );
        assert_eq!(detect_channel(&Env { exe_dir_writable: false, ..win.clone() }), Channel::Unknown);
        let dev = Env { current_exe: Some("C:/dev/target/debug/gnobbi.exe".into()), ..win };
        assert_eq!(detect_channel(&dev), Channel::Unknown);
    }

    #[test]
    fn only_two_channels_self_update() {
        assert!(Channel::AppImage { target: "x".into() }.self_updates());
        assert!(!Channel::MacStore.self_updates());
        assert!(!Channel::Unknown.self_updates());
    }
}
