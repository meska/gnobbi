//! `--install-desktop-entry`: explicit, idempotent per-user install of the launcher and icons.
use crate::IconChoice;
use std::{fs, io, path::Path};

const APP_ID: &str = "net.meskatech.gnobbi";
const DESKTOP: &str = include_str!("../../../packaging/linux/net.meskatech.gnobbi.desktop");

/// Installs for the current user (CLI and in-app UI): `$APPIMAGE` if set, else `current_exe`.
pub fn install_for_current_user(icon: IconChoice) -> Result<(), String> {
    // $APPIMAGE is the AppImage file; current_exe would point into the temporary mount.
    let exec = std::env::var("APPIMAGE")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
        .ok_or("executable path unknown")?;
    let home = std::env::var_os("HOME").ok_or("HOME unknown")?;
    install_desktop_entry(Path::new(&home), &exec, icon).map_err(|e| e.to_string())
}

/// User data dir shared by install and detection: an absolute `$XDG_DATA_HOME`, else `home/.local/share`.
pub(crate) fn user_share(home: &Path, xdg_data_home: Option<&Path>) -> std::path::PathBuf {
    match xdg_data_home {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        _ => home.join(".local/share"),
    }
}

/// Writes the entry and icons under the user data dir, then refreshes the user caches.
pub fn install_desktop_entry(home: &Path, exec: &str, icon: IconChoice) -> io::Result<()> {
    let xdg = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty());
    let share = user_share(home, xdg.as_deref().map(Path::new));
    install_desktop_entry_with(&share, exec, icon, &mut |prog, args| {
        // Missing binary or failure is deliberately ignored: the caches are only an optimisation.
        let _ = std::process::Command::new(prog)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    })
}

/// Same as [`install_desktop_entry`] with an injectable cache-command runner `(program, args)`.
pub fn install_desktop_entry_with(
    share: &Path,
    exec: &str,
    icon: IconChoice,
    run: &mut dyn FnMut(&str, &[&Path]),
) -> io::Result<()> {
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
        .replace('$', "\\$")
        .replace('%', "%%");
    let desktop = DESKTOP.replace("Exec=gnobbi", &format!("Exec=\"{quoted}\""));
    let apps = share.join("applications");
    fs::create_dir_all(&apps)?;
    fs::write(apps.join(format!("{APP_ID}.desktop")), desktop)?;
    for (size, png) in [("256x256", icon.theme_png_256()), ("512x512", icon.runtime_png())] {
        let dir = share.join("icons/hicolor").join(size).join("apps");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{APP_ID}.png")), png)?;
    }
    run("update-desktop-database", &[&apps]);
    run(
        "gtk-update-icon-cache",
        &[Path::new("-f"), Path::new("-t"), &share.join("icons/hicolor")],
    );
    Ok(())
}

/// Read-only detection of a missing/stale launcher entry (Linux only, no writes).
#[cfg(target_os = "linux")]
pub mod detect {
    use super::APP_ID;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EntryStatus {
        Missing,
        /// User entry exists but Exec is not the current executable, or the icon is gone.
        Stale,
        Ok,
        /// Only a system-wide entry exists (package install): never prompt.
        SystemPackage,
    }

    /// Environment snapshot, injectable for tests.
    #[derive(Debug, Clone, Default)]
    pub struct Env {
        pub home: Option<PathBuf>,
        pub xdg_data_home: Option<PathBuf>,
        pub xdg_data_dirs: Option<String>,
        /// `$APPIMAGE` if set, else `current_exe()`.
        pub exec: Option<String>,
        /// DISPLAY or WAYLAND_DISPLAY present.
        pub graphical: bool,
    }

    impl Env {
        pub fn from_process() -> Env {
            let var = |k| std::env::var_os(k).filter(|v| !v.is_empty());
            Env {
                home: var("HOME").map(PathBuf::from),
                xdg_data_home: var("XDG_DATA_HOME").map(PathBuf::from),
                xdg_data_dirs: var("XDG_DATA_DIRS").and_then(|v| v.into_string().ok()),
                exec: var("APPIMAGE")
                    .and_then(|v| v.into_string().ok())
                    .or_else(|| {
                        std::env::current_exe()
                            .ok()
                            .map(|p| p.to_string_lossy().into_owned())
                    }),
                graphical: var("DISPLAY").is_some() || var("WAYLAND_DISPLAY").is_some(),
            }
        }

        fn user_share(&self) -> Option<PathBuf> {
            let home = self.home.as_ref()?;
            Some(super::user_share(home, self.xdg_data_home.as_deref()))
        }

        fn system_shares(&self) -> Vec<PathBuf> {
            let dirs = self
                .xdg_data_dirs
                .as_deref()
                .filter(|d| !d.is_empty())
                .unwrap_or("/usr/local/share:/usr/share");
            dirs.split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .collect()
        }
    }

    fn entry_file(share: &Path) -> PathBuf {
        share.join("applications").join(format!("{APP_ID}.desktop"))
    }

    fn has_icon(share: &Path) -> bool {
        let Ok(sizes) = std::fs::read_dir(share.join("icons/hicolor")) else {
            return false;
        };
        sizes.flatten().any(|s| {
            ["png", "svg"]
                .iter()
                .any(|e| s.path().join("apps").join(format!("{APP_ID}.{e}")).exists())
        })
    }

