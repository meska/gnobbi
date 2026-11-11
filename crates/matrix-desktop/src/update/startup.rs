//! First thing in `main()`: `--self-test`, then the `update-pending.json` marker (AUTO_UPDATE §1.5).
//! File logic takes its paths as parameters; only the `*_real` wrappers resolve the real ones.
//! Persistence (not fixed by the plan, simplest choice): marker in `dirs.data`; `bad_builds` (u64 list)
//! and `update_outcome` ({"result": "updated"|"restored", "build": n}) are keys of `settings.json`
//! via `update_settings`, the store `Preferences` already shares. The UI shows the outcome once and clears it.
use super::install::{self, Lock};
use crate::{AppDirs, preferences::{read_settings, update_settings}};
use serde_json::{Value, json};
use std::{io::Write, path::{Path, PathBuf}};

pub const MARKER: &str = "update-pending.json";
pub const OUTCOME_UPDATED: &str = "updated";
pub const OUTCOME_RESTORED: &str = "restored";
/// The Windows helper could not install after the app had exited; `build` is the wanted build.
pub const OUTCOME_FAILED: &str = "failed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub from_build: u64,
    pub to_build: u64,
    pub attempts: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupAction {
    /// No marker, or an update still being tried: start normally.
    Continue,
    /// Stale or unreadable marker, removed.
    Ignored,
    /// `.prev` is back in place and `to_build` is in `bad_builds`: relaunch the old binary.
    Restored { to_build: u64 },
    /// Windows: the exe to put back is the running one, so only the helper can do it.
    RestoreViaHelper { to_build: u64 },
}

/// `Some(line)` when the first argument is `--self-test`. Pure: touches no file.
pub fn self_test_line(args: &[String], build: &str) -> Option<String> {
    (args.first().map(String::as_str) == Some("--self-test")).then(|| format!("gnobbi build={build}"))
}

pub(super) fn marker_path(dirs: &AppDirs) -> PathBuf {
    dirs.data.join(MARKER)
}

fn parse_pending(bytes: &[u8]) -> Option<Pending> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let n = |k: &str| v.get(k)?.as_u64();
    Some(Pending { from_build: n("from_build")?, to_build: n("to_build")?, attempts: n("attempts")? })
}

fn read_pending(dirs: &AppDirs) -> Option<Option<Pending>> {
    match std::fs::read(marker_path(dirs)) {
        Ok(b) => Some(parse_pending(&b)),
        Err(_) => None,
    }
}

