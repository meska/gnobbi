//! Old `~/Dime/profile` handling: offered to the user, never moved or copied on its own.
use std::path::{Path, PathBuf};

pub fn legacy_path(home: &Path) -> PathBuf {
    home.join("Dime").join("profile")
}

/// True when the old profile exists and the new data dir has no profile yet.
pub fn offered(legacy: &Path, data: &Path) -> bool {
    legacy.is_dir() && !data.join("profile").exists()
}

/// Atomic rename into `<data>/profile`. A failure (e.g. another filesystem) leaves the old profile untouched; no copy fallback.
pub fn adopt(legacy: &Path, data: &Path) -> std::io::Result<()> {
    let target = data.join("profile");
    if target.exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "profile already present"));
    }
    std::fs::rename(legacy, target)
}
