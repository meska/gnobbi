//! Old `~/Dime/profile` handling: offered to the user, never moved or copied on its own.
use std::path::{Path, PathBuf};

pub fn legacy_path(home: &Path) -> PathBuf {
    home.join("Dime").join("profile")
}

/// Profile of the pre-rename app folder, next to the new data dir (e.g. `.../share/dime/profile`).
pub fn renamed_path(data: &Path) -> Option<PathBuf> {
    let old = if data.file_name()?.to_str()? == "Gnobbi" { "Dime" } else { "dime" };
    Some(data.parent()?.join(old).join("profile"))
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
