//! Swap and transaction tests. The spawn operation is always injected, so no test starts or
//! terminates a process under test.
use super::{
    channel::Channel,
    install::{self, Paths, SwapError, Undo},
    stage::Ready,
};
use crate::AppDirs;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gnobbi-install-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

struct Fx {
    dirs: AppDirs,
    paths: Paths,
    channel: Channel,
    ready: Ready,
}

/// `Applications/gnobbi.AppImage` = "old", staging `.gnobbi.9.1.0.new` = "new", Ready for them.
fn fixture(name: &str) -> Fx {
    let root = tmp(name);
    let dirs = AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
    let dir = root.join("Applications");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("gnobbi.AppImage");
    std::fs::write(&target, b"old").unwrap();
    let new = dir.join(".gnobbi.9.1.0.new");
    std::fs::write(&new, b"new").unwrap();
    let ready = Ready { version: "1.0".into(), build: 9, path: new.clone(), size: 3, sha256: sha256(b"new") };
    let paths = Paths { target: target.clone(), new, prev: install::prev_of(&target) };
    Fx { dirs, paths, channel: Channel::AppImage { target }, ready }
}

impl Fx {
    fn marker(&self) -> PathBuf {
        self.dirs.data.join("update-pending.json")
    }
    fn read(&self, p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }
    /// Real file operations, only the spawn injected.
    fn run_with_spawn(&self, spawn: impl Fn(&Path) -> std::io::Result<()>) -> Result<(), String> {
        install::install_with(&self.dirs, &self.channel, &self.ready, Fx::link, Fx::rename, spawn)
    }
    fn link(a: &Path, b: &Path) -> std::io::Result<()> {
        std::fs::hard_link(a, b)
    }
    fn rename(a: &Path, b: &Path) -> std::io::Result<()> {
        std::fs::rename(a, b)
    }
}

#[test]
fn external_staging_is_refused_before_anything_is_written() {
    let fx = fixture("external");
    let outside = fx.paths.target.parent().unwrap().parent().unwrap().join("elsewhere.new");
    std::fs::write(&outside, b"new").unwrap();
    let ready = Ready { path: outside.clone(), ..fx.ready.clone() };
    let err = install::install_staged(&fx.dirs, &fx.channel, &ready).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: staging fuori dalla cartella del target");
    assert_eq!(fx.read(&outside), "new", "external input stays untouched");
    assert!(!fx.paths.prev.exists() && !fx.marker().exists());
}

#[test]
fn an_existing_backup_is_refused_untouched() {
    let fx = fixture("prev");
    std::fs::write(&fx.paths.prev, b"older").unwrap();
    let err = install::install_staged(&fx.dirs, &fx.channel, &fx.ready).unwrap_err();
    assert!(err.contains("backup precedente"), "{err}");
    assert_eq!(fx.read(&fx.paths.prev), "older");
    assert_eq!(fx.read(&fx.paths.target), "old");
}

#[test]
fn windows_channel_is_refused_instead_of_running_the_linux_step() {
    let fx = fixture("windows");
    let channel = Channel::WindowsPortable { exe: fx.paths.target.clone() };
    let err = install::install_staged(&fx.dirs, &channel, &fx.ready).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: aggiornamento Windows non disponibile");
    assert_eq!(fx.read(&fx.paths.target), "old");
}

#[test]
fn swap_puts_the_staging_in_place_and_consumes_it() {
    let fx = fixture("swap-ok");
    install::swap(&fx.paths, Fx::link, Fx::rename).unwrap();
    assert_eq!(fx.read(&fx.paths.target), "new");
    assert_eq!(fx.read(&fx.paths.prev), "old");
    assert!(!fx.paths.new.exists(), "the staging path is gone: it was renamed onto the target");
}

#[test]
fn backup_failure_leaves_the_target_untouched() {
    let fx = fixture("swap-link");
    let err = install::swap(&fx.paths, |_, _| Err(std::io::Error::other("no link")), Fx::rename).unwrap_err();
    assert!(matches!(err, SwapError::Backup(_)), "{err:?}");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists() && fx.paths.new.exists());
}

