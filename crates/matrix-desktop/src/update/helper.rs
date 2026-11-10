//! Windows update helper (INSTALL_STAGED_CONTRACT.md §Windows): the SAME exe in another mode.
//!
//! A running `gnobbi.exe` cannot be renamed or replaced by a second rename, so the app starts a
//! physical copy of itself (`.gnobbi-helper-<pid>-<nonce>.exe`, next to the target: not a hard
//! link, not `.prev`, not the staging), and that copy does the swap once the app is gone:
//!
//! 1. `main()` calls [`dispatch`] before the marker, the UI and anything else; `--self-test`
//!    is a different first argument and stays pure.
//! 2. The helper validates its argv (a trust boundary: every path must sit in the helper's own
//!    folder and follow our naming; the marker folder and the backup name are never taken from
//!    argv), opens the parent process handle, prints `READY`. Only then does the parent exit.
//! 3. `WaitForSingleObject` on that handle (bounded, never a blind sleep), then lock, the same
//!    path rules again, size/hash + fsync, hard-link backup, marker, and ONE
//!    `MoveFileExW(staging, target, MOVEFILE_REPLACE_EXISTING)`: same volume, no
//!    `COPY_ALLOWED`, the target name is never moved away first. Busy target or a filesystem
//!    without hard links = a safe one-line error, never a two-rename fallback.
//! 4. Restore (third start without confirmation) goes through the same helper:
//!    `MoveFileExW(prev, target, REPLACE_EXISTING)` consumes `.prev`; the marker is the only
//!    authority for it.
//!
//! `ReplaceFileW` is deliberately NOT used: it is not documented as crash-safe, and its failure
//! 1177 (`ERROR_UNABLE_TO_REMOVE_REPLACED`) can leave the original renamed to the backup name
//! with the target missing. Using it would need a native crash test first.
//!
//! The file steps take their operations as arguments, so tests inject the failure and never
//! start a process. Not proven natively: see the card handoff.

use super::{
    install::{self, Paths},
    stage::Ready,
    startup::{self, Pending},
};
use crate::AppDirs;
use std::{
    io,
    path::{Path, PathBuf},
};

pub const FLAG: &str = "--update-helper";
const PREFIX: &str = ".gnobbi-helper-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Install { ready: Ready },
    Restore,
}

/// What the helper is asked to do, and on which exe. Round-trips through argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub target: PathBuf,
    pub job: Job,
    pub parent: u32,
}

impl Request {
    pub fn install(target: PathBuf, ready: &Ready) -> Self {
        Request { target, job: Job::Install { ready: ready.clone() }, parent: std::process::id() }
    }
    pub fn restore(target: PathBuf) -> Self {
        Request { target, job: Job::Restore, parent: std::process::id() }
    }

    /// `--update-helper install <target> <parent> <new> <build> <size> <sha256>` or `... restore <target> <parent>`.
    pub fn to_args(&self) -> Vec<std::ffi::OsString> {
        let mut a: Vec<std::ffi::OsString> = vec![FLAG.into()];
        match &self.job {
            Job::Install { ready } => {
                a.extend(["install".into(), self.target.clone().into(), self.parent.to_string().into(), ready.path.clone().into()]);
                a.extend([ready.build.to_string().into(), ready.size.to_string().into(), ready.sha256.clone().into()]);
            }
            Job::Restore => a.extend(["restore".into(), self.target.clone().into(), self.parent.to_string().into()]),
        }
        a
    }

    /// Strict: exact argument count, numbers and hash well-formed. Nothing here is trusted yet.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let bad = || install::failed("parametri dell'helper non validi");
        let num = |s: &String| s.parse::<u64>().map_err(|_| bad());
        let (kind, target, parent) = match args {
            [flag, kind, target, parent, ..] if flag == FLAG => (kind.as_str(), PathBuf::from(target), parent.parse::<u32>().map_err(|_| bad())?),
            _ => return Err(bad()),
        };
        let job = match (kind, &args[4..]) {
            ("restore", []) => Job::Restore,
            ("install", [new, build, size, sha]) => {
                if sha.len() != 64 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(bad());
                }
                Job::Install { ready: Ready { version: String::new(), build: num(build)?, path: PathBuf::from(new), size: num(size)?, sha256: sha.clone() } }
            }
            _ => return Err(bad()),
        };
        Ok(Request { target, job, parent })
    }
}

fn is_exe(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

fn name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or_default()
}

