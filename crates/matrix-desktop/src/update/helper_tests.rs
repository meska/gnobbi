//! Windows helper protocol with injected link/move/spawn: no test starts a process or touches
//! a real exe. The native Windows behaviour (handle wait, `MoveFileExW`, a busy exe) is NOT
//! exercised here.
use super::{
    helper::{self, Job, Request},
    stage::Ready,
    startup::{self, Pending},
};
use crate::AppDirs;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
};

struct Fx {
    dirs: AppDirs,
    helper: PathBuf,
    req: Request,
    ready: Ready,
}

fn fixture(name: &str) -> Fx {
    let root = std::env::temp_dir().join(format!("gnobbi-helper-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    let dirs = AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
    let target = dir.join("gnobbi.exe");
    std::fs::write(&target, b"old").unwrap();
    let new = dir.join(".gnobbi.9.1.0.new");
    std::fs::write(&new, b"new").unwrap();
    let sha256 = Sha256::digest(b"new").iter().map(|b| format!("{b:02x}")).collect();
    let ready = Ready { version: String::new(), build: 9, path: new, size: 3, sha256 };
    let req = Request { target, job: Job::Install { ready: ready.clone() }, parent: 4242 };
    Fx { dirs, helper: dir.join(".gnobbi-helper-4242-1.exe"), req, ready }
}

impl Fx {
    fn target(&self) -> &Path {
        &self.req.target
    }
    fn prev(&self) -> PathBuf {
        super::install::prev_of(self.target())
    }
    fn marker(&self) -> PathBuf {
        self.dirs.data.join(startup::MARKER)
    }
    fn text(&self, p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }
    fn run(&self, link: impl Fn(&Path, &Path) -> io::Result<()>, mv: impl Fn(&Path, &Path) -> io::Result<()>, spawn: impl Fn(&Path) -> io::Result<()>) -> Result<(), String> {
        helper::execute(&self.dirs, &self.helper, &self.req, link, mv, spawn)
    }
}

fn link(a: &Path, b: &Path) -> io::Result<()> {
    std::fs::hard_link(a, b)
}
fn mv(a: &Path, b: &Path) -> io::Result<()> {
    std::fs::rename(a, b)
}
fn ok(_: &Path) -> io::Result<()> {
    Ok(())
}
fn args(req: &Request) -> Vec<String> {
    req.to_args().into_iter().map(|a| a.into_string().unwrap()).collect()
}

#[test]
fn request_round_trips_through_argv() {
    let fx = fixture("argv");
    assert_eq!(Request::parse(&args(&fx.req)).unwrap(), fx.req);
    let restore = Request { job: Job::Restore, ..fx.req.clone() };
    assert_eq!(Request::parse(&args(&restore)).unwrap(), restore);
}

#[test]
fn malformed_argv_is_refused() {
    let fx = fixture("badargv");
    let good = args(&fx.req);
    let mut bad_sha = good.clone();
    bad_sha[7] = "ABC".into();
    let mut bad_pid = good.clone();
    bad_pid[3] = "-1".into();
    let mut extra = args(&Request { job: Job::Restore, ..fx.req.clone() });
    extra.push("x".into());
    for a in [bad_sha, bad_pid, extra, good[..5].to_vec(), vec![helper::FLAG.into()], vec![]] {
        let e = Request::parse(&a).unwrap_err();
        assert_eq!(e, "Aggiornamento non riuscito: parametri dell'helper non validi", "{a:?}");
    }
}

#[test]
fn validation_pins_every_path_to_the_helper_folder_and_our_names() {
    let fx = fixture("validate");
    assert!(helper::validate(&fx.helper, &fx.req).is_ok());
    let dir = fx.helper.parent().unwrap().to_path_buf();
    let other = fx.dirs.data.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let with_ready = |p: PathBuf| Request { job: Job::Install { ready: Ready { path: p, ..fx.ready.clone() } }, ..fx.req.clone() };
    std::fs::write(dir.join("evil.new"), b"new").unwrap();
    std::fs::write(other.join(".gnobbi.9.new"), b"new").unwrap();
    std::fs::write(other.join("gnobbi.exe"), b"x").unwrap();
    let cases = [
        ("helper name", helper::validate(&dir.join("gnobbi.exe"), &fx.req)),
        ("staging name", helper::validate(&fx.helper, &with_ready(dir.join("evil.new")))),
        ("staging folder", helper::validate(&fx.helper, &with_ready(other.join(".gnobbi.9.new")))),
        ("staging dotdot", helper::validate(&fx.helper, &with_ready(dir.join("..").join("app").join(".gnobbi.9.1.0.new")))),
        ("target folder", helper::validate(&fx.helper, &Request { target: other.join("gnobbi.exe"), ..fx.req.clone() })),
        ("target is helper", helper::validate(&fx.helper, &Request { target: fx.helper.clone(), ..fx.req.clone() })),
        ("target not exe", helper::validate(&fx.helper, &Request { target: dir.join("gnobbi.AppImage"), ..fx.req.clone() })),
    ];
    for (what, r) in cases {
        let e = r.expect_err(what);
        assert!(e.starts_with("Aggiornamento non riuscito: ") && e.lines().count() == 1, "{what}: {e}");
    }
    std::fs::write(fx.prev(), b"older").unwrap();
    assert!(helper::validate(&fx.helper, &fx.req).unwrap_err().contains("rimuovi"));
}

#[test]
fn install_makes_backup_marker_and_one_move() {
    let fx = fixture("ok");
    let moves = Cell::new(0);
    let started = Cell::new(false);
    fx.run(link, |a, b| { moves.set(moves.get() + 1); assert_eq!((a, b), (fx.ready.path.as_path(), fx.target())); mv(a, b) }, |t| { started.set(t == fx.target()); Ok(()) }).unwrap();
    assert_eq!((moves.get(), started.get()), (1, true), "exactly one replacing move, then the target is started");
    assert_eq!(fx.text(fx.target()), "new");
    assert_eq!(fx.text(&fx.prev()), "old");
    assert_eq!(startup::pending(&fx.dirs).map(|p| (p.to_build, p.attempts)), Some((9, 0)));
}

#[test]
fn a_busy_target_fails_safely_without_a_second_rename() {
    let fx = fixture("busy");
    let moves = Cell::new(0);
    let e = fx.run(link, |_, _| { moves.set(moves.get() + 1); Err(io::Error::from(io::ErrorKind::PermissionDenied)) }, ok).unwrap_err();
    assert_eq!(e, "Aggiornamento non riuscito: cartella non scrivibile");
    assert_eq!(moves.get(), 1, "no fallback move");
    assert_eq!(fx.text(fx.target()), "old");
    assert!(!fx.prev().exists() && !fx.marker().exists(), "our own backup and marker are withdrawn");
    assert!(fx.ready.path.exists(), "staging stays for a retry");
}

#[test]
fn a_move_that_loses_the_target_keeps_the_only_copy() {
    let fx = fixture("lost");
    let e = fx.run(link, |_, b| { std::fs::remove_file(b).unwrap(); Err(io::Error::other("boom")) }, ok).unwrap_err();
    assert_eq!(e, "Aggiornamento non riuscito: posizione non riuscita e ripristino non riuscito");
    assert_eq!(fx.text(&fx.prev()), "old");
    assert!(fx.marker().exists());
}

#[test]
fn a_filesystem_without_hard_links_stops_before_the_move() {
    let fx = fixture("nolink");
    let moves = Cell::new(0);
    let e = fx.run(|_, _| Err(io::Error::other("hard link non supportato")), |a, b| { moves.set(1); mv(a, b) }, ok).unwrap_err();
    assert!(e.contains("hard link"), "{e}");
    assert_eq!(moves.get(), 0);
    assert_eq!(fx.text(fx.target()), "old");
    assert!(!fx.marker().exists());
}

#[test]
fn a_tampered_staging_is_refused_before_anything_is_written() {
    let fx = fixture("tamper");
    std::fs::write(&fx.ready.path, b"evil").unwrap();
    let e = fx.run(link, mv, ok).unwrap_err();
    assert!(e.starts_with("Aggiornamento non riuscito: staging"), "{e}");
    assert!(!fx.prev().exists() && !fx.marker().exists());
    assert_eq!(fx.text(fx.target()), "old");
}

#[test]
fn a_new_build_that_does_not_start_is_rolled_back() {
    let fx = fixture("spawnfail");
    let e = fx.run(link, mv, |_| Err(io::Error::other("no avvio"))).unwrap_err();
    assert_eq!(e, "Aggiornamento non riuscito: no avvio");
    assert_eq!(fx.text(fx.target()), "old");
    assert!(!fx.prev().exists() && !fx.marker().exists());
}

#[test]
fn a_failed_rollback_keeps_prev_and_marker() {
    let fx = fixture("rollbackfail");
    let n = Cell::new(0);
    let e = fx.run(link, |a, b| { n.set(n.get() + 1); if n.get() == 1 { mv(a, b) } else { Err(io::Error::other("busy")) } }, |_| Err(io::Error::other("no avvio"))).unwrap_err();
    assert_eq!(e, "Aggiornamento non riuscito: avvio non riuscito e ripristino non riuscito");
    assert_eq!(fx.text(&fx.prev()), "old");
    assert!(fx.marker().exists());
}

#[test]
fn a_second_helper_is_refused_while_the_lock_is_held() {
    let fx = fixture("lock");
    let refused = Cell::new(false);
    fx.run(link, mv, |_| { refused.set(super::install::Lock::acquire(fx.target()).is_err()); Ok(()) }).unwrap();
    assert!(refused.get());
}

#[test]
fn restore_needs_the_marker_and_two_failed_starts() {
    let fx = fixture("restore");
    std::fs::write(fx.prev(), b"old").unwrap();
    std::fs::write(fx.target(), b"bad").unwrap();
    let restore = Request { job: Job::Restore, ..fx.req.clone() };
    let run = |mvs: &Cell<u32>| helper::execute(&fx.dirs, &fx.helper, &restore, link, |a, b| { mvs.set(mvs.get() + 1); mv(a, b) }, ok);
    let mvs = Cell::new(0);
    assert!(run(&mvs).unwrap_err().contains("nessun ripristino"));
    startup::write_pending(&fx.dirs, &Pending { from_build: 8, to_build: 9, attempts: 1 }).unwrap();
    assert!(run(&mvs).is_err());
    assert_eq!(mvs.get(), 0, "no move without the authority of a twice-failed marker");
    startup::write_pending(&fx.dirs, &Pending { from_build: 8, to_build: 9, attempts: 2 }).unwrap();
    run(&mvs).unwrap();
    assert_eq!(mvs.get(), 1);
    assert!(startup::bad_builds(&fx.dirs).contains(&9) && !fx.marker().exists());
    assert_eq!(startup::update_outcome(&fx.dirs), Some((startup::OUTCOME_RESTORED.into(), 8)));
}

#[test]
fn startup_hands_the_restore_to_the_helper_without_touching_files() {
    let fx = fixture("startup");
    std::fs::write(fx.prev(), b"old").unwrap();
    startup::write_pending(&fx.dirs, &Pending { from_build: 8, to_build: 9, attempts: 2 }).unwrap();
    let act = startup::handle_marker_with(&fx.dirs, 9, fx.target(), &fx.prev(), true);
    assert_eq!(act, startup::StartupAction::RestoreViaHelper { to_build: 9 });
    assert_eq!((fx.text(fx.target()), fx.text(&fx.prev())), ("old".into(), "old".into()));
    assert!(fx.marker().exists() && startup::bad_builds(&fx.dirs).is_empty());
}

#[test]
fn cleanup_removes_only_finished_copies_with_our_exact_name() {
    let fx = fixture("cleanup");
    let dir = fx.helper.parent().unwrap();
    for n in [".gnobbi-helper-1-2.exe", ".gnobbi-helper-33-444.exe"] {
        std::fs::write(dir.join(n), b"x").unwrap();
    }
    for n in ["gnobbi-helper-1-2.exe", ".gnobbi-helper-a-2.exe", ".gnobbi-helper-1-2.exe.bak", ".gnobbi-helper-1.exe"] {
        std::fs::write(dir.join(n), b"x").unwrap();
    }
    std::fs::create_dir(dir.join(".gnobbi-helper-5-6.exe")).unwrap();
    helper::cleanup_stale(fx.target());
    let mut left: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    left.sort();
    assert_eq!(left, [".gnobbi-helper-1-2.exe.bak", ".gnobbi-helper-1.exe", ".gnobbi-helper-5-6.exe", ".gnobbi-helper-a-2.exe", ".gnobbi.9.1.0.new", "gnobbi-helper-1-2.exe", "gnobbi.exe"]);
}

#[test]
fn dispatch_ignores_everything_but_its_flag_and_self_test_stays_pure() {
    assert_eq!(helper::dispatch(&[]), None);
    assert_eq!(helper::dispatch(&["--self-test".into()]), None);
    assert_eq!(helper::dispatch(&["--install-desktop-entry".into()]), None);
    assert!(helper::handshake_ok("READY\r\n") && !helper::handshake_ok("") && !helper::handshake_ok("ready?"));
}

#[cfg(not(windows))]
#[test]
fn the_helper_mode_is_refused_off_windows() {
    assert_eq!(helper::dispatch(&[helper::FLAG.into(), "restore".into()]), Some(1));
}

// Windows answers NotFound (not NotADirectory) for a path under a file: no such error to provoke.
#[cfg(unix)]
#[test]
fn restore_reports_an_unreadable_target_instead_of_hiding_it_as_missing_backup() {
    let fx = fixture("restore-error");
    let dir = fx.target().parent().unwrap().to_path_buf();
    // A file where a folder should be: the metadata error is not "not found".
    std::fs::write(dir.join("blocked.exe"), b"x").unwrap();
    let target = dir.join("blocked.exe").join("gnobbi.exe");
    let req = Request { target: target.clone(), job: Job::Restore, parent: 4242 };
    let err = helper::validate(&dir.join("blocked.exe").join(".gnobbi-helper-4242-1.exe"), &req).unwrap_err();
    assert!(err.starts_with("Aggiornamento non riuscito: ") && !err.contains("non trovato") && err.lines().count() == 1, "{err}");
}
