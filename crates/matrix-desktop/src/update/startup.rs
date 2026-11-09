//! First thing in `main()`: `--self-test`, then the `update-pending.json` marker (AUTO_UPDATE §1.5).
//! File logic takes its paths as parameters; only the `*_real` wrappers resolve the real ones.
//! Persistence (not fixed by the plan, simplest choice): marker in `dirs.data`; `bad_builds` (u64 list)
//! and `update_outcome` ({"result": "updated"|"restored", "build": n}) are keys of `settings.json`
//! via `update_settings`, the store `Preferences` already shares. The UI shows the outcome once and clears it.
use crate::{AppDirs, preferences::{read_settings, update_settings}};
use serde_json::{Value, json};
use std::{ffi::OsString, io::Write, path::{Path, PathBuf}};

pub const MARKER: &str = "update-pending.json";
pub const OUTCOME_UPDATED: &str = "updated";
pub const OUTCOME_RESTORED: &str = "restored";

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
}

/// `Some(line)` when the first argument is `--self-test`. Pure: touches no file.
pub fn self_test_line(args: &[String], build: &str) -> Option<String> {
    (args.first().map(String::as_str) == Some("--self-test")).then(|| format!("gnobbi build={build}"))
}

fn marker_path(dirs: &AppDirs) -> PathBuf {
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
    update_settings(dirs, |m| {
        // The union is computed on the values read under the lock: a read before it would be a
        // snapshot to write back over whatever another writer committed meanwhile.
        let mut list: Vec<u64> = m
            .get("bad_builds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        if !list.contains(&build) {
            list.push(build);
        }
        m.insert("bad_builds".into(), json!(list));
    })
}

/// Outcome to show once: `(OUTCOME_UPDATED, to_build)` or `(OUTCOME_RESTORED, from_build)`.
pub fn update_outcome(dirs: &AppDirs) -> Option<(String, u64)> {
    let map = read_settings(dirs);
    let o = map.get("update_outcome")?;
    Some((o.get("result")?.as_str()?.to_owned(), o.get("build")?.as_u64()?))
}

fn save_outcome(dirs: &AppDirs, result: &str, build: u64) -> std::io::Result<()> {
    update_settings(dirs, |m| {
        m.insert("update_outcome".into(), json!({ "result": result, "build": build }));
    })
}

/// Decision + file work of the startup marker. `build` is the running build already parsed.
pub fn handle_marker(dirs: &AppDirs, build: u64, target: &Path, prev: &Path) -> StartupAction {
    let pending = match read_pending(dirs) {
        None => return StartupAction::Continue,
        Some(Some(p)) if p.to_build == build => p,
        // Different build or unparsable: obsolete.
        Some(_) => {
            let _ = std::fs::remove_file(marker_path(dirs));
            return StartupAction::Ignored;
        }
    };
    let attempts = pending.attempts.saturating_add(1);
    if attempts <= 2 {
        if let Err(e) = write_pending(dirs, &Pending { attempts, ..pending }) {
            eprintln!("update: marker not updated: {e}");
        }
        return StartupAction::Continue;
    }
    // Third start without confirmation: put the old binary back.
    if let Err(e) = restore(target, prev) {
        eprintln!("update: restore failed: {e}");
        return StartupAction::Continue;
    }
    if let Err(e) = add_bad_build(dirs, pending.to_build).and_then(|()| save_outcome(dirs, OUTCOME_RESTORED, pending.from_build)) {
        eprintln!("update: state not saved: {e}");
    }
    let _ = std::fs::remove_file(marker_path(dirs));
    StartupAction::Restored { to_build: pending.to_build }
}

fn restore(target: &Path, prev: &Path) -> std::io::Result<()> {
    match std::fs::rename(prev, target) {
        Ok(()) => Ok(()),
        // shortcut: Windows cannot replace the running exe; move it aside first. Not exercised by tests.
        Err(first) => {
            let mut aside: OsString = target.as_os_str().to_owned();
            aside.push(".bad");
            std::fs::rename(target, &aside).and_then(|()| std::fs::rename(prev, target)).map_err(|_| first)
        }
    }
}

/// Called after the first frame or the sign-in screen: the new build works.
pub fn confirm_update_at(dirs: &AppDirs, prev: &Path) {
    let Some(pending) = read_pending(dirs) else { return };
    if let Some(Err(e)) = pending.map(|p| save_outcome(dirs, OUTCOME_UPDATED, p.to_build)) {
        eprintln!("update: outcome not saved: {e}");
    }
    let _ = std::fs::remove_file(marker_path(dirs));
    let _ = std::fs::remove_file(prev);
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

fn prev_of(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_owned();
    s.push(".prev");
    PathBuf::from(s)
}

fn dirs_real() -> Option<AppDirs> {
    crate::app_dirs(crate::Platform::CURRENT, |k| std::env::var(k).ok())
}

/// Public entry for the UI: call once after the first frame or the sign-in screen.
pub fn confirm_update() {
    if let (Some(dirs), Some(target)) = (dirs_real(), target_real(|k| std::env::var(k).ok())) {
        confirm_update_at(&dirs, &prev_of(&target));
    }
}

/// Thin, untested: start the restored binary with the same arguments and leave.
fn relaunch(target: &Path) -> ! {
    let r = std::process::Command::new(target)
        .args(std::env::args_os().skip(1))
        .env_remove("APPIMAGE")
        .env_remove("APPDIR")
        .env_remove("ARGV0")
        .spawn();
    if let Err(e) = r {
        eprintln!("update: relaunch failed: {e}");
    }
    std::process::exit(0)
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
    let prev = prev_of(&target);
    if let StartupAction::Restored { .. } = handle_marker(&dirs, build, &target, &prev) {
        relaunch(&target);
    }
}
