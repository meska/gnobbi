//! macOS Keychain protection-secret lifecycle; call only on a blocking backend worker.
use crate::{persistence::AccountStore, signin::Secret};
use security_framework::os::macos::keychain::SecKeychain;

pub use crate::signin::VaultError;

const SERVICE: &str = "gnobbi.sdk-protection-v1";
// Keychain OSStatus codes (Security/SecBase.h).
const ITEM_NOT_FOUND: i32 = -25300;
const INTERACTION_NOT_ALLOWED: i32 = -25308;
// A locked keychain with prompts disabled reports auth-failed; both mean "needs the user to unlock".
const AUTH_FAILED: i32 = -25293;

/// The login Keychain as a profile protection source.
pub struct MacKeychain;
impl crate::signin::ProtectionSource for MacKeychain {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        Keychain::new(store).create()
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        Keychain::new(store).read()
    }
}

fn classify(error: &security_framework::base::Error) -> VaultError {
    match error.code() {
        ITEM_NOT_FOUND => VaultError::Missing,
        INTERACTION_NOT_ALLOWED | AUTH_FAILED => VaultError::Locked,
        _ => VaultError::Unavailable,
    }
}

/// Borrows the account's exclusive store lock; the item account is that store's path,
/// so another profile or account never resolves to this secret.
pub struct Keychain<'a> {
    store: &'a AccountStore,
    /// None = the user's default (login) Keychain.
    keychain: Option<SecKeychain>,
}
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
