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

#[derive(Debug)]
pub enum MigrationError {
    Ambiguous { old: PathBuf, new: PathBuf },
    Io(io::Error),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ambiguous { old, new } => write!(f, "ambiguous store migration: {} -> {}", old.display(), new.display()),
            Self::Io(error) => error.fmt(f),
        }
    }
}

/// Publish all stores together; a split or duplicate store requires manual intervention.
pub fn migrate(old: &Path, new: &Path) -> Result<(), MigrationError> {
    if old == new {
        return Ok(());
    }
    let ambiguous = || MigrationError::Ambiguous { old: old.to_owned(), new: new.to_owned() };
    // An unreadable path cannot safely be classified as absent.
    let exists = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    };
    let old_present = STORES.iter().map(|name| exists(&old.join(name))).collect::<io::Result<Vec<_>>>().map_err(|_| ambiguous())?;
    let new_present = STORES.iter().map(|name| exists(&new.join(name))).collect::<io::Result<Vec<_>>>().map_err(|_| ambiguous())?;
    if new_present.iter().any(|v| *v) {
        return if old_present.iter().any(|v| *v) { Err(ambiguous()) } else { Ok(()) };
    }
    if !old_present.iter().any(|v| *v) {
        return Ok(());
    }
    let mut staging_name = new.as_os_str().to_os_string();
    staging_name.push(".staging");
    let staging = PathBuf::from(staging_name);
    let migrate = || -> io::Result<()> {
        match std::fs::remove_dir_all(&staging) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        crate::create_private_dir(new.parent().ok_or_else(|| io::Error::other("missing parent"))?)?;
        crate::create_private_dir(&staging)?;
        for (name, present) in STORES.iter().zip(old_present.iter()) {
            if *present {
                let (from, to) = (old.join(name), staging.join(name));
                if !std::fs::symlink_metadata(&from)?.file_type().is_dir() {
                    return Err(io::Error::other("store is not a directory"));
                }
                copy_tree(&from, &to)?;
                if listing(&from)? != listing(&to)? {
                    return Err(io::Error::other("copy differs from source"));
                }
            }
        }
        std::fs::rename(&staging, new)?;
        Ok(())
    };
    if let Err(error) = migrate() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(MigrationError::Io(error));
    }
    for (name, present) in STORES.iter().zip(old_present.iter()) {
        if *present {
            // A failed deletion leaves duplicates and therefore stops the next startup.
            let _ = std::fs::remove_dir_all(old.join(name));
        }
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
            t if t.is_file() => {
                std::fs::copy(&src, &dst)?; // keeps the 0600 mode the store checks
                use std::io::Read;
                let (mut a, mut b) = (std::io::BufReader::new(std::fs::File::open(&src)?), std::io::BufReader::new(std::fs::File::open(&dst)?));
                let (mut left, mut right) = ([0u8; 65536], [0u8; 65536]);
                loop {
                    let count = a.read(&mut left)?;
                    b.read_exact(&mut right[..count])?;
                    if left[..count] != right[..count] { return Err(io::Error::other("copy differs from source")); }
                    if count == 0 { break; }
                }
                if b.read(&mut right)? != 0 { return Err(io::Error::other("copy differs from source")); }
            },
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
    fn ambiguous_split_or_duplicate_stores_stop_without_changes() {
        let (old, new) = (scratch("ambiguous-old"), scratch("ambiguous-new"));
        seed(&old);
        std::fs::create_dir_all(old.join("profile")).unwrap();
        std::fs::write(old.join("profile/source"), b"source").unwrap();
        std::fs::create_dir_all(new.join("accounts")).unwrap();
        std::fs::write(new.join("accounts/dest"), b"dest").unwrap();
        let error = migrate(&old, &new).unwrap_err();
        assert!(matches!(&error, MigrationError::Ambiguous { old: o, new: n } if o == &old && n == &new));
        let error = error.to_string();
        assert!(error.contains("ambiguous"));
        assert!(error.contains(old.to_str().unwrap()) && error.contains(new.to_str().unwrap()));
        assert_eq!(std::fs::read(old.join("profile/source")).unwrap(), b"source");
        assert_eq!(std::fs::read(old.join("accounts/3/account-7/session.sealed")).unwrap(), b"sealed");
        assert_eq!(std::fs::read(new.join("accounts/dest")).unwrap(), b"dest");
        std::fs::remove_dir_all(old.join("accounts")).unwrap();
        assert!(migrate(&old, &new).unwrap_err().to_string().contains("ambiguous"));
        assert!(old.join("profile/source").exists() && new.join("accounts/dest").exists());
    }

    #[test]
    fn stale_staging_is_discarded_and_complete_destination_is_untouched() {
        let (old, new) = (scratch("stale-old"), scratch("stale-new").join("data"));
        seed(&old);
        let staging = new.with_extension("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("junk"), b"x").unwrap();
        migrate(&old, &new).unwrap();
        assert!(!staging.exists() && !new.join("junk").exists());
        migrate(&old, &new).unwrap();
        assert_eq!(std::fs::read(new.join("accounts/3/account-7/session.sealed")).unwrap(), b"sealed");
    }

    #[test]
    fn complete_destination_with_both_stores_is_untouched() {
        let (old, new) = (scratch("complete-old"), scratch("complete-new"));
        std::fs::create_dir_all(new.join("accounts")).unwrap();
        std::fs::create_dir_all(new.join("profile")).unwrap();
        std::fs::write(new.join("accounts/a"), b"a").unwrap();
        std::fs::write(new.join("profile/p"), b"p").unwrap();
        migrate(&old, &new).unwrap();
        assert_eq!(std::fs::read(new.join("accounts/a")).unwrap(), b"a");
        assert_eq!(std::fs::read(new.join("profile/p")).unwrap(), b"p");
        assert!(!old.join("accounts").exists() && !old.join("profile").exists());
    }

    #[test]
    fn second_store_failure_keeps_both_sources_for_fallback() {
        let (old, new) = (scratch("second-fail-old"), scratch("second-fail-new").join("data"));
        seed(&old);
        crate::create_private_dir(&old.join("profile")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", old.join("profile/link")).unwrap();
        assert!(migrate(&old, &new).is_err());
        assert!(!new.exists());
        assert!(!new.with_extension("staging").exists());
        assert_eq!(std::fs::read(old.join("accounts/3/account-7/session.sealed")).unwrap(), b"sealed");
        assert!(old.join("profile/link").is_symlink());
        std::fs::write(old.join("accounts/3/account-7/session.sealed"), b"updated").unwrap();
        std::fs::remove_file(old.join("profile/link")).unwrap();
        migrate(&old, &new).unwrap();
        assert_eq!(std::fs::read(new.join("accounts/3/account-7/session.sealed")).unwrap(), b"updated");
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
