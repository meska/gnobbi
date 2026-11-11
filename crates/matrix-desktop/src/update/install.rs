//! Install a verified staging file over the running binary (INSTALL_STAGED_CONTRACT.md,
//! rettifica 09/11/2026). Read-only until every check has passed.
//!
//! Linux order: lock → re-hash and sync the staging → marker → `hard_link(target, prev)` →
//! one `rename(staging, target)` → relaunch. The target is never moved out of the way first,
//! so its name is never free in between. Windows cannot rename the running `gnobbi.exe`, so
//! [`install_staged`] hands the same transaction to [`super::helper`] (a physical copy of this
//! exe, a single `MoveFileExW`); the Linux step is never run there.
//!
//! [`swap`] and [`install_with`] take their file operations as arguments, so a test can force
//! any failure without starting or terminating the process under test: the spawn and the exit
//! belong to the caller ([`install_and_restart`]).

use super::{
    channel::Channel,
    stage::Ready,
    startup::{self, Pending},
    verify::{self, Candidate, UpdateError},
};
use crate::AppDirs;
use std::{io, path::{Path, PathBuf}};

/// Target, verified staging and backup of one transaction; all three in the target folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The binary to replace.
    pub target: PathBuf,
    /// The verified staging file (`Ready.path`), still named `.gnobbi.*.new`.
    pub new: PathBuf,
    /// Whole target path plus `.prev`, never an extension swap.
    pub prev: PathBuf,
}

/// `<target>.prev`.
pub fn prev_of(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_owned();
    s.push(".prev");
    PathBuf::from(s)
}

/// One install at a time per target. The OS releases it when this process dies.
pub(super) struct Lock {
    _file: std::fs::File,
    #[cfg(test)]
    _no_spawn: std::sync::RwLockReadGuard<'static, ()>,
}

impl Lock {
    pub(super) fn acquire(target: &Path) -> Result<Self, String> {
        #[cfg(test)]
        let no_spawn = crate::preferences::no_spawn();
        let mut s = target.as_os_str().to_owned();
        s.push(".install.lock");
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(PathBuf::from(s)).map_err(|e| message(&e))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock {
                _file: file,
                #[cfg(test)]
                _no_spawn: no_spawn,
            }),
            Err(std::fs::TryLockError::WouldBlock) => Err(failed("altra installazione in corso")),
            Err(std::fs::TryLockError::Error(e)) => Err(message(&e)),
        }
    }
}

/// Regular file: not a directory, not a symlink/reparse point.
/// A metadata error other than "not found" (e.g. PermissionDenied) is reported, not hidden.
pub(super) fn regular(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => Ok(m.file_type().is_file()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(message(&e)),
    }
}

/// Channel and paths, validated before anything is written. Fails closed: external staging,
/// a symlinked target and an existing backup are refused untouched.
pub fn paths_for(channel: &Channel, ready: &Ready) -> Result<Paths, String> {
    let target = match channel {
        Channel::AppImage { target } => target.clone(),
        Channel::WindowsPortable { exe } => exe.clone(),
        _ => return Err(failed("canale senza aggiornamento in-app")),
    };
    check_paths(target, ready.path.clone())
}

