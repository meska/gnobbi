#![cfg(all(feature = "sdk", unix))]
use matrix_engine::persistence::AccountStore;
use matrix_engine::signin::{ProtectionSource, Secret, SignInError, SignInProfile, VaultError};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::{
    cell::Cell,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct PrivateRoot(PathBuf);
impl PrivateRoot {
    fn new() -> Self {
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!(
                "vault-profile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn private_file(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct FakeSource {
    read: Result<(), VaultError>,
    reads: Cell<u32>,
    creates: Cell<u32>,
}
impl FakeSource {
    fn failing(error: VaultError) -> Self {
        Self {
            read: Err(error),
            reads: Cell::new(0),
            creates: Cell::new(0),
        }
    }
}
impl ProtectionSource for FakeSource {
    fn create(&self, _: &AccountStore) -> Result<Secret, VaultError> {
        self.creates.set(self.creates.get() + 1);
        Err(VaultError::Unavailable)
    }
    fn read(&self, _: &AccountStore) -> Result<Secret, VaultError> {
        self.reads.set(self.reads.get() + 1);
        self.read?;
        Ok(Secret::new("a".repeat(64)).unwrap())
    }
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn vault_unlock_on_a_profile_without_vault_mode_never_touches_the_vault() {
    let root = PrivateRoot::new();
    root.private_file("active-account", "1");
    let mut profile = SignInProfile::open(root.0.clone()).unwrap();
    let source = FakeSource::failing(VaultError::Missing);
    let result = block_on(profile.unlock_vault(&source, None));
    assert!(matches!(result, Err(SignInError::ProtectionMismatch)));
    assert_eq!((source.reads.get(), source.creates.get()), (0, 0));
}

#[test]
fn phrase_unlock_on_a_vault_profile_is_refused_before_any_store_access() {
    let root = PrivateRoot::new();
    root.private_file("active-account", "1");
    root.private_file("protection-mode", "vault-v1");
    let mut profile = SignInProfile::open(root.0.clone()).unwrap();
    let result = block_on(profile.unlock(Secret::new("local unlock phrase".into()).unwrap(), None));
    assert!(matches!(result, Err(SignInError::ProtectionMismatch)));
}

#[test]
fn damaged_protection_mode_is_a_storage_error_not_a_silent_legacy_profile() {
    for text in ["", "vault-v2", "vault-v1\n", "legacy"] {
        let root = PrivateRoot::new();
        root.private_file("active-account", "1");
        root.private_file("protection-mode", text);
        assert!(matches!(
            SignInProfile::open(root.0.clone()),
            Err(SignInError::Storage)
        ));
    }
}

#[test]
fn missing_vault_item_is_reported_without_creating_a_key_or_touching_the_store() {
    let root = PrivateRoot::new();
    root.private_file("active-account", "1");
    root.private_file("protection-mode", "vault-v1");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(root.0.join("account-1"))
        .unwrap();
    let mut profile = SignInProfile::open(root.0.clone()).unwrap();
    let source = FakeSource::failing(VaultError::Missing);
    let result = block_on(profile.unlock_vault(&source, None));
    assert!(matches!(
        result,
        Err(SignInError::Vault(VaultError::Missing))
    ));
    assert_eq!((source.reads.get(), source.creates.get()), (1, 0));
    let mut names: Vec<_> = std::fs::read_dir(root.0.join("account-1"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, [std::ffi::OsString::from(".lock")]);
}
