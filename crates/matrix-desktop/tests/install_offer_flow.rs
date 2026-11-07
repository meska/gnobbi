#![cfg(target_os = "linux")]
//! End to end of the launcher-entry proposal under a temporary HOME: when it is offered, what each
//! choice does, that «Non chiedere più» survives a restart, and that nothing is written outside HOME.
use matrix_desktop::desktop_entry::{
    detect::{entry_status, should_prompt, Env, EntryStatus},
    install_desktop_entry, install_desktop_entry_with,
};
use matrix_desktop::install_offer::{Choice, State};
use matrix_desktop::{AppDirs, IconChoice, Preferences};
use std::{fs, path::{Path, PathBuf}};

const ID: &str = "net.meskatech.gnobbi";

/// Sandbox: `root/{home,sys,app.AppImage}`. `Env` points only inside it.
fn sandbox(name: &str) -> (Env, PathBuf) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("offer-flow-{name}"));
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
    let share = env.home.as_ref().unwrap().join(".local/share");
    install_desktop_entry_with(&share, exec, IconChoice::default(), &mut |_, _| {}).unwrap();
}

/// Every path under `root`, relative and sorted: a filesystem fingerprint of names.
fn tree(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            if p.is_dir() {
                walk(&p, root, out);
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out.sort();
    out
}

fn dirs(root: &Path) -> AppDirs {
    AppDirs {
        config: root.join("home/.config/gnobbi"),
        data: root.join("home/.local/share/gnobbi"),
        cache: root.join("home/.cache/gnobbi"),
    }
}

#[test]
fn full_flow_missing_add_then_moved_then_never_across_restart() {
    let (env, root) = sandbox("flow");
    let d = dirs(&root);
    let mut prefs = Preferences::load(&d);
    assert_eq!(entry_status(&env), EntryStatus::Missing);

    // First launch: proposed once ready.
    let mut s = State::default();
    s.poll(true, false, || should_prompt(&env, prefs.desktop_entry_dont_ask));
    assert!(s.open);

    // «Aggiungi al menu»: entry and icon appear, nothing more is proposed.
    let exec = env.exec.clone().unwrap();
    assert!(!s.choose(Choice::Add, &mut prefs.desktop_entry_dont_ask, || {
        install(&env, &exec);
        Ok(())
    }));
    assert!(!s.open);
    assert_eq!(entry_status(&env), EntryStatus::Ok);
    assert!(!should_prompt(&env, false));

    // AppImage moved: the entry is stale and proposed again, on the next launch.
    let moved = root.join("moved.AppImage");
    fs::rename(&exec, &moved).unwrap();
    let env2 = Env { exec: Some(moved.to_string_lossy().into_owned()), ..env.clone() };
    assert_eq!(entry_status(&env2), EntryStatus::Stale);
    let mut s = State::default();
    s.poll(true, false, || should_prompt(&env2, prefs.desktop_entry_dont_ask));
    assert!(s.open);

    // «Non ora»: nothing saved, proposed again at the next launch.
    assert!(!s.choose(Choice::Later, &mut prefs.desktop_entry_dont_ask, || panic!()));
    let mut s = State::default();
    s.poll(true, false, || should_prompt(&env2, prefs.desktop_entry_dont_ask));
    assert!(s.open);

    // «Non chiedere più»: saved, and respected after a restart (fresh State, prefs reloaded).
    assert!(s.choose(Choice::Never, &mut prefs.desktop_entry_dont_ask, || panic!()));
    prefs.save(&d).unwrap();
    let reloaded = Preferences::load(&d);
    assert!(reloaded.desktop_entry_dont_ask);
    let mut s = State::default();
    s.poll(true, false, || should_prompt(&env2, reloaded.desktop_entry_dont_ask));
    assert!(!s.open);

    // Reversible from Settings: «Aggiungi al menu» repairs the entry and clears the opt-out.
    let mut p = reloaded;
    assert!(State::default().choose(Choice::Add, &mut p.desktop_entry_dont_ask, || {
        install(&env2, env2.exec.as_deref().unwrap());
        Ok(())
    }));
    p.save(&d).unwrap();
    assert!(!Preferences::load(&d).desktop_entry_dont_ask);
    assert_eq!(entry_status(&env2), EntryStatus::Ok);

    // Nothing outside HOME: the sandbox holds only home, sys and the (moved) executable.
    let top: Vec<_> = tree(&root).into_iter().filter(|p| !p.contains('/')).collect();
    assert_eq!(top, ["home", "moved.AppImage", "sys"]);
    assert!(tree(&root.join("sys")).is_empty());
}

#[test]
fn system_entry_and_missing_display_never_propose_and_write_nothing() {
    let (mut env, root) = sandbox("noprompt");
    fs::create_dir_all(root.join("sys/applications")).unwrap();
    fs::write(root.join(format!("sys/applications/{ID}.desktop")), "").unwrap();
    let before = tree(&root);
    assert_eq!(entry_status(&env), EntryStatus::SystemPackage);
    assert!(!should_prompt(&env, false));
    fs::remove_file(root.join(format!("sys/applications/{ID}.desktop"))).unwrap();
    env.graphical = false; // no DISPLAY / WAYLAND_DISPLAY
    assert!(!should_prompt(&env, false));
    env.graphical = true;
    assert!(should_prompt(&env, false));
    assert_eq!(tree(&root).len(), before.len() - 1, "detection only reads");
    assert!(tree(&root.join("home")).is_empty());
}

#[test]
fn dialog_tree_has_three_labelled_buttons_and_no_help_text() {
    use accesskit::Role;
    let s = matrix_desktop::accessibility::install_offer::Snapshot {
        generation: 1,
        dialog: Some([None; 3]),
        ..Default::default()
    };
    let tree = s.tree().unwrap();
    // window + dialog + 3 buttons, nothing else
    assert_eq!(tree.nodes.len(), 5);
    for (_, n) in &tree.nodes {
        assert!(n.description().is_none() && n.value().is_none(), "help text on {:?}", n.role());
        assert!(n.label().is_some(), "unlabelled {:?}", n.role());
    }
    let buttons = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Button).count();
    assert_eq!(buttons, 3);
}

/// Bug: the entry is installed under HOME/.local/share but looked up under
/// XDG_DATA_HOME, so with that variable set the proposal returns at every launch.
#[test]
fn xdg_data_home_is_where_the_entry_is_installed() {
    let (mut env, root) = sandbox("xdg");
    env.xdg_data_home = Some(root.join("data"));
    // The public installer reads the process XDG_DATA_HOME; no other test here reads the process env.
    unsafe { std::env::set_var("XDG_DATA_HOME", env.xdg_data_home.as_ref().unwrap()) };
    install_desktop_entry(env.home.as_ref().unwrap(), env.exec.as_deref().unwrap(), IconChoice::default()).unwrap();
    assert_eq!(entry_status(&env), EntryStatus::Ok);
    assert!(!should_prompt(&env, false));
}
