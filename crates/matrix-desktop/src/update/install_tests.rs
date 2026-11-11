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
        install::install_with(&self.dirs, &self.channel, &self.ready, Fx::link, Fx::rename, spawn, |p| std::fs::remove_file(p))
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
    assert_eq!(err, "Aggiornamento non riuscito: rimuovi gnobbi.AppImage.prev");
    assert_eq!(fx.read(&fx.paths.prev), "older");
    assert_eq!(fx.read(&fx.paths.target), "old");
}

// On Windows the same call starts the real helper (covered by the next test's pre-checks).
#[cfg(not(windows))]
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
    install::swap(&fx.paths, Fx::link, || Ok(()), Fx::rename).unwrap();
    assert_eq!(fx.read(&fx.paths.target), "new");
    assert_eq!(fx.read(&fx.paths.prev), "old");
    assert!(!fx.paths.new.exists(), "the staging path is gone: it was renamed onto the target");
}

#[test]
fn backup_failure_leaves_the_target_untouched() {
    let fx = fixture("swap-link");
    let err = install::swap(&fx.paths, |_, _| Err(std::io::Error::other("no link")), || Ok(()), Fx::rename).unwrap_err();
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
    let err = install::swap(&fx.paths, Fx::link, || Ok(()), rename).unwrap_err();
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
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Ok(()), |p| std::fs::remove_file(p)).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: busy");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists(), "the aborted transaction left no orphan backup");
    assert!(!fx.marker().exists());
}

#[test]
fn failed_rollback_keeps_the_recoverable_copy() {
    let fx = fixture("swap-lost");
    let err = install::swap(&fx.paths, Fx::link, || Ok(()), |_, _| Err(std::io::Error::other("busy"))).unwrap_err();
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
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Err(std::io::Error::other("no start")), |p| std::fs::remove_file(p)).unwrap_err();
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
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)), Fx::rename, |_| Ok(()), |p| std::fs::remove_file(p)).unwrap_err();
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
fn changed_staging_is_removed_after_reverification_fails() {
    let fx = fixture("changed-cleanup");
    std::fs::write(&fx.paths.new, b"bad").unwrap();
    let err = fx.run_with_spawn(|_| panic!("must not spawn")).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: staging con hash diverso");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.new.exists(), "invalid staging must be removed");
    assert!(!fx.marker().exists());
}

#[test]
fn pending_transaction_is_not_overwritten() {
    let fx = fixture("pending-existing");
    super::startup::write_pending(&fx.dirs, &super::startup::Pending { from_build: 1, to_build: 2, attempts: 1 }).unwrap();
    let original = fx.read(&fx.marker());
    let spawned = Cell::new(false);
    let result = fx.run_with_spawn(|_| { spawned.set(true); Ok(()) });
    assert_eq!(fx.read(&fx.marker()), original, "pending transaction must remain unchanged");
    assert!(!spawned.get(), "must not spawn during an existing transaction");
    assert!(result.is_err(), "existing transaction must be refused");
    assert_eq!(fx.read(&fx.marker()), original);
    assert_eq!(fx.read(&fx.paths.target), "old");
}

#[test]
fn permission_denied_during_recovery_is_exact() {
    let fx = fixture("recovery-perm");
    let calls = Cell::new(0);
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, |a, b| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 { Err(std::io::ErrorKind::PermissionDenied.into()) } else { Fx::rename(a, b) }
    }, |_| Err(std::io::Error::other("spawn")), |p| std::fs::remove_file(p)).unwrap_err();
    assert_eq!(fx.read(&fx.paths.prev), "old");
    assert!(fx.marker().exists());
    assert_eq!(err, "Aggiornamento non riuscito: cartella non scrivibile");
}

