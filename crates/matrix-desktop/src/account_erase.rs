//! Deleting one account from disk: its vault items, data dir, cache dir and catalog entry.
use crate::{AccountCatalog, AppDirs};
use matrix_domain::AccountId;
use std::path::PathBuf;

/// `forget` receives the account's store folders and removes the account's vault items;
/// false aborts before anything on disk is touched. Other accounts are never read or changed.
pub fn erase_account(
    dirs: &AppDirs,
    id: AccountId,
    forget: impl FnOnce(&[PathBuf]) -> bool,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let mut catalog = AccountCatalog::load(dirs);
    if catalog.is_damaged() {
        return Err(Error::other("damaged catalog"));
    }
    let (Some(data), Some(cache)) = (catalog.data_dir(dirs, id), catalog.cache_dir(dirs, id))
    else {
        return Err(ErrorKind::NotFound.into());
    };
    let stores: Vec<PathBuf> = std::fs::read_dir(&data)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("account-") && e.path().is_dir())
        .map(|e| e.path())
        .collect();
    if !forget(&stores) {
        return Err(Error::other("vault items not removed"));
    }
    for dir in [data, cache] {
        match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    catalog.remove(id);
    catalog.save(dirs)
}