    /// First token of the `Exec=` line, unquoted per the Desktop Entry spec.
    fn exec_target(desktop: &str) -> Option<String> {
        let rest = desktop
            .lines()
            .find_map(|l| l.strip_prefix("Exec="))?
            .trim();
        let mut out = String::new();
        let mut chars = rest.chars();
        if let Some(q) = rest.strip_prefix('"') {
            chars = q.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
        } else {
            out.extend(chars.take_while(|c| !c.is_whitespace()));
        }
        Some(out.replace("%%", "%"))
    }

    pub fn entry_status(env: &Env) -> EntryStatus {
        let user = env.user_share();
        if let Some(share) = &user
            && let Ok(text) = std::fs::read_to_string(entry_file(share))
        {
            let exec_ok = exec_target(&text).is_some_and(|t| {
                Path::new(&t).exists() && env.exec.as_deref().is_none_or(|cur| cur == t)
            });
            return if exec_ok && has_icon(share) {
                EntryStatus::Ok
            } else {
                EntryStatus::Stale
            };
        }
        if env.system_shares().iter().any(|s| entry_file(s).exists()) {
            EntryStatus::SystemPackage
        } else {
            EntryStatus::Missing
        }
    }

    /// Offer installation only for Missing/Stale, with a graphical session and no opt-out.
    pub fn should_prompt(env: &Env, dont_ask_again: bool) -> bool {
        !dont_ask_again
            && env.graphical
            && matches!(entry_status(env), EntryStatus::Missing | EntryStatus::Stale)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        /// Temp root with a fake exe; returns (env, root).
        fn setup(name: &str) -> (Env, PathBuf) {
            let root =
                std::env::temp_dir().join(format!("gnobbi-de-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("home")).unwrap();
            fs::create_dir_all(root.join("sys")).unwrap();
            fs::write(root.join("app.AppImage"), "x").unwrap();
            let env = Env {
                home: Some(root.join("home")),
                xdg_data_dirs: Some(root.join("sys").to_string_lossy().into_owned()),
                exec: Some(root.join("app.AppImage").to_string_lossy().into_owned()),
                graphical: true,
                ..Env::default()
            };
            (env, root)
        }

        fn install(env: &Env, exec: &str) {
            let share = super::super::user_share(
                env.home.as_ref().unwrap(),
                env.xdg_data_home.as_deref(),
            );
            crate::desktop_entry::install_desktop_entry_with(
                &share,
                exec,
                crate::IconChoice::default(),
                &mut |_, _| {},
            )
            .unwrap();
        }

        #[test]
        fn missing_prompts() {
            let (env, _) = setup("missing");
            assert_eq!(entry_status(&env), EntryStatus::Missing);
            assert!(should_prompt(&env, false));
        }

        #[test]
        fn correct_entry_does_not_prompt() {
            let (env, _) = setup("ok");
            install(&env, env.exec.as_deref().unwrap());
            assert_eq!(entry_status(&env), EntryStatus::Ok);
            assert!(!should_prompt(&env, false));
        }

        #[test]
        fn stale_exec_prompts() {
            let (env, root) = setup("stale");
            let gone = root.join("moved.AppImage");
            install(&env, gone.to_str().unwrap());
            assert_eq!(entry_status(&env), EntryStatus::Stale);
            assert!(should_prompt(&env, false));
            // existing but different executable is stale too
            fs::write(&gone, "x").unwrap();
            assert_eq!(entry_status(&env), EntryStatus::Stale);
        }

        #[test]
        fn missing_icon_is_stale() {
            let (env, root) = setup("icon");
            install(&env, env.exec.as_deref().unwrap());
            fs::remove_dir_all(root.join("home/.local/share/icons")).unwrap();
            assert_eq!(entry_status(&env), EntryStatus::Stale);
        }

        #[test]
        fn system_entry_does_not_prompt() {
            let (env, root) = setup("sys");
            fs::create_dir_all(root.join("sys/applications")).unwrap();
            fs::write(root.join(format!("sys/applications/{APP_ID}.desktop")), "").unwrap();
            assert_eq!(entry_status(&env), EntryStatus::SystemPackage);
            assert!(!should_prompt(&env, false));
        }

        #[test]
        fn no_display_or_opt_out_does_not_prompt() {
            let (mut env, _) = setup("nodisp");
            assert!(should_prompt(&env, false));
            assert!(!should_prompt(&env, true));
            env.graphical = false;
            assert!(!should_prompt(&env, false));
        }

        #[test]
        fn install_and_detect_agree_with_xdg_data_home_set() {
            let (mut env, root) = setup("xdgrt");
            env.xdg_data_home = Some(root.join("xdg"));
            install(&env, env.exec.as_deref().unwrap());
            assert_eq!(entry_status(&env), EntryStatus::Ok);
        }

        #[test]
        fn exec_with_percent_and_space_round_trips() {
            let (mut env, root) = setup("pct");
            let exe = root.join("my 50% apps/G.AppImage");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, "x").unwrap();
            env.exec = Some(exe.to_string_lossy().into_owned());
            install(&env, env.exec.as_deref().unwrap());
            assert_eq!(entry_status(&env), EntryStatus::Ok);
        }

        #[test]
        fn xdg_data_home_overrides_home() {
            let (mut env, root) = setup("xdgh");
            install(&env, env.exec.as_deref().unwrap());
            assert_eq!(entry_status(&env), EntryStatus::Ok);
            env.xdg_data_home = Some(root.join("elsewhere"));
            assert_eq!(entry_status(&env), EntryStatus::Missing);
        }
    }
}
