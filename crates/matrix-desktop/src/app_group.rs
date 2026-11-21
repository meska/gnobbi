//! iOS: the account stores live in the App Group container, so the notification extension can open them.
//! Layout there: `<container>/Library/Application Support/Gnobbi/<accounts/N | profile>/account-<id>/`.
//! Settings and `accounts.json` stay in the app's own container (`AppDirs::config`).
use std::io;
use std::path::{Path, PathBuf};

pub const GROUP: &str = "group.net.meskatech.gnobbi";
/// The folders of `AppDirs::data` that hold an account store (see `AccountCatalog`).
const STORES: [&str; 2] = ["accounts", "profile"];

/// The group container, if this build is entitled to it (a build without the App Group gets None: nothing moves).
#[cfg(all(target_os = "ios", not(target_abi = "sim")))]
pub fn container() -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSString};
    let url = NSFileManager::defaultManager().containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(GROUP))?;
    Some(PathBuf::from(url.path()?.to_string()))
}

/// Where `AppDirs::data` goes when the group exists.
#[cfg(all(target_os = "ios", not(target_abi = "sim")))]
pub fn data_dir() -> Option<PathBuf> {
    Some(container()?.join("Library").join("Application Support").join("Gnobbi"))
}

/// Moves the store folders from `old` to `new` (both `data` dirs). Per folder: copy next to the target, check names and
/// sizes against the source, rename into place, only then delete the source. A crash leaves the source whole.
/// A target that already exists is the result of an earlier rename (nothing else creates it): the leftover source goes.
pub fn migrate(old: &Path, new: &Path) -> io::Result<()> {
    if old == new {
        return Ok(());
    }
    for name in STORES {
        let (from, to) = (old.join(name), new.join(name));
        if !from.is_dir() {
            continue;
        }
        if !to.exists() {
            let tmp = new.join(format!("{name}.migrating"));
            let _ = std::fs::remove_dir_all(&tmp);
            crate::create_private_dir(new)?;
            copy_tree(&from, &tmp)?;
            if listing(&from)? != listing(&tmp)? {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(io::Error::other("copy differs from source"));
            }
            std::fs::rename(&tmp, &to)?;
        }
        let _ = std::fs::remove_dir_all(&from); // best effort: the next launch drops what is left
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    crate::create_private_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        match entry.file_type()? {
            t if t.is_dir() => copy_tree(&src, &dst)?,
            t if t.is_file() => drop(std::fs::copy(&src, &dst)?), // keeps the 0600 mode the store checks
            _ => return Err(io::Error::other("not a regular file")), // the store refuses these too
        }
    }
    Ok(())
}

/// Relative path and size of every file, sorted.
fn listing(root: &Path) -> io::Result<Vec<(PathBuf, u64)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(root, &path, out)?;
            } else {
                out.push((path.strip_prefix(root).unwrap_or(&path).to_owned(), entry.metadata()?.len()));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnobbi-app-group-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(old: &Path) {
        let store = old.join("accounts/3/account-7");
        crate::create_private_dir(&store).unwrap();
        std::fs::write(store.join("session.sealed"), b"sealed").unwrap();
        std::fs::set_permissions(store.join("session.sealed"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(old.join("settings.json"), b"{}").unwrap();
    }

    #[test]
    fn moves_the_stores_and_only_them() {
        let (old, new) = (scratch("move-old"), scratch("move-new").join("data"));
        seed(&old);
        migrate(&old, &new).unwrap();
        let moved = new.join("accounts/3/account-7/session.sealed");
        assert_eq!(std::fs::read(&moved).unwrap(), b"sealed");
        assert_eq!(std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(new.join("accounts/3")).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(!old.join("accounts").exists());
        assert!(old.join("settings.json").exists());
        assert!(!new.join("accounts.migrating").exists());
    }

    #[test]
    fn second_run_and_missing_source_change_nothing() {
        let (old, new) = (scratch("again-old"), scratch("again-new"));
        migrate(&old, &new).unwrap(); // nothing to move
        seed(&old);
        migrate(&old, &new).unwrap();
        migrate(&old, &new).unwrap();
        assert!(new.join("accounts/3/account-7/session.sealed").exists());
    }

    #[test]
    fn leftover_source_after_a_crash_is_dropped_but_a_partial_copy_is_redone() {
        let (old, new) = (scratch("crash-old"), scratch("crash-new"));
        seed(&old);
        // crash after the rename, before the delete: both exist
        migrate(&old, &new).unwrap();
        seed(&old);
        migrate(&old, &new).unwrap();
        assert!(!old.join("accounts").exists());
        // crash during the copy: a stale `.migrating` is discarded, the source still whole
        let (old, new) = (scratch("part-old"), scratch("part-new"));
        seed(&old);
        std::fs::create_dir_all(new.join("accounts.migrating")).unwrap();
        std::fs::write(new.join("accounts.migrating/junk"), b"x").unwrap();
        migrate(&old, &new).unwrap();
        assert!(!new.join("accounts/junk").exists());
        assert!(new.join("accounts/3/account-7/session.sealed").exists());
    }

    #[test]
    fn a_symlink_in_a_store_aborts_and_keeps_the_source() {
        let (old, new) = (scratch("link-old"), scratch("link-new"));
        seed(&old);
        std::os::unix::fs::symlink("/etc/hosts", old.join("accounts/3/account-7/link")).unwrap();
        assert!(migrate(&old, &new).is_err());
        assert!(old.join("accounts/3/account-7/session.sealed").exists());
        assert!(!new.join("accounts").exists());
    }
}
