#![cfg(target_os = "linux")]
//! Install/repair of an old launcher (no `%u`, no `MimeType`) registers Gnobbi for `matrix:` links, checked
//! with the real `xdg-mime`/`gio` under a disposable HOME (never the real one). Skips when a tool is missing.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const ID: &str = "net.meskatech.gnobbi.desktop";
const MIME: &str = "x-scheme-handler/matrix";

fn has(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// HOME with the old entry (copy of the one on a Linux test machine, Exec on an existing AppImage) installed.
fn old_home(name: &str) -> (PathBuf, PathBuf) {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&home);
    let apps = home.join(".local/share/applications");
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(home.join("Applications")).unwrap();
    let exec = home.join("Applications/Gnobbi-x86_64.AppImage");
    fs::write(&exec, "x").unwrap();
    // GLib ignores a launcher whose Exec is not an executable file, as for a real AppImage.
    fs::set_permissions(&exec, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    fs::write(
        apps.join(ID),
        format!(
            "[Desktop Entry]\nType=Application\nName=Gnobbi\nComment=Matrix chat client\n\
             Exec=\"{}\"\nIcon=net.meskatech.gnobbi\nTerminal=false\n\
             Categories=Network;InstantMessaging;\nStartupWMClass=net.meskatech.gnobbi\n",
            exec.display()
        ),
    )
    .unwrap();
    (home, exec)
}

fn env<'a>(cmd: &'a mut Command, home: &Path) -> &'a mut Command {
    // Only the disposable HOME is visible: no inherited XDG overrides, system dirs stay for the tools.
    cmd.env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_DIRS")
        .env_remove("XDG_CURRENT_DESKTOP")
}

/// Installs in-process with the disposable HOME (the `xdg-mime` children inherit it); serialised because
/// the environment is process-wide.
fn repair(home: &Path, exec: &Path) {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = Command::new("true");
    env(&mut c, home);
    for (k, v) in c.get_envs() {
        // SAFETY: guarded by ENV; no other thread in this test binary touches the environment.
        unsafe {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    matrix_desktop::desktop_entry::install_desktop_entry(
        home,
        exec.to_str().unwrap(),
        matrix_desktop::IconChoice::default(),
    )
    .unwrap();
}

fn out(home: &Path, prog: &str, args: &[&str]) -> String {
    let mut c = Command::new(prog);
    let o = env(&mut c, home).args(args).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn default_handler(home: &Path) -> String {
    out(home, "xdg-mime", &["query", "default", MIME])
}

#[test]
fn repair_of_old_entry_registers_gnobbi_for_matrix_and_is_idempotent() {
    if !has("xdg-mime") || !has("gio") {
        return eprintln!("skipped: xdg-mime/gio missing");
    }
    let (home, exec) = old_home("mime-old-entry");
    let entry = home.join(".local/share/applications").join(ID);
    let cache = home.join(".local/share/applications/mimeinfo.cache");
    let mut state = Vec::new();
    for _ in 0..2 {
        repair(&home, &exec);
        let text = fs::read_to_string(&entry).unwrap();
        assert!(
            text.lines().any(|l| l == format!("MimeType={MIME};")),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|l| l.starts_with("Exec=") && l.ends_with(" %u")),
            "{text}"
        );
        assert_eq!(default_handler(&home), ID);
        // mimeinfo.cache comes from update-desktop-database, which the CT may lack.
        if has("update-desktop-database") {
            let c = fs::read_to_string(&cache).unwrap();
            assert!(
                c.lines()
                    .any(|l| l.starts_with(&format!("{MIME}=")) && l.contains(ID)),
                "{c}"
            );
        }
        let gio = out(&home, "gio", &["mime", MIME]);
        assert!(gio.contains(ID), "{gio}");
        state.push((text, gio));
    }
    assert_eq!(state[0], state[1], "second repair changed something");
}

// Install/repair never takes the scheme from another client (nheko here), however often it runs.
#[test]
fn repair_keeps_foreign_default_and_mimeapps_list() {
    if !has("xdg-mime") {
        return eprintln!("skipped: xdg-mime missing");
    }
    let (home, exec) = old_home("mime-foreign-handler");
    let apps = home.join(".local/share/applications");
    // GLib only honours a default whose Exec is an existing executable.
    fs::write(
        apps.join("nheko.desktop"),
        format!("[Desktop Entry]\nType=Application\nName=nheko\nExec=/bin/sh %u\nMimeType={MIME};\n"),
    )
    .unwrap();
    let list = home.join(".config/mimeapps.list");
    fs::create_dir_all(list.parent().unwrap()).unwrap();
    fs::write(
        &list,
        format!("[Default Applications]\n{MIME}=nheko.desktop\ntext/html=other.desktop\n"),
    )
    .unwrap();
    assert_eq!(default_handler(&home), "nheko.desktop");

    for _ in 0..2 {
        repair(&home, &exec);
    }

    let kept = fs::read_to_string(&list).expect("mimeapps.list deleted");
    assert!(kept.contains("text/html=other.desktop"), "{kept}");
    assert_eq!(
        default_handler(&home),
        "nheko.desktop",
        "foreign default overwritten without consent"
    );
}
