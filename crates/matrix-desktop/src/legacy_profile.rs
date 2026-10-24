//! Old `~/Dime/profile` handling: offered to the user, never moved or copied on its own.
use std::path::{Path, PathBuf};

pub fn legacy_path(home: &Path) -> PathBuf {
    home.join("Dime").join("profile")
}

/// True when the old profile exists and the new data dir has no profile yet.
pub fn offered(legacy: &Path, data: &Path) -> bool {
    legacy.is_dir() && absent(&data.join("profile"))
}

/// A directory created empty at startup counts as absent.
fn absent(path: &Path) -> bool {
    !path.exists() || path.read_dir().is_ok_and(|mut entries| entries.next().is_none())
}

/// Atomic rename into `<data>/profile`. A failure (e.g. another filesystem) leaves the old profile untouched; no copy fallback.
pub fn adopt(legacy: &Path, data: &Path) -> std::io::Result<()> {
    let target = data.join("profile");
    if !absent(&target) {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "profile already present"));
    }
    // rename onto an empty directory replaces it atomically on Unix
    std::fs::rename(legacy, target)
}
