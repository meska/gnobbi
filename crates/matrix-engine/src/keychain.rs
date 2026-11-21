//! macOS and iOS Keychain protection-secret lifecycle; call only on a blocking backend worker.
#[cfg(target_os = "ios")]
use crate::signin::AccountProtectionSource;
use crate::signin::{ProtectionSource, Secret};
use matrix_domain::AccountId;
#[cfg(target_os = "macos")]
use crate::persistence::AccountStore;
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::SecKeychain;

pub use crate::signin::VaultError;

const SERVICE: &str = "gnobbi.sdk-protection-v1";
/// Keychain access group of the app and its notification extension: the app's own id group, always entitled.
#[cfg(all(target_os = "ios", not(target_abi = "sim")))]
const SHARED_GROUP: &str = "T4RJLVSR79.net.meskatech.gnobbi";
// Keychain OSStatus codes (Security/SecBase.h).
const ITEM_NOT_FOUND: i32 = -25300;
const INTERACTION_NOT_ALLOWED: i32 = -25308;
// A locked keychain with prompts disabled reports auth-failed; both mean "needs the user to unlock".
const AUTH_FAILED: i32 = -25293;

fn classify(error: &security_framework::base::Error) -> VaultError {
    match error.code() {
        ITEM_NOT_FOUND => VaultError::Missing,
        INTERACTION_NOT_ALLOWED | AUTH_FAILED => VaultError::Locked,
        _ => VaultError::Unavailable,
    }
}

#[cfg(target_os = "macos")]
/// The login Keychain as a profile protection source.
pub struct MacKeychain;
#[cfg(target_os = "macos")]
impl ProtectionSource for MacKeychain {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        Keychain::new(store).create()
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        Keychain::new(store).read()
    }
}

#[cfg(target_os = "macos")]
/// Borrows the account's exclusive store lock; the item account is that store's path,
/// so another profile or account never resolves to this secret.
pub struct Keychain<'a> {
    store: &'a AccountStore,
    /// None = the user's default (login) Keychain.
    keychain: Option<SecKeychain>,
}
#[cfg(target_os = "macos")]
impl<'a> Keychain<'a> {
    pub fn new(store: &'a AccountStore) -> Self {
        Self {
            store,
            keychain: None,
        }
    }

    /// Same contract against an explicit Keychain file (disposable test keychains).
    pub fn in_keychain(store: &'a AccountStore, keychain: SecKeychain) -> Self {
        Self {
            store,
            keychain: Some(keychain),
        }
    }

    fn keychain(&self) -> Result<SecKeychain, VaultError> {
        match &self.keychain {
            Some(k) => Ok(k.clone()),
            None => SecKeychain::default().map_err(|e| classify(&e)),
        }
    }

    fn account(&self) -> Result<&str, VaultError> {
        self.store.path.to_str().ok_or(VaultError::Unavailable)
    }

    pub fn read(&self) -> Result<Secret, VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let (password, _) = self
            .keychain()?
            .find_generic_password(SERVICE, self.account()?)
            .map_err(|e| classify(&e))?;
        let value = zeroize::Zeroizing::new(password.to_vec());
        if value.len() != 64 || !value.iter().all(u8::is_ascii_hexdigit) {
            return Err(VaultError::InvalidItem);
        }
        let text = std::str::from_utf8(&value).map_err(|_| VaultError::InvalidItem)?;
        Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
    }

    pub fn create(&self) -> Result<Secret, VaultError> {
        use std::fmt::Write;
        let account = self.account()?;
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let keychain = self.keychain()?;
        match keychain.find_generic_password(SERVICE, account) {
            Ok(_) => return Err(VaultError::AlreadyExists),
            Err(e) if classify(&e) == VaultError::Missing => {}
            Err(e) => return Err(classify(&e)),
        }
        let mut random = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
        for byte in random.iter() {
            write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
        }
        keychain
            .add_generic_password(SERVICE, account, text.as_bytes())
            .map_err(|e| classify(&e))?;
        Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
    }

    /// Removes this store's item only (rollback and test cleanup).
    pub fn delete(&self) -> Result<(), VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let (_, item) = self
            .keychain()?
            .find_generic_password(SERVICE, self.account()?)
            .map_err(|e| classify(&e))?;
        item.delete();
        Ok(())
    }
}

/// One Keychain item per catalog account, keyed by its stable ID (account `account-id:<n>`;
/// store paths are absolute, so the two namespaces never meet). Every call touches that
/// item only.
pub struct AccountKeychain {
    /// None = the user's default (login) Keychain.
    #[cfg(target_os = "macos")]
    keychain: Option<SecKeychain>,
}

// The only platform-specific part: iOS has one app-scoped keychain, no prompts to disable.
#[cfg(target_os = "ios")]
impl AccountKeychain {
    pub fn new() -> Self {
        Self {}
    }