/// `.gnobbi-helper-<digits>-<digits>.exe`
fn is_helper_name(n: &str) -> bool {
    n.strip_prefix(PREFIX).and_then(|r| r.strip_suffix(".exe")).and_then(|r| r.split_once('-')).is_some_and(|(a, b)| {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        digits(a) && digits(b)
    })
}

/// Everything the argv claims must hold on disk before the helper says READY (and again
/// right before the swap): all files in the helper's own folder, ours by name, regular files,
/// no backup yet. `.prev` is derived, never read from argv.
pub fn validate(helper_exe: &Path, req: &Request) -> Result<Paths, String> {
    let bad = |c: &str| install::failed(c);
    let absolute_plain = |p: &Path| p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir));
    if !is_helper_name(name(helper_exe)) {
        return Err(bad("helper fuori posto"));
    }
    if !absolute_plain(&req.target) || !is_exe(&req.target) || req.target == helper_exe || req.target.parent() != helper_exe.parent() {
        return Err(bad("binario da aggiornare non valido"));
    }
    match &req.job {
        Job::Install { ready } => {
            let n = name(&ready.path);
            if !absolute_plain(&ready.path) || !(n.starts_with(".gnobbi.") && n.ends_with(".new")) {
                return Err(bad("staging non valido"));
            }
            install::check_paths(req.target.clone(), ready.path.clone())
        }
        Job::Restore => {
            let prev = install::prev_of(&req.target);
            if !install::regular(&req.target) || !install::regular(&prev) {
                return Err(bad("backup da ripristinare non trovato"));
            }
            Ok(Paths { target: req.target.clone(), new: PathBuf::new(), prev })
        }
    }
}

/// The transaction proper, after the parent is gone. `link` makes the backup, `mv` is the one
/// replacing move (`MoveFileExW` in production), `spawn` starts the target.
pub fn execute<L, M, S>(dirs: &AppDirs, helper_exe: &Path, req: &Request, link: L, mv: M, spawn: S) -> Result<(), String>
where
    L: Fn(&Path, &Path) -> io::Result<()>,
    M: Fn(&Path, &Path) -> io::Result<()>,
    S: Fn(&Path) -> io::Result<()>,
{
    let paths = validate(helper_exe, req)?;
    let _lock = install::Lock::acquire(&paths.target)?;
    match &req.job {
        Job::Install { ready } => install_job(dirs, &paths, ready, link, mv, spawn),
        Job::Restore => restore_job(dirs, &paths, mv, spawn),
    }
}

fn install_job(dirs: &AppDirs, paths: &Paths, ready: &Ready, link: impl Fn(&Path, &Path) -> io::Result<()>, mv: impl Fn(&Path, &Path) -> io::Result<()>, spawn: impl Fn(&Path) -> io::Result<()>) -> Result<(), String> {
    install::reverify(paths, ready)?;
    startup::write_pending(dirs, &Pending { from_build: install::running_build(), to_build: ready.build, attempts: 0 }).map_err(|e| install::message(&e))?;
    if let Err(e) = link(&paths.target, &paths.prev) {
        startup::abort_pending(dirs);
        return Err(install::message(&e));
    }
    if let Err(e) = mv(&paths.new, &paths.target) {
        // The move is one rename: a failure leaves the target as it was. Only if the name is
        // gone is `prev` the sole copy, and then it and the marker stay.
        if install::regular(&paths.target) {
            install::drop_backup(paths);
            startup::abort_pending(dirs);
            return Err(install::message(&e));
        }
        return Err(install::failed("posizione non riuscita e ripristino non riuscito"));
    }
    install::sync_dir(&paths.target);
    if let Err(e) = spawn(&paths.target) {
        // New build did not start: `prev` moves back (and is consumed), then the marker goes.
        return Err(match mv(&paths.prev, &paths.target) {
            Ok(()) => {
                install::drop_backup(paths);
                startup::abort_pending(dirs);
                install::message(&e)
            }
            Err(_) => install::failed("avvio non riuscito e ripristino non riuscito"),
        });
    }
    Ok(())
}

fn restore_job(dirs: &AppDirs, paths: &Paths, mv: impl Fn(&Path, &Path) -> io::Result<()>, spawn: impl Fn(&Path) -> io::Result<()>) -> Result<(), String> {
    // Same rule as the startup counter: only a transaction that already failed twice.
    let Some(pending) = startup::pending(dirs).filter(|p| p.attempts >= 2) else {
        return Err(install::failed("nessun ripristino in sospeso"));
    };
    mv(&paths.prev, &paths.target).map_err(|e| install::message(&e))?;
    startup::finish_restore(dirs, &pending);
    spawn(&paths.target).map_err(|e| install::message(&e))
}