#[test]
fn failed_staging_rename_restores_the_old_binary() {
    let fx = fixture("swap-rename");
    let calls = Cell::new(0);
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { Err(std::io::Error::other("busy")) } else { std::fs::rename(a, b) }
    };
    let err = install::swap(&fx.paths, Fx::link, rename).unwrap_err();
    match err {
        SwapError::Place { undo: Undo::Restored, .. } => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.read(&fx.paths.target), "old");
    // A rename between two names of the same inode is a no-op on Unix, so `prev` may still be
    // there and points at the restored target; dropping it is the wrapper's job.
    if fx.paths.prev.exists() {
        assert_eq!(fx.read(&fx.paths.prev), "old");
    }
    assert!(fx.paths.new.exists(), "the failed staging is still there for the caller");
}

#[test]
fn a_failed_staging_rename_aborts_cleanly() {
    let fx = fixture("install-rename");
    let calls = Cell::new(0);
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { Err(std::io::Error::other("busy")) } else { std::fs::rename(a, b) }
    };
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Ok(())).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: busy");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists(), "the aborted transaction left no orphan backup");
    assert!(!fx.marker().exists());
}

#[test]
fn failed_rollback_keeps_the_recoverable_copy() {
    let fx = fixture("swap-lost");
    let err = install::swap(&fx.paths, Fx::link, |_, _| Err(std::io::Error::other("busy"))).unwrap_err();
    match err {
        SwapError::Place { undo: Undo::Lost(_), .. } => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert_eq!(fx.read(&fx.paths.prev), "old", "prev is the copy to recover from");
}

#[test]
fn install_keeps_marker_and_backup_until_the_new_build_confirms() {
    let fx = fixture("install-ok");
    fx.run_with_spawn(|_| Ok(())).unwrap();
    assert_eq!(fx.read(&fx.paths.target), "new");
    assert_eq!(fx.read(&fx.paths.prev), "old");
    assert!(!fx.paths.new.exists());
    let marker = fx.read(&fx.marker());
    assert!(marker.contains("\"to_build\":9") && marker.contains("\"attempts\":0"), "{marker}");
}

#[test]
fn install_rolls_back_when_the_new_binary_does_not_start() {
    let fx = fixture("install-spawn");
    let err = fx.run_with_spawn(|_| Err(std::io::Error::other("no start"))).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: no start");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists(), "the rollback consumed the backup: no orphan left");
    assert!(!fx.marker().exists(), "the aborted transaction dropped its own marker");
}

#[test]
fn install_keeps_backup_and_marker_when_that_rollback_also_fails() {
    let fx = fixture("install-lost");
    let calls = Cell::new(0);
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() >= 2 { Err(std::io::Error::other("busy")) } else { std::fs::rename(a, b) }
    };
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Err(std::io::Error::other("no start"))).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: avvio non riuscito e ripristino non riuscito");
    assert_eq!(fx.read(&fx.paths.prev), "old");
    assert!(fx.marker().exists(), "transaction data stays next to the only recoverable copy");
}

#[test]
fn a_second_install_is_refused_while_the_first_holds_the_lock() {
    let fx = fixture("lock");
    let refused = Cell::new(None);
    fx.run_with_spawn(|_| {
        refused.set(Some(install::Lock::acquire(&fx.paths.target).is_err()));
        Ok(())
    })
    .unwrap();
    assert_eq!(refused.get(), Some(true));
}

#[test]
fn permission_denied_is_the_exact_contract_line() {
    let fx = fixture("perm");
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)), Fx::rename, |_| Ok(())).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: cartella non scrivibile");
    assert!(!fx.marker().exists() && !fx.paths.prev.exists());
}

#[test]
fn a_failing_marker_write_is_reported_before_the_swap() {
    let fx = fixture("marker");
    // `dirs.data` is a file, so the marker cannot be written at all.
    std::fs::write(&fx.dirs.data, b"not a directory").unwrap();
    let err = fx.run_with_spawn(|_| Ok(())).unwrap_err();
    assert!(err.starts_with("Aggiornamento non riuscito: "), "{err}");
    assert_eq!(err.lines().count(), 1, "{err}");
    assert_eq!(fx.read(&fx.paths.target), "old", "nothing was swapped");
    assert!(!fx.paths.prev.exists());
}

#[test]
fn errors_are_always_one_line() {
    let fx = fixture("lines");
    let stale = Ready { sha256: "0".repeat(64), ..fx.ready.clone() };
    for err in [
        install::install_staged(&fx.dirs, &fx.channel, &stale).unwrap_err(),
        install::install_with(&fx.dirs, &fx.channel, &fx.ready, |_, _| Err(std::io::Error::other("a\nb")), Fx::rename, |_| Ok(())).unwrap_err(),
    ] {
        assert!(err.starts_with("Aggiornamento non riuscito: "), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}
