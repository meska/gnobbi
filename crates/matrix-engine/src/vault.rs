//! Linux native protection-secret lifecycle; call only on a blocking backend worker.
use crate::{persistence::AccountStore, signin::Secret};

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
        read_entry("gnobbi", &self.store.path)
    }

    pub fn create(&self) -> Result<Secret, VaultError> {
        use std::fmt::Write;
        let mut random = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
        for byte in random.iter() {
            write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
        }
        store_new(&self.store.path, text.as_bytes())?;
        Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
    }
}

/// Saves `value` under the gnobbi name for a profile folder; refuses to touch an existing entry.
fn store_new(profile: &std::path::Path, value: &[u8]) -> Result<(), VaultError> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| VaultError::Unavailable)?;
    let collection = service
        .get_default_collection()
        .map_err(|_| VaultError::Unavailable)?;
    let path = profile.to_str().ok_or(VaultError::Unavailable)?;
    let attributes = std::collections::HashMap::from([
        ("application", "gnobbi"),
        ("purpose", "sdk-protection-v1"),
        ("account-store", path),
    ]);
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
            read_legacy(o).is_ok() && matches!(read_entry("gnobbi", n), Err(VaultError::Missing))
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
        let value = read_raw("dime", o)?;
        store_new(n, &value)?;
    }
    Ok(pairs.len())
}

/// Read-only lookup of a protection secret saved under the previous application name for a
/// profile folder; never writes or deletes. `Missing` means no such entry.
pub fn read_legacy(profile: &std::path::Path) -> Result<Secret, VaultError> {
    read_entry("dime", profile)
}

fn read_entry(application: &str, profile: &std::path::Path) -> Result<Secret, VaultError> {
    let value = read_raw(application, profile)?;
    let text = std::str::from_utf8(&value).map_err(|_| VaultError::InvalidItem)?;
    Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
}

fn read_raw(
    application: &str,
    profile: &std::path::Path,
) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| VaultError::Unavailable)?;
    let path = profile.to_str().ok_or(VaultError::Unavailable)?;
    let items = service
        .search_items(std::collections::HashMap::from([
            ("application", application),
            ("purpose", "sdk-protection-v1"),
            ("account-store", path),
        ]))
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
