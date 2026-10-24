#![cfg(all(feature = "linux-vault", target_os = "linux"))]
use matrix_domain::AccountId;
use matrix_engine::{
    persistence::AccountStore,
    vault::{ProtectionVault, VaultError},
};
use std::path::PathBuf;

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn locked_vault_never_prompts_or_replaces_a_key() {
    assert_eq!(
        std::env::var("DIME_VAULT_FIXTURE").as_deref(),
        Ok("isolated")
    );
    let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join("locked");
    std::fs::create_dir(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    ProtectionVault::new(&store).create().unwrap();
    let empty = AccountStore::create(&root, AccountId::new(2)).unwrap();
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = service.get_default_collection().unwrap();
    collection.lock().unwrap();
    assert!(collection.is_locked().unwrap());
    assert!(matches!(
        ProtectionVault::new(&store).read(),
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        ProtectionVault::new(&store).create(),
        Err(VaultError::AlreadyExists)
    ));
    assert!(matches!(
        ProtectionVault::new(&empty).create(),
        Err(VaultError::Locked)
    ));
    assert!(collection.is_locked().unwrap());
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn generates_protection_secret_in_real_vault() {
    assert_eq!(
        std::env::var("DIME_VAULT_FIXTURE").as_deref(),
        Ok("isolated")
    );
    let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join("profile");
    std::fs::create_dir(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    assert!(ProtectionVault::new(&store).create().is_ok());
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn refuses_preexisting_protection_item_without_changing_it() {
    assert_eq!(
        std::env::var("DIME_VAULT_FIXTURE").as_deref(),
        Ok("isolated")
    );
    let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join("existing");
    std::fs::create_dir(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = service.get_default_collection().unwrap();
    let path = root.join("account-1");
    let attributes = std::collections::HashMap::from([
        ("application", "gnobbi"),
        ("purpose", "sdk-protection-v1"),
        ("account-store", path.to_str().unwrap()),
    ]);
    let original = collection
        .create_item(
            "synthetic preexisting",
            attributes.clone(),
            b"existing-synthetic-value",
            false,
            "text/plain",
        )
        .unwrap();
    assert!(matches!(
        ProtectionVault::new(&store).create(),
        Err(VaultError::AlreadyExists)
    ));
    let items = collection.search_items(attributes).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item_path, original.item_path);
    let value = zeroize::Zeroizing::new(items[0].get_secret().unwrap());
    assert!(value.as_slice() == b"existing-synthetic-value");
}

fn unavailable_without_service(fixture: &str) {
    assert_eq!(std::env::var("DIME_VAULT_FIXTURE").as_deref(), Ok(fixture));
    let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join("profile");
    std::fs::create_dir(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    assert!(matches!(
        ProtectionVault::new(&store).create(),
        Err(VaultError::Unavailable)
    ));
    assert!(matches!(
        ProtectionVault::new(&store).read(),
        Err(VaultError::Unavailable)
    ));
}

#[test]
#[ignore = "requires DIME_VAULT_FIXTURE=absent with a bus address that has no listener"]
fn absent_bus_fails_closed_without_inventing_a_key() {
    unavailable_without_service("absent");
}

#[test]
#[ignore = "requires DIME_VAULT_FIXTURE=denied with a private bus whose policy denies the service"]
fn denied_service_fails_closed_without_inventing_a_key() {
    unavailable_without_service("denied");
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn reads_old_name_entry_without_changing_it_and_reports_missing() {
    assert_eq!(
        std::env::var("DIME_VAULT_FIXTURE").as_deref(),
        Ok("isolated")
    );
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let old = home.join("old-profile/account-1");
    let other = home.join("old-profile/account-2");
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = service.get_default_collection().unwrap();
    let attributes = std::collections::HashMap::from([
        ("application", "dime"),
        ("purpose", "sdk-protection-v1"),
        ("account-store", old.to_str().unwrap()),
    ]);
    let seeded = "ab".repeat(32);
    let item = collection
        .create_item(
            "synthetic old",
            attributes.clone(),
            seeded.as_bytes(),
            false,
            "text/plain",
        )
        .unwrap();
    assert!(matrix_engine::vault::read_legacy(&old).is_ok());
    assert!(matches!(
        matrix_engine::vault::read_legacy(&other),
        Err(VaultError::Missing)
    ));
    let items = collection.search_items(attributes).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item_path, item.item_path);
    assert_eq!(items[0].get_secret().unwrap(), seeded.as_bytes());
}