    /// Device builds read from the shared group first; an item that predates it (default group) is
    /// found by the plain lookup and copied into the group, the original stays.
    fn find(&self, account: &str) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
        use security_framework::passwords::{generic_password, PasswordOptions};
        #[cfg(not(target_abi = "sim"))]
        {
            let mut options = PasswordOptions::new_generic_password(SERVICE, account);
            options.set_access_group(SHARED_GROUP);
            match generic_password(options) {
                Ok(v) => return Ok(zeroize::Zeroizing::new(v)),
                Err(e) if e.code() != ITEM_NOT_FOUND => return Err(classify(&e)),
                Err(_) => {}
            }
        }
        let value = generic_password(PasswordOptions::new_generic_password(SERVICE, account))
            .map(zeroize::Zeroizing::new)
            .map_err(|e| classify(&e))?;
        #[cfg(not(target_abi = "sim"))]
        let _ = self.add(account, &value); // best effort: the extension reads only the shared group
        Ok(value)
    }

    fn add(&self, account: &str, value: &[u8]) -> Result<(), VaultError> {
        use security_framework::{
            access_control::{ProtectionMode, SecAccessControl},
            passwords::{set_generic_password_options, PasswordOptions},
        };
        let mut options = PasswordOptions::new_generic_password(SERVICE, account);
        // Not synced to iCloud, not in backups, readable by background work after first unlock.
        options.set_access_synchronized(Some(false));
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
            0,
        )
        .map_err(|e| classify(&e))?;
        options.set_access_control(access);
        // The notification extension shares this group (the app-id group: the simulator has no entitlements for it).
        #[cfg(not(target_abi = "sim"))]
        options.set_access_group(SHARED_GROUP);
        set_generic_password_options(value, options).map_err(|e| classify(&e))
    }

    fn remove(&self, account: &str) -> Result<(), VaultError> {
        security_framework::passwords::delete_generic_password(SERVICE, account)
            .map_err(|e| classify(&e))
    }
}

#[cfg(target_os = "macos")]
impl AccountKeychain {
    pub fn new() -> Self {
        Self { keychain: None }
    }

    /// Same contract against an explicit Keychain file (disposable test keychains).
    pub fn in_keychain(keychain: SecKeychain) -> Self {
        Self {
            keychain: Some(keychain),
        }
    }

    fn keychain(&self) -> Result<SecKeychain, VaultError> {
        match &self.keychain {
            Some(k) => Ok(k.clone()),
            None => SecKeychain::default().map_err(|e| classify(&e)),
        }
    }

    fn find(&self, account: &str) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let (password, _) = self
            .keychain()?
            .find_generic_password(SERVICE, account)
            .map_err(|e| classify(&e))?;
        Ok(zeroize::Zeroizing::new(password.to_vec()))
    }

    fn add(&self, account: &str, value: &[u8]) -> Result<(), VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        self.keychain()?
            .add_generic_password(SERVICE, account, value)
            .map_err(|e| classify(&e))
    }

    fn remove(&self, account: &str) -> Result<(), VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let (_, item) = self
            .keychain()?
            .find_generic_password(SERVICE, account)
            .map_err(|e| classify(&e))?;
        item.delete();
        Ok(())
    }
}

impl AccountKeychain {
    fn name(id: AccountId) -> String {
        format!("account-id:{}", id.as_u64())
    }

    fn store_new(&self, account: &str, value: &[u8]) -> Result<(), VaultError> {
        match self.find(account) {
            Ok(_) => return Err(VaultError::AlreadyExists),
            Err(VaultError::Missing) => {}
            Err(e) => return Err(e),
        }
        self.add(account, value)
    }

    pub fn create(&self, id: AccountId) -> Result<Secret, VaultError> {
        let mut random = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        let secret = crate::signin::secret_from_random(&random)?;
        self.store_new(&Self::name(id), secret.0.as_bytes())?;
        Ok(secret)
    }

    pub fn read(&self, id: AccountId) -> Result<Secret, VaultError> {
        crate::signin::secret_from_item(&self.find(&Self::name(id))?)
    }

    /// Removes only this account's item; an already absent item is fine.
    pub fn delete(&self, id: AccountId) -> Result<(), VaultError> {
        self.delete_named(&Self::name(id))
    }

    fn delete_named(&self, name: &str) -> Result<(), VaultError> {
        match self.remove(name) {
            Err(VaultError::Missing) => Ok(()),
            other => other,
        }
    }

    /// Removes the path-keyed item of one account store; an already absent item is fine.
    pub fn delete_store(&self, path: &std::path::Path) -> Result<(), VaultError> {
        self.delete_named(path.to_str().ok_or(VaultError::Unavailable)?)
    }

    /// Copies the single path-keyed item of the old install to the account's own item (the
    /// first account of the catalog); the original stays. No-op when the account has one.
    #[cfg(target_os = "macos")]
    pub fn migrate_store_secret(
        &self,
        store: &AccountStore,
        id: AccountId,
    ) -> Result<(), VaultError> {
        match self.read(id) {
            Ok(_) => return Ok(()),
            Err(VaultError::Missing) => {}
            Err(e) => return Err(e),
        }
        let path = store.path.to_str().ok_or(VaultError::Unavailable)?;
        let value = self.find(path)?;
        crate::signin::secret_from_item(&value)?;
        self.store_new(&Self::name(id), &value)
    }
}

impl Default for AccountKeychain {
    fn default() -> Self {
        Self::new()
    }
}

/// The app's Keychain as a profile protection source: one item per account ID, so the store
/// path (which moves with the app container) never matters.
#[cfg(target_os = "ios")]
pub struct IosKeychain;
#[cfg(target_os = "ios")]
impl ProtectionSource for IosKeychain {
    fn create(&self, store: &crate::persistence::AccountStore) -> Result<Secret, VaultError> {
        self.create_account(store.account_id())
    }
    fn read(&self, store: &crate::persistence::AccountStore) -> Result<Secret, VaultError> {
        self.read_account(store.account_id())
    }
}
#[cfg(target_os = "ios")]
impl AccountProtectionSource for IosKeychain {
    fn create_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        // iOS keeps Keychain items after uninstall while the data dir is gone: an item for a
        // free ID is an orphan, so replace it instead of failing every future sign-in.
        AccountKeychain::new().delete(id)?;
        AccountKeychain::new().create(id)
    }
    fn read_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        AccountKeychain::new().read(id)
    }
    fn delete_account(&self, id: AccountId) -> Result<(), VaultError> {
        AccountKeychain::new().delete(id)
    }
}