/// Same approach as `update_settings`: temp file in the same folder, owner-only, rename.
pub fn write_pending(dirs: &AppDirs, p: &Pending) -> std::io::Result<()> {
    std::fs::create_dir_all(&dirs.data)?;
    let tmp = dirs.data.join(format!("{MARKER}.tmp"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&tmp)?.write_all(json!({ "from_build": p.from_build, "to_build": p.to_build, "attempts": p.attempts }).to_string().as_bytes())?;
    std::fs::rename(tmp, marker_path(dirs))
}

/// Persistent `bad_builds`, never proposed again (I3).
pub fn bad_builds(dirs: &AppDirs) -> Vec<u64> {
    let map = read_settings(dirs);
    map.get("bad_builds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

pub fn add_bad_build(dirs: &AppDirs, build: u64) -> std::io::Result<()> {
    let mut list = bad_builds(dirs);
    if !list.contains(&build) {
        list.push(build);
    }
    update_settings(dirs, |m| {
        m.insert("bad_builds".into(), json!(list));
    })
}

/// Outcome to show once: `(OUTCOME_UPDATED, to_build)` or `(OUTCOME_RESTORED, from_build)`.
pub fn update_outcome(dirs: &AppDirs) -> Option<(String, u64)> {
    let map = read_settings(dirs);
    let o = map.get("update_outcome")?;
    Some((o.get("result")?.as_str()?.to_owned(), o.get("build")?.as_u64()?))
}

/// Reads and clears the stored outcome: it is shown once and never reappears.
pub fn take_outcome(dirs: &AppDirs) -> Option<(String, u64)> {
    let o = update_outcome(dirs)?;
    if let Err(e) = update_settings(dirs, |m| {
        m.remove("update_outcome");
    }) {
        eprintln!("update: outcome not cleared: {e}");
    }
    Some(o)
}

pub(super) fn save_outcome(dirs: &AppDirs, result: &str, build: u64) -> std::io::Result<()> {
    update_settings(dirs, |m| {
        m.insert("update_outcome".into(), json!({ "result": result, "build": build }));
    })
}

/// Decision + file work of the startup marker. `build` is the running build already parsed.
pub fn handle_marker(dirs: &AppDirs, build: u64, target: &Path, prev: &Path) -> StartupAction {
    handle_marker_with(dirs, build, target, prev, cfg!(windows))
}

pub(super) fn handle_marker_with(dirs: &AppDirs, build: u64, target: &Path, prev: &Path, via_helper: bool) -> StartupAction {
    if read_pending(dirs).is_none() {
        return StartupAction::Continue;
    }
    // Same lock as the installer: while another process owns the transaction, nothing here
    // may count, restore or delete. On Windows it is released on return, before the helper runs.
    let Ok(_lock) = Lock::acquire(target) else { return StartupAction::Continue };
    // Read again under the lock: the first read proved nothing.
    let pending = match read_pending(dirs) {
        None => return StartupAction::Continue,
        Some(Some(p)) if p.to_build == build => p,
        // Provably obsolete: this build is newer than the one the marker waits for.
        Some(Some(p)) if p.to_build < build => {
            if let Err(e) = std::fs::remove_file(marker_path(dirs)) {
                eprintln!("update: obsolete marker not removed: {e}");
            }
            return StartupAction::Ignored;
        }
        // Another build's transaction (an older instance) or an unreadable marker: not ours,
        // not provably dead, so nothing is deleted.
        Some(_) => return StartupAction::Ignored,
    };
    let attempts = pending.attempts.saturating_add(1);
    if attempts <= 2 {
        if let Err(e) = write_pending(dirs, &Pending { attempts, ..pending }) {
            eprintln!("update: marker not updated: {e}");
        }
        return StartupAction::Continue;
    }
    // Third start without confirmation: put the old binary back.
    if via_helper {
        return StartupAction::RestoreViaHelper { to_build: pending.to_build };
    }
    // Revalidated under the lock: restoring needs both files to be plain files.
    if !matches!((install::regular(target), install::regular(prev)), (Ok(true), Ok(true))) {
        eprintln!("update: restore skipped: target or backup missing");
        return StartupAction::Continue;
    }
    if let Err(e) = restore(target, prev) {
        eprintln!("update: restore failed: {e}");
        return StartupAction::Continue;
    }
    finish_restore(dirs, &pending);
    StartupAction::Restored { to_build: pending.to_build }
}

/// Bookkeeping after the old binary is back; the marker goes last.
pub(super) fn finish_restore(dirs: &AppDirs, pending: &Pending) {
    if let Err(e) = add_bad_build(dirs, pending.to_build).and_then(|()| save_outcome(dirs, OUTCOME_RESTORED, pending.from_build)) {
        eprintln!("update: state not saved: {e}");
    }
    let _ = std::fs::remove_file(marker_path(dirs));
}

/// The readable marker, if any.
pub(super) fn pending(dirs: &AppDirs) -> Option<Pending> {
    read_pending(dirs).flatten()
}

/// Puts the backup back: a single rename over the target, so the executable name is never
/// free in between. No second-rename fallback: it is destructive on every platform and can
/// leave the target missing (INSTALL_STAGED_CONTRACT.md §3).
fn restore(target: &Path, prev: &Path) -> std::io::Result<()> {
    std::fs::rename(prev, target)
}

/// Deletes the marker of the transaction this process abandoned (`to_build` is its identity):
/// the marker only, with the caller holding the install lock. Another build's marker, or
/// none, is left alone. `prev` and the staging stay (INSTALL_STAGED_CONTRACT.md §5).
pub(super) fn abort_pending(dirs: &AppDirs, to_build: u64, remove: impl Fn(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    match read_pending(dirs) {
        Some(Some(p)) if p.to_build == to_build => remove(&marker_path(dirs)),
        _ => Ok(()),
    }
}

/// Called after the first frame or the sign-in screen: the new build works. Facade of
/// [`confirm_update_for`] with this process's build; the target is `prev` minus `.prev`.
pub fn confirm_update_at(dirs: &AppDirs, prev: &Path) {
    let Some(target) = prev.to_str().and_then(|p| p.strip_suffix(".prev")).map(PathBuf::from) else { return };
    confirm_update_for(dirs, install::running_build(), &target);
}

/// Confirms the transaction of `build` under the install lock. Only a readable marker of that
/// very build confirms: a missing, malformed or foreign marker leaves `prev` where it is. The
/// marker goes before `prev`, and `prev` stays if the marker could not be removed.
pub fn confirm_update_for(dirs: &AppDirs, build: u64, target: &Path) {
    // The old process may still be exiting and holding the lock: wait a little, not forever.
    let mut lock = Lock::acquire(target);
    for _ in 0..40 {
        if lock.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        lock = Lock::acquire(target);
    }
    let Ok(_lock) = lock else { return eprintln!("update: confirm skipped, install lock busy") };
    let Some(Some(pending)) = read_pending(dirs) else { return };
    if pending.to_build != build {
        return;
    }
    if let Err(e) = save_outcome(dirs, OUTCOME_UPDATED, build) {
        eprintln!("update: outcome not saved: {e}");
    }
    if let Err(e) = std::fs::remove_file(marker_path(dirs)) {
        return eprintln!("update: marker not removed, backup kept: {e}");
    }
    let prev = install::prev_of(target);
    if let Err(e) = std::fs::remove_file(&prev).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
        eprintln!("update: backup not removed: {e}");
    }
}

/// Update target: Linux `~/Applications/gnobbi.AppImage` if present, else `$APPIMAGE`; Windows the running exe.
fn target_real(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::current_exe().ok();
    }
    if cfg!(target_os = "macos") {
        return None;
    }
    let fixed = env("HOME").map(|h| Path::new(&h).join("Applications/gnobbi.AppImage")).filter(|p| p.exists());
    fixed.or_else(|| env("APPIMAGE").filter(|a| !a.is_empty()).map(PathBuf::from))
}

fn dirs_real() -> Option<AppDirs> {
    crate::app_dirs(crate::Platform::CURRENT, |k| std::env::var(k).ok())
}

/// Public entry for the UI: call once after the first frame or the sign-in screen.
pub fn confirm_update() {
    if let (Some(dirs), Some(target)) = (dirs_real(), target_real(|k| std::env::var(k).ok())) {
        confirm_update_for(&dirs, install::running_build(), &target);
    }
}

/// Starts the restored binary with the same arguments. Returns the error instead of exiting:
/// a failed spawn must not look like a successful relaunch (INSTALL_STAGED_CONTRACT.md §5).
fn relaunch(target: &Path) -> std::io::Result<()> {
    let _child = std::process::Command::new(target)
        .args(std::env::args_os().skip(1))
        .env_remove("APPIMAGE")
        .env_remove("APPDIR")
        .env_remove("ARGV0")
        .spawn()?;
    Ok(())
}

/// `Some(exit code)` when the restored build was started; `None` keeps this process alive.
pub(super) fn leave_after_restore(target: &Path, spawn: impl Fn(&Path) -> std::io::Result<()>) -> Option<i32> {
    match spawn(target) {
        Ok(()) => Some(0),
        Err(e) => {
            eprintln!("update: relaunch failed: {e}");
            None
        }
    }
}

/// First call of `main()`. Ordine fisso (N1): `--self-test` esce senza toccare nulla, poi il marcatore.
pub fn run_at_start() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(line) = self_test_line(&args, crate::build_info::BUILD_INFO.build) {
        println!("{line}");
        std::process::exit(0);
    }
    // Non-numeric build = development build: no action.
    let Ok(build) = crate::build_info::BUILD_INFO.build.parse::<u64>() else { return };
    let (Some(dirs), Some(target)) = (dirs_real(), target_real(|k| std::env::var(k).ok())) else { return };
    let prev = install::prev_of(&target);
    match handle_marker(&dirs, build, &target, &prev) {
        StartupAction::Restored { .. } => {
            // The old build is back at `target`; this process is the rejected new one and leaves,
            // but only once the old one has started. If it did not, stay up: closing here would
            // leave the user with no running build.
            if let Some(code) = leave_after_restore(&target, relaunch) {
                std::process::exit(code);
            }
        }
        // The helper restores after this process is gone; if it cannot start, keep running.
        StartupAction::RestoreViaHelper { .. } => match super::helper::start(&super::helper::Request::restore(target.clone())) {
            Ok(()) => std::process::exit(0),
            Err(e) => eprintln!("update: {e}"),
        },
        _ => {}
    }
    if cfg!(windows) {
        super::helper::cleanup_stale(&target);
    }
}