#[test]
fn errors_are_always_one_line() {
    let fx = fixture("lines");
    let stale = Ready { sha256: "0".repeat(64), ..fx.ready.clone() };
    for err in [
        install::install_staged(&fx.dirs, &fx.channel, &stale).unwrap_err(),
        install::install_with(&fx.dirs, &fx.channel, &fx.ready, |_, _| Err(std::io::Error::other("a\nb")), Fx::rename, |_| Ok(()), |p| std::fs::remove_file(p)).unwrap_err(),
    ] {
        assert!(err.starts_with("Aggiornamento non riuscito: "), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}

#[test]
fn order_is_backup_then_marker_then_the_single_rename() {
    let fx = fixture("order");
    let log = std::cell::RefCell::new(Vec::new());
    install::swap(
        &fx.paths,
        |a, b| { log.borrow_mut().push("link"); Fx::link(a, b) },
        || { log.borrow_mut().push("mark"); assert!(fx.paths.prev.exists(), "no marker without its backup"); Ok(()) },
        |a, b| { log.borrow_mut().push("rename"); Fx::rename(a, b) },
    )
    .unwrap();
    assert_eq!(*log.borrow(), ["link", "mark", "rename"]);
}

#[test]
fn install_is_refused_while_another_process_holds_the_lock() {
    // A marker that appears between the first check and the swap is caught by the check
    // under the lock: the spawn-time marker is the only one this install may leave.
    let fx = fixture("pending-under-lock");
    let _held = install::Lock::acquire(&fx.paths.target).unwrap();
    let err = fx.run_with_spawn(|_| panic!("must not spawn")).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: altra installazione in corso");
    assert!(!fx.paths.prev.exists() && !fx.marker().exists());
    assert_eq!(fx.read(&fx.paths.target), "old");
}

fn deny(_: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

// rename(prev, target) on the same inode is a no-op only on unix; on Windows it moves prev away.
#[cfg(unix)]
#[test]
fn failed_cleanup_after_a_rollback_is_reported_and_keeps_the_marker_with_the_backup() {
    let fx = fixture("cleanup-fail");
    let calls = Cell::new(0);
    // Place fails, the rollback rename "succeeds" as a no-op on the same inode: prev stays.
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { Err(std::io::Error::other("busy")) } else { Fx::rename(a, b) }
    };
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Ok(()), deny).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: busy", "the failed cleanup goes to the log, not into the line");
    assert_eq!(err.lines().count(), 1);
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert_eq!(fx.read(&fx.paths.prev), "old", "backup not removed, still reported");
    assert!(fx.marker().exists(), "the marker is dropped only after the backup is gone");
}

// Windows has no file-identity proof (`same_file` is always true there, see install.rs).
#[cfg(unix)]
#[test]
fn a_backup_that_is_not_the_target_is_never_deleted_by_cleanup() {
    let fx = fixture("cleanup-other");
    std::fs::write(&fx.paths.prev, b"different").unwrap();
    let paths = fx.paths.clone();
    let err = install::drop_backup(&paths, |_| panic!("must not remove")).unwrap_err();
    assert!(err.to_string().contains("conservato"), "{err}");
    assert_eq!(fx.read(&paths.prev), "different");
}

#[test]
fn cleanup_leaves_a_marker_of_another_transaction_alone() {
    let fx = fixture("cleanup-foreign");
    super::startup::write_pending(&fx.dirs, &super::startup::Pending { from_build: 1, to_build: 77, attempts: 1 }).unwrap();
    let msg = install::cleanup(&fx.dirs, &fx.paths, 9, &|_: &Path| panic!("must not remove"), "base".into());
    assert_eq!(msg, "base");
    assert!(fx.marker().exists());
}

// --- install_and_restart in a child process: the test binary re-runs itself ---------------

const CHILD_ENV: &str = "GNOBBI_INSTALL_CHILD";

/// Runs only as the child of `run_child`; in the parent run it is a no-op.
#[cfg(unix)]
#[test]
fn child_entry() {
    let Ok(mode) = std::env::var(CHILD_ENV) else { return };
    let fx = fixture(&format!("child-{mode}"));
    if mode == "ok" {
        // A target that is a real, executable, harmless program: the relaunch really starts it.
        std::fs::write(&fx.paths.new, b"#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fx.paths.new, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let ready = if mode == "ok" { Ready { size: 17, sha256: sha256(b"#!/bin/sh\nexit 0\n"), ..fx.ready.clone() } } else { Ready { sha256: "0".repeat(64), ..fx.ready.clone() } };
    // On success the process exits 0 inside the call; reaching the next line means it did not.
    let err = install::install_and_restart(&fx.dirs, &fx.channel, &ready).unwrap_err();
    println!("returned: {err}");
    std::process::exit(42);
}

#[cfg(unix)]
fn run_child(mode: &str) -> std::process::Output {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "update::install_tests::child_entry", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, mode)
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn install_and_restart_returns_the_error_and_does_not_exit() {
    let out = run_child("fail");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(42), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("returned: Aggiornamento non riuscito: staging con hash diverso"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn install_and_restart_exits_zero_only_after_the_new_build_started() {
    let out = run_child("ok");
    assert_eq!(out.status.code(), Some(0), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("returned:"));
}

// --- QA: gaps found against the task checklist -----------------------------

#[test]
fn missing_staging_is_refused_before_anything_is_written() {
    let fx = fixture("no-staging");
    std::fs::remove_file(&fx.paths.new).unwrap();
    let err = install::install_staged(&fx.dirs, &fx.channel, &fx.ready).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: staging non trovato");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists() && !fx.marker().exists());
}

#[test]
fn the_order_is_marker_then_swap_then_relaunch_of_the_new_target() {
    let fx = fixture("order-full");
    let seen = Cell::new(false);
    fx.run_with_spawn(|t| {
        // At relaunch time the swap is done and the marker is on disk; the exit comes later.
        assert_eq!(std::fs::read_to_string(t).unwrap(), "new");
        assert!(fx.marker().exists() && fx.paths.prev.exists());
        seen.set(true);
        Ok(())
    })
    .unwrap();
    assert!(seen.get());
}

#[cfg(unix)]
#[test]
fn the_installed_target_is_executable_0755_even_if_the_staging_was_not() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture("mode");
    std::fs::set_permissions(&fx.paths.new, std::fs::Permissions::from_mode(0o600)).unwrap();
    fx.run_with_spawn(|_| Ok(())).unwrap();
    let mode = std::fs::metadata(&fx.paths.target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
}

#[test]
fn permission_denied_on_the_staging_rename_is_the_exact_line_and_cleans_up() {
    let fx = fixture("perm-rename");
    let calls = Cell::new(0);
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { Err(std::io::ErrorKind::PermissionDenied.into()) } else { Fx::rename(a, b) }
    };
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| panic!("must not spawn"), |p| std::fs::remove_file(p)).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: cartella non scrivibile");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(!fx.paths.prev.exists() && !fx.marker().exists(), "no orphan backup or marker");
    assert!(fx.paths.new.exists(), "the staging is not consumed by a failed install");
}

#[test]
fn a_failed_spawn_with_a_failed_cleanup_reports_it_and_keeps_the_marker() {
    let fx = fixture("spawn-cleanup");
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, Fx::rename, |_| Err(std::io::Error::other("no start")), deny).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: no start");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(fx.marker().exists(), "the marker outlives a backup that could not be removed");
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_as_backup_counts_as_a_preexisting_backup() {
    let fx = fixture("prev-dangling");
    std::os::unix::fs::symlink("/nonexistent/x", &fx.paths.prev).unwrap();
    let err = install::install_staged(&fx.dirs, &fx.channel, &fx.ready).unwrap_err();
    assert!(err.contains("rimuovi"), "{err}");
    assert_eq!(fx.read(&fx.paths.target), "old");
    assert!(fx.paths.prev.symlink_metadata().is_ok(), "the symlink is left alone");
}

#[test]
fn windows_parent_refuses_a_pending_marker_or_altered_staging_before_starting_the_helper() {
    let fx = fixture("win-precheck");
    let channel = Channel::WindowsPortable { exe: fx.paths.target.clone() };
    // Without the pre-check the answer would be the helper's "aggiornamento Windows non disponibile".
    super::startup::write_pending(&fx.dirs, &super::startup::Pending { from_build: 1, to_build: 9, attempts: 0 }).unwrap();
    let err = install::install_staged(&fx.dirs, &channel, &fx.ready).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: aggiornamento precedente in sospeso");
    std::fs::remove_file(fx.marker()).unwrap();
    std::fs::write(&fx.paths.new, b"bad").unwrap();
    let err = install::install_staged(&fx.dirs, &channel, &fx.ready).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: staging con hash diverso");
    assert!(!fx.paths.new.exists(), "altered staging is dropped");
    assert_eq!(fx.read(&fx.paths.target), "old");
}

#[cfg(unix)]
#[test]
fn an_orphan_backup_on_the_same_inode_is_dropped_and_the_install_proceeds() {
    let fx = fixture("prev-orphan");
    std::fs::hard_link(&fx.paths.target, &fx.paths.prev).unwrap();
    install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, Fx::rename, |_| Ok(()), |p| std::fs::remove_file(p)).unwrap();
    assert_eq!(fx.read(&fx.paths.target), "new");
    assert_eq!(fx.read(&fx.paths.prev), "old", "a fresh backup of the old build");
    assert!(fx.marker().exists());
}

#[test]
fn permission_denied_stays_the_exact_line_even_when_the_cleanup_fails_too() {
    let fx = fixture("perm-cleanup");
    let calls = Cell::new(0);
    let rename = |a: &Path, b: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { Err(std::io::ErrorKind::PermissionDenied.into()) } else { Fx::rename(a, b) }
    };
    let err = install::install_with(&fx.dirs, &fx.channel, &fx.ready, Fx::link, rename, |_| Ok(()), deny).unwrap_err();
    assert_eq!(err, "Aggiornamento non riuscito: cartella non scrivibile");
}
