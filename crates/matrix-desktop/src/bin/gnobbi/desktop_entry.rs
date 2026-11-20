use matrix_desktop::desktop_entry::detect::Env;

fn should_install(env: &Env, dont_ask: bool) -> bool {
    matrix_desktop::desktop_entry::detect::should_prompt(env, dont_ask)
}

pub fn install(icon: matrix_desktop::IconChoice) -> std::io::Result<()> {
    use std::{io, path::Path, process::{Command, Stdio}};
    let env = Env::from_process();
    let home = env.home.ok_or_else(|| io::Error::other("HOME unavailable"))?;
    let share = env.xdg_data_home.filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".local/share"));
    let exec = env.exec.ok_or_else(|| io::Error::other("Executable unavailable"))?;
    matrix_desktop::desktop_entry::install_desktop_entry_with(&share, &exec, icon, &mut |program, args: &[&Path]| {
        if program != "xdg-mime" {
            let _ = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    })
}

pub fn install_at_start(icon: matrix_desktop::IconChoice, dont_ask: bool) {
    let _ = std::thread::Builder::new().name("desktop-entry".into()).spawn(move || {
        if should_install(&Env::from_process(), dont_ask) {
            let _ = install(icon);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_desktop::desktop_entry::{detect::entry_status, install_desktop_entry_with};
    use matrix_desktop::IconChoice;
    use std::fs;

    #[test]
    fn automatic_install_respects_status_and_opt_out() {
        let root = std::env::temp_dir().join(format!("gnobbi-entry-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("gnobbi");
        fs::write(&exe, "test").unwrap();
        let home = root.join("home");
        let system = root.join("system");
        let env = Env { home: Some(home.clone()), xdg_data_home: None, xdg_data_dirs: Some(system.to_string_lossy().into()), exec: Some(exe.to_string_lossy().into()), graphical: true };
        assert!(should_install(&env, false));
        assert!(!should_install(&env, true));
        install_desktop_entry_with(&home.join(".local/share"), exe.to_str().unwrap(), IconChoice::default(), &mut |_, _| {}).unwrap();
        assert!(!should_install(&env, false));
        fs::remove_file(home.join(".local/share/icons/hicolor/256x256/apps/net.meskatech.gnobbi.png")).unwrap();
        fs::remove_file(home.join(".local/share/icons/hicolor/512x512/apps/net.meskatech.gnobbi.png")).unwrap();
        assert!(should_install(&env, false));
        assert!(!should_install(&env, true));
        fs::remove_file(home.join(".local/share/applications/net.meskatech.gnobbi.desktop")).unwrap();
        install_desktop_entry_with(&system, exe.to_str().unwrap(), IconChoice::default(), &mut |_, _| {}).unwrap();
        assert!(!should_install(&env, false), "{status:?}", status = entry_status(&env));
        fs::remove_dir_all(root).unwrap();
    }
}
