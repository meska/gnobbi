//! Windows Credential Manager protection-secret lifecycle; call only on a blocking backend worker.
//! One generic credential per account store, named after the store path, so another profile or
//! account never resolves to this secret. Persisted per machine and user, never roamed.
use crate::{persistence::AccountStore, signin::Secret};
use matrix_domain::AccountId;
use std::fmt::Write;
use windows::{
    Win32::{
        Foundation::{ERROR_NOT_FOUND, FILETIME},
        Security::Credentials::{
            CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW,
            CredFree, CredReadW, CredWriteW,
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

pub use crate::signin::VaultError;

const SERVICE: &str = "gnobbi.sdk-protection-v1";

/// The Windows Credential Manager as a profile protection source.
pub struct CredentialManager;
impl crate::signin::ProtectionSource for CredentialManager {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        create(&store_name(&store.path)?)
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        read(&store_name(&store.path)?)
    }
}

fn store_name(path: &std::path::Path) -> Result<String, VaultError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(VaultError::Unavailable)
}

fn target(name: &str) -> Vec<u16> {
    format!("{SERVICE}/{name}")
        .encode_utf16()
        .chain([0])
        .collect()
}

fn classify(error: &windows::core::Error) -> VaultError {
    if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
        VaultError::Missing
    } else {
        VaultError::Unavailable
    }
}

fn read(name: &str) -> Result<Secret, VaultError> {
    let target = target(name);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target` is NUL-terminated; on success `found` is a Credential Manager
    // allocation that is copied out and then released with CredFree.
    let value = unsafe {
        CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut found)
            .map_err(|e| classify(&e))?;
        let blob = std::slice::from_raw_parts(
            (*found).CredentialBlob,
            (*found).CredentialBlobSize as usize,
        );
        let copy = zeroize::Zeroizing::new(blob.to_vec());
        CredFree(found as *const _);
        copy
    };
    if value.len() != 64 || !value.iter().all(u8::is_ascii_hexdigit) {
        return Err(VaultError::InvalidItem);
    }
    let text = std::str::from_utf8(&value).map_err(|_| VaultError::InvalidItem)?;
    Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
}

fn create(name: &str) -> Result<Secret, VaultError> {
    match read(name) {
        Ok(_) | Err(VaultError::InvalidItem) => return Err(VaultError::AlreadyExists),
        Err(VaultError::Missing) => {}
        Err(e) => return Err(e),
    }
    let mut random = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *random).map_err(|_| VaultError::Unavailable)?;
    let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
    for byte in random.iter() {
        write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
    }
    let mut target = target(name);
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: text.len() as u32,
        CredentialBlob: text.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: the structure and the buffers it points to outlive the call; CredWriteW copies them.
    unsafe { CredWriteW(&credential, 0) }.map_err(|e| classify(&e))?;
    Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
}

/// Removes one credential; an already absent one is fine.
fn delete(name: &str) -> Result<(), VaultError> {
    let target = target(name);
    // SAFETY: `target` is NUL-terminated.
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Err(e) if classify(&e) != VaultError::Missing => Err(VaultError::Unavailable),
        _ => Ok(()),
    }
}

pub fn delete_account_secret(id: AccountId) -> Result<(), VaultError> {
    delete(&format!("account-id:{}", id.as_u64()))
}

pub fn delete_store_secret(path: &std::path::Path) -> Result<(), VaultError> {
    delete(&store_name(path)?)
}

/// Real Credential Manager + sealed store. Phases (GNOBBI_WINCRED_PHASE=write|read|clean, root in
/// GNOBBI_WINCRED_ROOT) run in separate processes to prove the data survives a restart; with no
/// phase the whole cycle runs in one process under %LOCALAPPDATA%.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{persistence::AccountStore, signin::ProtectionSource};
    use std::path::PathBuf;

    const ID: u64 = 207;

    fn root() -> PathBuf {
        std::env::var_os("GNOBBI_WINCRED_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
                    .join(format!("Gnobbi-test-{}", std::process::id()))
            })
    }

    fn write(root: &std::path::Path) {
        let _ = crate::fsx::create_private_dir(root);
        let store = AccountStore::create(root, AccountId::new(ID)).unwrap();
        let secret = CredentialManager.create(&store).unwrap();
        assert!(matches!(
            CredentialManager.create(&store),
            Err(VaultError::AlreadyExists)
        ));
        store
            .seal(&secret.0, &serde_json::json!({"probe": "p207"}))
            .unwrap();
        println!(
            "WRITE ok: store={} secret_len={}",
            store.path.display(),
            secret.0.len()
        );
    }

    fn read(root: &std::path::Path) {
        let store = AccountStore::open(root, AccountId::new(ID)).unwrap();
        let secret = CredentialManager.read(&store).unwrap();
        let value = store.unseal(&secret.0).unwrap();
        assert_eq!(value, serde_json::json!({"probe": "p207"}));
        assert!(store.unseal(&"x".repeat(64)).is_err());
        println!("READ ok: store={} unsealed={value}", store.path.display());
    }

    fn clean(root: &std::path::Path) {
        let store = AccountStore::open(root, AccountId::new(ID)).unwrap();
        delete_store_secret(&store.path).unwrap();
        assert!(matches!(
            CredentialManager.read(&store),
            Err(VaultError::Missing)
        ));
        delete_store_secret(&store.path).unwrap();
        let path = store.path.clone();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
        println!("CLEAN ok: {}", path.display());
    }

    #[test]
    #[ignore = "needs a Windows logon session (not SSH): run with --ignored under PsExec -s, see the WINDOWS_SPIKE notes"]
    fn credential_and_sealed_store_survive_reopen() {
        let root = root();
        match std::env::var("GNOBBI_WINCRED_PHASE").as_deref() {
            Ok("write") => write(&root),
            Ok("read") => read(&root),
            Ok("clean") => clean(&root),
            _ => {
                write(&root);
                read(&root);
                clean(&root);
            }
        }
    }
}
