use super::startup::*;
use crate::AppDirs;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gnobbi-startup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Env { dirs: AppDirs, target: PathBuf, prev: PathBuf }

fn env(name: &str) -> Env {
    let root = tmp(name);
    let dirs = AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
    let target = root.join("gnobbi.AppImage");
    let prev = root.join("gnobbi.AppImage.prev");
    std::fs::write(&target, "new").unwrap();
    std::fs::write(&prev, "old").unwrap();
    Env { dirs, target, prev }
}

fn marker(e: &Env) -> PathBuf { e.dirs.data.join(MARKER) }
fn pend(e: &Env, to: u64, attempts: u64) {
    write_pending(&e.dirs, &Pending { from_build: 10, to_build: to, attempts }).unwrap();
}
fn start(e: &Env, build: u64) -> StartupAction { handle_marker(&e.dirs, build, &e.target, &e.prev) }

#[test]
fn confirmed_start_clears_marker_and_prev() {
    let e = env("confirm");
    pend(&e, 11, 0);
    assert_eq!(start(&e, 11), StartupAction::Continue);
    confirm_update_at(&e.dirs, &e.prev);
    assert!(!marker(&e).exists() && !e.prev.exists());
    assert_eq!(std::fs::read_to_string(&e.target).unwrap(), "new");
    assert_eq!(update_outcome(&e.dirs), Some((OUTCOME_UPDATED.into(), 11)));
}

#[test]
fn three_unconfirmed_starts_restore_prev() {
    let e = env("restore");
    pend(&e, 11, 0);
    assert_eq!(start(&e, 11), StartupAction::Continue);
    assert_eq!(start(&e, 11), StartupAction::Continue);
    assert_eq!(std::fs::read_to_string(&e.target).unwrap(), "new");
    assert_eq!(start(&e, 11), StartupAction::Restored { to_build: 11 });
    assert_eq!(std::fs::read_to_string(&e.target).unwrap(), "old");
    assert!(!marker(&e).exists());
    assert_eq!(bad_builds(&e.dirs), vec![11]);
    assert_eq!(update_outcome(&e.dirs), Some((OUTCOME_RESTORED.into(), 10)));
}

#[test]
fn marker_for_other_build_is_removed() {
    let e = env("stale");
    pend(&e, 12, 1);
    assert_eq!(start(&e, 11), StartupAction::Ignored);
    assert!(!marker(&e).exists());
    assert!(e.prev.exists() && bad_builds(&e.dirs).is_empty() && update_outcome(&e.dirs).is_none());
}

#[test]
fn no_marker_and_confirm_without_marker_do_nothing() {
    let e = env("none");
    assert_eq!(start(&e, 11), StartupAction::Continue);
    confirm_update_at(&e.dirs, &e.prev);
    assert!(e.prev.exists() && update_outcome(&e.dirs).is_none());
}

#[test]
fn restore_failure_keeps_going() {
    let e = env("nofail");
    pend(&e, 11, 2);
    std::fs::remove_file(&e.prev).unwrap();
    std::fs::remove_file(&e.target).unwrap();
    assert_eq!(start(&e, 11), StartupAction::Continue);
    assert!(bad_builds(&e.dirs).is_empty());
}

#[test]
fn self_test_flag_is_pure() {
    let a = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(self_test_line(&a(&["--self-test"]), "42").as_deref(), Some("gnobbi build=42"));
    assert_eq!(self_test_line(&a(&["--signin"]), "42"), None);
    assert_eq!(self_test_line(&[], "42"), None);
}