/// Deferred cleanup of finished helper copies: our exact name pattern, regular files only. A
/// helper still running is an exe image in use, which Windows refuses to delete, so a live
/// one survives and the next start retries.
pub fn cleanup_stale(target: &Path) {
    let Some(dir) = target.parent() else { return };
    let me = std::env::current_exe().ok();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if is_helper_name(name(&p)) && Some(&p) != me.as_ref() && install::regular(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// `Some(exit code)` when this process is the helper. Called first in `main()`.
pub fn dispatch(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some(FLAG) {
        return None;
    }
    Some(match run_helper(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("update: {e}");
            1
        }
    })
}

#[cfg(not(windows))]
fn run_helper(_: &[String]) -> Result<(), String> {
    Err(install::failed("aggiornamento Windows non disponibile"))
}

#[cfg(not(windows))]
pub fn start(_: &Request) -> Result<(), String> {
    Err(install::failed("aggiornamento Windows non disponibile"))
}

#[cfg(windows)]
pub use win::start;

#[cfg(windows)]
fn run_helper(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    let req = Request::parse(args)?;
    let me = std::env::current_exe().map_err(|e| install::message(&e))?;
    let dirs = crate::app_dirs(crate::Platform::CURRENT, |k| std::env::var(k).ok()).ok_or_else(|| install::failed("cartella dati non trovata"))?;
    validate(&me, &req)?;
    let parent = win::Parent::open(req.parent).map_err(|e| install::message(&e))?;
    // The parent leaves only after this line; from here on it is a waiting handle, not a pid.
    println!("{READY}");
    std::io::stdout().flush().map_err(|e| install::message(&e))?;
    parent.wait(60_000).map_err(|e| install::message(&e))?;
    execute(&dirs, &me, &req, |a, b| std::fs::hard_link(a, b), win::move_replace, |t| std::process::Command::new(t).spawn().map(drop))
}

const READY: &str = "READY";

/// True when the helper's first stdout line is the handshake.
pub fn handshake_ok(first_line: &str) -> bool {
    first_line.trim() == READY
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::windows::ffi::OsStrExt,
        process::Stdio,
        sync::mpsc,
        time::Duration,
    };
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
            Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        },
        core::PCWSTR,
    };

    fn os_error(e: windows::core::Error) -> io::Error {
        // HRESULT_FROM_WIN32 keeps the Win32 code in the low 16 bits.
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    }

    /// Process handle that can be waited on.
    pub struct Parent(HANDLE);

    impl Parent {
        pub fn open(pid: u32) -> io::Result<Self> {
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.map(Parent).map_err(os_error)
        }
        pub fn wait(&self, ms: u32) -> io::Result<()> {
            if unsafe { WaitForSingleObject(self.0, ms) } == WAIT_OBJECT_0 {
                Ok(())
            } else {
                Err(io::Error::other("il programma non si è chiuso"))
            }
        }
    }

    impl Drop for Parent {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    /// The single atomic step. No `MOVEFILE_COPY_ALLOWED`: a cross-volume move fails instead
    /// of degrading into copy + delete.
    pub fn move_replace(from: &Path, to: &Path) -> io::Result<()> {
        let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
        let (f, t) = (wide(from), wide(to));
        unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVEFILE_REPLACE_EXISTING) }.map_err(os_error)
    }

    /// Copies this exe, starts the copy as helper and waits for `READY`. On any failure the
    /// copy is removed (after killing the child) and the caller keeps running.
    pub fn start(req: &Request) -> Result<(), String> {
        let me = std::env::current_exe().and_then(std::fs::canonicalize).map_err(|e| install::message(&e))?;
        if std::fs::canonicalize(&req.target).map_err(|e| install::message(&e))? != me {
            return Err(install::failed("binario in esecuzione diverso dal target"));
        }
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let copy = me.with_file_name(format!("{PREFIX}{}-{nonce}.exe", std::process::id()));
        let result = (|| {
            let mut src = std::fs::File::open(&me)?;
            let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(&copy)?;
            io::copy(&mut src, &mut dst)?;
            dst.sync_all()?;
            drop(dst);
            let mut child = std::process::Command::new(&copy).args(req.to_args()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
            let out = child.stdout.take().ok_or_else(|| io::Error::other("helper senza canale"))?;
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let mut line = String::new();
                let _ = BufReader::new(out).read_line(&mut line);
                let _ = tx.send(line);
            });
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(line) if handshake_ok(&line) => Ok(()),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(io::Error::other("helper non pronto"))
                }
            }
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&copy);
        }
        result.map_err(|e| install::message(&e))
    }
}
