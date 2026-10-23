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
        let service =
            secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
                .map_err(|_| VaultError::Unavailable)?;
        let path = self.store.path.to_str().ok_or(VaultError::Unavailable)?;
        let attributes = std::collections::HashMap::from([
            ("application", "dime"),
            ("purpose", "sdk-protection-v1"),
            ("account-store", path),
        ]);
        let items = service
            .search_items(attributes)
            .map_err(|_| VaultError::Unavailable)?;
        if items.unlocked.len() + items.locked.len() > 1 {
            return Err(VaultError::InvalidItem);
        }
        if !items.locked.is_empty() {
            return Err(VaultError::Locked);
        }
        let item = items.unlocked.first().ok_or(VaultError::Missing)?;
        let value =
            zeroize::Zeroizing::new(item.get_secret().map_err(|_| VaultError::Unavailable)?);
        if value.len() != 64 || !value.iter().all(u8::is_ascii_hexdigit) {
            return Err(VaultError::InvalidItem);
        }
        let text = std::str::from_utf8(&value).map_err(|_| VaultError::InvalidItem)?;
        Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
    }

    pub fn create(&self) -> Result<Secret, VaultError> {
        use std::fmt::Write;
        let service =
            secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
                .map_err(|_| VaultError::Unavailable)?;
        let collection = service
            .get_default_collection()
            .map_err(|_| VaultError::Unavailable)?;
        let path = self.store.path.to_str().ok_or(VaultError::Unavailable)?;
        let attributes = std::collections::HashMap::from([
            ("application", "dime"),
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
        let mut random = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
        for byte in random.iter() {
            write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
        }
        collection
            .create_item(
                "Dime SDK protection",
                attributes,
                text.as_bytes(),
                false,
                "text/plain",
            )
            .map_err(|_| VaultError::Unavailable)?;
        Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
    }
}