/// The path rules shared with the Windows helper, which re-runs them on its own argv.
pub(super) fn check_paths(target: PathBuf, new: PathBuf) -> Result<Paths, String> {
    if !regular(&target)? {
        return Err(failed("binario da aggiornare non trovato"));
    }
    if !regular(&new)? {
        return Err(failed("staging non trovato"));
    }
    if new.parent() != target.parent() {
        return Err(failed("staging fuori dalla cartella del target"));
    }
    let prev = prev_of(&target);
    if new == target || new == prev {
        return Err(failed("staging non valido"));
    }
    // An existing backup means an earlier transaction is still recoverable: stop without
    // overwriting or deleting anything. Unix only: a backup that is the target's own inode
    // is no copy at all; the installer drops it under the lock (see `install_with`).
    match std::fs::symlink_metadata(&prev) {
        Ok(_) if cfg!(unix) && same_file(&prev, &target) => {}
        Ok(_) => return Err(failed(&format!("rimuovi {}", prev.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(message(&e)),
    }
    Ok(Paths { target, new, prev })
}

/// Which copy is still recoverable after a failed swap.
#[derive(Debug)]
pub enum Undo {
    /// `prev` went back over the target (the rename consumed it): the old build runs.
    Restored,
    /// The old build could not be put back: keep `prev` and the transaction data.
    Lost(io::Error),
}

#[derive(Debug)]
pub enum SwapError {
    /// The backup link failed: nothing was created, the target was never touched.
    Backup(io::Error),
    /// The marker could not be written: the backup link exists, the target is untouched.
    Mark(io::Error),
    /// The staging could not be put in place.
    Place { cause: io::Error, undo: Undo },
}

/// The file-level half of the transaction: no spawn, no exit, no process state. Contract
/// order: `link` makes the backup (a hard link: same bytes, no copy), `mark` writes the
/// marker, `rename` is the single atomic step that puts the staging in place. A crash between
/// two steps never leaves a marker without its backup. Precondition: [`paths_for`] passed.
pub fn swap<L, K, R>(paths: &Paths, link: L, mark: K, rename: R) -> Result<(), SwapError>
where
    L: Fn(&Path, &Path) -> io::Result<()>,
    K: FnOnce() -> io::Result<()>,
    R: Fn(&Path, &Path) -> io::Result<()>,
{
    link(&paths.target, &paths.prev).map_err(SwapError::Backup)?;
    mark().map_err(SwapError::Mark)?;
    if let Err(cause) = rename(&paths.new, &paths.target) {
        // A successful rollback rename consumes `prev`, so no backup is left behind; on
        // failure `prev` is the only copy still worth keeping, and nothing is deleted.
        let undo = match rename(&paths.prev, &paths.target) {
            Ok(()) => Undo::Restored,
            Err(e) => Undo::Lost(e),
        };
        return Err(SwapError::Place { cause, undo });
    }
    Ok(())
}

/// The whole transaction with injected operations: `spawn` starts the new binary, and nothing
/// here terminates the current process.
pub(crate) fn install_with<L, R, S, D>(
    dirs: &AppDirs,
    channel: &Channel,
    ready: &Ready,
    link: L,
    rename: R,
    spawn: S,
    remove: D,
) -> Result<(), String>
where
    L: Fn(&Path, &Path) -> io::Result<()>,
    R: Fn(&Path, &Path) -> io::Result<()>,
    S: Fn(&Path) -> io::Result<()>,
    D: Fn(&Path) -> io::Result<()>,
{
    let early = paths_for(channel, ready)?;
    let _lock = Lock::acquire(&early.target)?;
    // Everything checked before the lock is checked again: the state may have changed.
    let paths = paths_for(channel, ready)?;
    refuse_pending(dirs)?;
    // No marker and a backup that is the same inode as the target: an orphan link, not a copy.
    drop_backup(&paths, &remove).map_err(|e| message(&e))?;
    reverify(&paths, ready)?;
    // With this transaction's identity: attempts=0 is the first start.
    let mark = || startup::write_pending(dirs, &Pending { from_build: running_build(), to_build: ready.build, attempts: 0 });
    match swap(&paths, &link, mark, &rename) {
        Ok(()) => {}
        Err(SwapError::Backup(e)) => return Err(message(&e)),
        // The target is untouched: our backup link and (if written) our marker go away.
        Err(SwapError::Mark(e)) => return Err(cleanup(dirs, &paths, ready.build, &remove, message(&e))),
        Err(SwapError::Place { cause, undo }) => {
            return Err(match undo {
                Undo::Restored => cleanup(dirs, &paths, ready.build, &remove, message(&cause)),
                // Nothing is deleted: `prev` is the copy to recover from, the marker stays.
                Undo::Lost(e) => lost("posizione non riuscita e ripristino non riuscito", &e),
            });
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The staging was already 0755 and a rename keeps the mode: re-asserted, not trusted.
        if let Err(e) = std::fs::set_permissions(&paths.target, std::fs::Permissions::from_mode(0o755)) {
            eprintln!("update: chmod not applied: {e}");
        }
    }
    sync_dir(&paths.target);
    if let Err(e) = spawn(&paths.target) {
        // The new binary did not start: put the old one back, and only then drop the marker.
        return Err(match rename(&paths.prev, &paths.target) {
            Ok(()) => cleanup(dirs, &paths, ready.build, &remove, message(&e)),
            Err(e) => lost("avvio non riuscito e ripristino non riuscito", &e),
        });
    }
    Ok(())
}

/// A pending marker is an unfinished transaction (and its `prev`): refuse, touch nothing.
/// Call under the lock; an unreadable state also refuses.
pub(super) fn refuse_pending(dirs: &AppDirs) -> Result<(), String> {
    match startup::marker_path(dirs).symlink_metadata() {
        Ok(_) => Err(failed("aggiornamento precedente in sospeso")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(message(&e)),
    }
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    matches!((std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)), (Ok(a), Ok(b)) if a.dev() == b.dev() && a.ino() == b.ino())
}

// Windows: no identity proof; relies on MoveFileExW being atomic (a rolled-back `prev` is consumed), not on a check.
#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> bool {
    true
}

/// Removes a backup that a successful rollback left behind: a rename between two names of the
/// same inode is a no-op on Unix, so `prev` can still be there with the old build back on the
/// target. Only that link is dropped; a `prev` that is a different file is the one recoverable
/// copy and is kept, reported as an error.
pub(super) fn drop_backup(paths: &Paths, remove: impl Fn(&Path) -> io::Result<()>) -> io::Result<()> {
    match std::fs::symlink_metadata(&paths.prev) {
        Ok(_) if same_file(&paths.prev, &paths.target) => remove(&paths.prev),
        Ok(_) => Err(io::Error::other("backup diverso dal binario, conservato")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// End of a failed transaction whose target is intact: backup first, then our marker, and the
/// marker only if the backup is gone. A failed cleanup is logged; `base` stays the one-line message.
pub(super) fn cleanup(dirs: &AppDirs, paths: &Paths, to_build: u64, remove: &impl Fn(&Path) -> io::Result<()>, base: String) -> String {
    match drop_backup(paths, remove).and_then(|()| startup::abort_pending(dirs, to_build, remove)) {
        Ok(()) => base,
        Err(e) => {
            eprintln!("update: cleanup incomplete: {e}");
            base
        }
    }
}

/// Production entry. Ends with the new binary started; the marker stays until that build
/// confirms at the documented checkpoint.
pub fn install_staged(dirs: &AppDirs, channel: &Channel, ready: &Ready) -> Result<(), String> {
    if matches!(channel, Channel::WindowsPortable { .. }) {
        // Returns once the helper has confirmed it holds this process's handle: the caller
        // must exit now, the swap happens after that.
        let paths = paths_for(channel, ready)?;
        // The helper runs after this process is gone and cannot report: refuse here what it would.
        refuse_pending(dirs)?;
        reverify(&paths, ready)?;
        return super::helper::start(&super::helper::Request::install(paths.target, ready));
    }
    install_with(dirs, channel, ready, |a, b| std::fs::hard_link(a, b), |a, b| std::fs::rename(a, b), relaunch, |p| std::fs::remove_file(p))
}

/// Production entry for the UI: on failure it returns the error and the process stays alive;
/// it exits only once the new build has started.
pub fn install_and_restart(dirs: &AppDirs, channel: &Channel, ready: &Ready) -> Result<std::convert::Infallible, String> {
    install_staged(dirs, channel, ready)?;
    std::process::exit(0)
}

/// Size and hash again, immediately before the swap, then the bytes to the disk.
pub(super) fn reverify(paths: &Paths, ready: &Ready) -> Result<(), String> {
    let candidate = Candidate {
        version: ready.version.clone(),
        build: ready.build,
        file_name: paths.new.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        sha256: ready.sha256.clone(),
        size: ready.size,
    };
    if let Err(e) = verify::verify_file(&paths.new, &candidate) {
        // Altered staging is useless and must not be reused; the target is the intact copy.
        if matches!(e, UpdateError::SizeMismatch { .. } | UpdateError::Sha256Mismatch) {
            let _ = std::fs::remove_file(&paths.new);
        }
        return Err(describe(e));
    }
    // write(true) without truncate: Windows FlushFileBuffers is denied on a read-only handle.
    std::fs::OpenOptions::new().write(true).open(&paths.new).and_then(|f| f.sync_all()).map_err(|e| message(&e))
}

fn describe(e: UpdateError) -> String {
    failed(match e {
        UpdateError::SizeMismatch { .. } => "staging di dimensione diversa",
        UpdateError::Sha256Mismatch => "staging con hash diverso",
        UpdateError::Io(std::io::ErrorKind::PermissionDenied) => "cartella non scrivibile",
        _ => "staging non verificabile",
    })
}

pub(super) fn running_build() -> u64 {
    super::build_number(crate::build_info::BUILD_INFO.build).unwrap_or(0)
}

/// Best effort: makes the rename itself durable where the filesystem supports it.
pub(super) fn sync_dir(target: &Path) {
    if let Some(dir) = target.parent()
        && let Ok(f) = std::fs::File::open(dir)
    {
        let _ = f.sync_all();
    }
}

/// Starts the new binary on the same path, without the AppImage environment of the old one.
/// A started process is not a working update: the marker carries that confirmation.
fn relaunch(target: &Path) -> io::Result<()> {
    let _child = std::process::Command::new(target)
        .args(std::env::args_os().skip(1))
        .env_remove("APPIMAGE")
        .env_remove("APPDIR")
        .env_remove("ARGV0")
        .spawn()?;
    Ok(())
}

/// Failed rollback: PermissionDenied keeps the exact contract line, the rest stays generic.
pub(super) fn lost(generic: &str, e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::PermissionDenied { message(e) } else { failed(generic) }
}

pub(super) fn failed(cause: &str) -> String {
    let cause = cause.lines().next().unwrap_or_default().trim();
    format!("Aggiornamento non riuscito: {}", if cause.is_empty() { "errore" } else { cause })
}

/// One line, per INSTALL_STAGED_CONTRACT.md §8.
pub(super) fn message(e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::PermissionDenied {
        failed("cartella non scrivibile")
    } else {
        failed(&e.to_string())
    }
}
