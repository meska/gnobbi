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
    /// An item left by an earlier install (the Keychain outlives the app) makes the id unusable.
    fn available(&self, path: &std::path::Path) -> Result<bool, VaultError> {
        Keychain::item_exists(None, path.to_str().ok_or(VaultError::Unavailable)?).map(|exists| !exists)
    }
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

    /// Whether an item exists for this account (never reads the secret).
    pub(crate) fn item_exists(keychain: Option<SecKeychain>, account: &str) -> Result<bool, VaultError> {
        let _no_prompt = SecKeychain::disable_user_interaction().map_err(|e| classify(&e))?;
        let keychain = match keychain {
            Some(k) => k,
            None => SecKeychain::default().map_err(|e| classify(&e))?,
        };
        match keychain.find_generic_password(SERVICE, account) {
            Ok(_) => Ok(true),
            Err(e) => match classify(&e) {
                VaultError::Missing => Ok(false),
                other => Err(other),
            },
        }
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

    /// Read exactly the requested namespace; the legacy item may belong to another profile.
    fn find_group(&self, account: &str, shared: bool) -> Result<zeroize::Zeroizing<Vec<u8>>, VaultError> {
        use security_framework::passwords::{generic_password, PasswordOptions};
        let mut options = PasswordOptions::new_generic_password(SERVICE, account);
        #[cfg(not(target_abi = "sim"))]
        if shared { options.set_access_group(SHARED_GROUP); }
        #[cfg(target_abi = "sim")]
        let _ = shared;
        generic_password(options).map(zeroize::Zeroizing::new).map_err(|e| {
            eprintln!("Gnobbi: keychain lookup group={} result=code:{}", if shared { "shared" } else { "default" }, e.code());
            classify(&e)
        })
    }

    fn find_all_groups(&self, account: &str) -> Result<Vec<zeroize::Zeroizing<Vec<u8>>>, VaultError> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};
        let mut options = ItemSearchOptions::new();
        options.class(ItemClass::generic_password()).service(SERVICE).account(account).load_data(true).limit(Limit::All);
        match options.search() {
            Ok(items) => Ok(items.into_iter().filter_map(|item| match item {
                SearchResult::Data(data) => Some(zeroize::Zeroizing::new(data)),
                _ => None,
            }).collect()),
            Err(e) if classify(&e) == VaultError::Missing => Ok(Vec::new()),
            Err(e) => Err(classify(&e)),
        }
    }

    pub fn delete_profile_store(&self, path: &std::path::Path) -> Result<(), VaultError> {
        let name = crate::signin::ios_profile_item(path).ok_or(VaultError::Unavailable)?;
        self.delete_named(&name)
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
    fn available(&self, path: &std::path::Path) -> Result<bool, VaultError> {
        let name = crate::signin::ios_profile_item(path).ok_or(VaultError::Unavailable)?;
        let keychain = AccountKeychain::new();
        for shared in [true, false] {
            match keychain.find_group(&name, shared) {
                Ok(_) => return Ok(false),
                Err(VaultError::Missing) => (),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
    fn create(&self, store: &crate::persistence::AccountStore) -> Result<Secret, VaultError> {
        let name = crate::signin::ios_profile_item(&store.path).ok_or(VaultError::Unavailable)?;
        let mut random = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
        let secret = crate::signin::secret_from_random(&random)?;
        AccountKeychain::new().store_new(&name, secret.0.as_bytes())?;
        Ok(secret)
    }
    fn read(&self, store: &crate::persistence::AccountStore) -> Result<Secret, VaultError> {
        let name = crate::signin::ios_profile_item(&store.path).ok_or(VaultError::Unavailable)?;
        let keychain = AccountKeychain::new();
        eprintln!("Gnobbi: unlock profile={name} attempt");
        let legacy = AccountKeychain::name(store.account_id());
        let order = crate::signin::ios_lookup_order(&name, &legacy);
        match keychain.find_group(order[0].0, order[0].1) {
            Ok(value) => {
                let secret = crate::signin::secret_from_item(&value)?;
                if store.unseal(&secret.0).is_ok() {
                    eprintln!("Gnobbi: unlock profile={name} group=shared result=Ok");
                    return Ok(secret);
                }
                // Never replace an existing profile-specific item with an ambiguous legacy value.
                eprintln!("Gnobbi: unlock profile={name} group=shared result=Error");
                return Err(VaultError::InvalidItem);
            }
            Err(VaultError::Missing) => (),
            Err(e) => return Err(e),
        }
        for (item, shared) in order.into_iter().skip(1) {
            let values = if shared {
                match keychain.find_group(item, true) {
                    Ok(value) => vec![value],
                    Err(VaultError::Missing) => Vec::new(),
                    Err(e) => return Err(e),
                }
            } else {
                keychain.find_all_groups(item)?
            };
            if let Some(value) = crate::signin::ios_first_valid(values, |value| {
                crate::signin::secret_from_item(value).is_ok_and(|secret| store.unseal(&secret.0).is_ok())
            }) {
                keychain.store_new(&name, &value)?;
                let copied = keychain.find_group(&name, true)?;
                let verified = crate::signin::secret_from_item(&copied)?;
                if store.unseal(&verified.0).is_err() { return Err(VaultError::InvalidItem); }
                eprintln!("Gnobbi: unlock profile={name} group={} result=Ok migration=copied", if shared { "shared" } else { "accessible" });
                return Ok(verified);
            }
        }
        eprintln!("Gnobbi: unlock profile={name} result=Missing");
        Err(VaultError::Missing)
    }
}
#[cfg(target_os = "ios")]
impl AccountProtectionSource for IosKeychain {
    fn create_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        AccountKeychain::new().create(id)
    }
    fn read_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        AccountKeychain::new().read(id)
    }
    fn delete_account(&self, _id: AccountId) -> Result<(), VaultError> {
        // The unscoped legacy item may still be needed by another profile.
        Ok(())
    }
    fn delete_profile(&self, _id: AccountId, store: &std::path::Path) -> Result<(), VaultError> {
        AccountKeychain::new().delete_profile_store(store)
    }
}
