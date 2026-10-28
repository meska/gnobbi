//! Linux native protection-secret lifecycle; call only on a blocking backend worker.
use crate::{persistence::AccountStore, signin::Secret};
use matrix_domain::AccountId;

pub use crate::signin::VaultError;

/// The real Linux Secret Service as a profile protection source.
pub struct SecretService;
impl crate::signin::ProtectionSource for SecretService {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        ProtectionVault::new(store).create()
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        ProtectionVault::new(store).read()
    }
}

/// Borrows the account's exclusive store lock; never substitutes another profile.
pub struct ProtectionVault<'a> {
    store: &'a AccountStore,
}
impl<'a> ProtectionVault<'a> {
    pub fn new(store: &'a AccountStore) -> Self {
        Self { store }
    }

    pub fn read(&self) -> Result<Secret, VaultError> {
        read_entry("gnobbi", &Key::Store(&self.store.path))
    }

    pub fn create(&self) -> Result<Secret, VaultError> {
        let mut text = new_secret_text()?;
        store_new(&Key::Store(&self.store.path), text.as_bytes())?;
        Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
    }
}

fn new_secret_text() -> Result<zeroize::Zeroizing<String>, VaultError> {
    use std::fmt::Write;
    let mut random = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
    let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
    for byte in random.iter() {
        write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
    }
    Ok(text)
}

/// Which vault item: the legacy path-keyed one, or the one of a stable catalog account ID.
enum Key<'a> {
    Store(&'a std::path::Path),
    Account(AccountId),
}

fn attributes(application: &str, key: &Key) -> Result<Vec<(String, String)>, VaultError> {
    let (name, value) = match key {
        Key::Store(path) => (
            "account-store",
            path.to_str().ok_or(VaultError::Unavailable)?.to_owned(),
        ),
        Key::Account(id) => ("account-id", id.as_u64().to_string()),
    };
    Ok(vec![
        ("application".into(), application.into()),
        ("purpose".into(), "sdk-protection-v1".into()),
        (name.into(), value),
    ])
}

fn as_map(pairs: &[(String, String)]) -> std::collections::HashMap<&str, &str> {
    pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// Saves `value` under the gnobbi name for a key; refuses to touch an existing entry.
fn store_new(key: &Key, value: &[u8]) -> Result<(), VaultError> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| VaultError::Unavailable)?;
    let collection = service
        .get_default_collection()
        .map_err(|_| VaultError::Unavailable)?;
    let pairs = attributes("gnobbi", key)?;
    let attributes = as_map(&pairs);
    let existing = service
        .search_items(attributes.clone())
        .map_err(|_| VaultError::Unavailable)?;
    if !existing.unlocked.is_empty() || !existing.locked.is_empty() {
        return Err(VaultError::AlreadyExists);
    }
    if collection
        .is_locked()
        .map_err(|_| VaultError::Unavailable)?
    {
        return Err(VaultError::Locked);
    }
    collection
        .create_item(
            "Gnobbi SDK protection",
            attributes,
            value,
            false,
            "text/plain",
        )
        .map_err(|_| VaultError::Unavailable)?;
    Ok(())
}

/// One vault item per catalog account, keyed by its stable ID: every call touches that
/// item only. Call only on a blocking backend worker.
pub fn create_account_secret(id: AccountId) -> Result<Secret, VaultError> {
    let mut text = new_secret_text()?;
    store_new(&Key::Account(id), text.as_bytes())?;
    Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
}

pub fn read_account_secret(id: AccountId) -> Result<Secret, VaultError> {
    read_entry("gnobbi", &Key::Account(id))
}

/// Removes only this account's item; an already absent item is fine.
pub fn delete_account_secret(id: AccountId) -> Result<(), VaultError> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| VaultError::Unavailable)?;
    let pairs = attributes("gnobbi", &Key::Account(id))?;
    let items = service
        .search_items(as_map(&pairs))
        .map_err(|_| VaultError::Unavailable)?;
    if !items.locked.is_empty() {
        return Err(VaultError::Locked);
    }
    for item in items.unlocked {
        item.delete().map_err(|_| VaultError::Unavailable)?;
    }
    Ok(())
}

/// Copies the single path-keyed item of the old install to the account's own item (the
/// first account of the catalog); the original stays. No-op when the account has an item.
pub fn migrate_store_secret(store: &AccountStore, id: AccountId) -> Result<(), VaultError> {
    match read_raw("gnobbi", &Key::Account(id)) {
        Ok(_) => return Ok(()),
        Err(VaultError::Missing) => {}
        Err(e) => return Err(e),
    }
    let value = read_raw("gnobbi", &Key::Store(&store.path))?;
    store_new(&Key::Account(id), &value)
}

/// Account folders of the adopted profile that still lack a gnobbi entry while the old profile
/// folder has one: `(old key, new key)` pairs. Read-only.
fn legacy_pairs(
    old: &std::path::Path,
    new: &std::path::Path,
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let Ok(dir) = std::fs::read_dir(new) else {
        return Vec::new();
    };
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("account-") && e.path().is_dir())
        .map(|e| (old.join(e.file_name()), e.path()))
        .filter(|(o, n)| {
            read_legacy(o).is_ok()
                && matches!(
                    read_entry("gnobbi", &Key::Store(n)),
                    Err(VaultError::Missing)
                )
        })
        .collect()
}

/// How many old-name entries could be copied for the adopted profile (read-only).
pub fn legacy_pending(old: &std::path::Path, new: &std::path::Path) -> usize {
    legacy_pairs(old, new).len()
}

/// Explicit copy of old-name entries to the gnobbi name; the originals stay untouched.
pub fn copy_legacy_profile(
    old: &std::path::Path,
    new: &std::path::Path,
) -> Result<usize, VaultError> {
    let pairs = legacy_pairs(old, new);
    for (o, n) in &pairs {
        let value = read_raw("dime", &Key::Store(o))?;
        store_new(&Key::Store(n), &value)?;
    }
    Ok(pairs.len())
}

/// Read-only lookup of a protection secret saved under the previous application name for a
/// profile folder; never writes or deletes. `Missing` means no such entry.
pub fn read_legacy(profile: &std::path::Path) -> Result<Secret, VaultError> {
    read_entry("dime", &Key::Store(profile))
}

fn read_entry(application: &str, key: &Key) -> Result<Secret, VaultError> {
    let value = read_raw(application, key)?;
    let text = std::str::from_utf8(&value).map_err(|_| VaultError::InvalidItem)?;
    Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
}

fn read_raw(application: &str, key: &Key) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| VaultError::Unavailable)?;
    let pairs = attributes(application, key)?;
    let items = service
        .search_items(as_map(&pairs))
        .map_err(|_| VaultError::Unavailable)?;
    if items.unlocked.len() + items.locked.len() > 1 {
        return Err(VaultError::InvalidItem);
    }
    if !items.locked.is_empty() {
        return Err(VaultError::Locked);
    }
    let item = items.unlocked.first().ok_or(VaultError::Missing)?;
    let value = zeroize::Zeroizing::new(item.get_secret().map_err(|_| VaultError::Unavailable)?);
    if value.len() != 64 || !value.iter().all(u8::is_ascii_hexdigit) {
        return Err(VaultError::InvalidItem);
    }
    Ok(value)
}
