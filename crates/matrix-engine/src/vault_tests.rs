use crate::{persistence::AccountStore, vault::ProtectionVault};
use matrix_domain::AccountId;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};
use zeroize::Zeroizing;

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn refuses_untrusted_or_ambiguous_native_items() {
    use crate::vault::VaultError;
    let root = private_root("invalid-item");
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = service.get_default_collection().unwrap();
    let path = root.join("account-1");
    let attrs = std::collections::HashMap::from([
        ("application", "dime"),
        ("purpose", "sdk-protection-v1"),
        ("account-store", path.to_str().unwrap()),
    ]);
    for value in [
        b"short".as_slice(),
        &[b'g'; 64],
        &[255; 64],
        &[b'a'; 65],
        b"",
    ] {
        let item = collection
            .create_item(
                "malformed fixture",
                attrs.clone(),
                value,
                false,
                "text/plain",
            )
            .unwrap();
        assert!(matches!(
            ProtectionVault::new(&store).read(),
            Err(VaultError::InvalidItem)
        ));
        assert!(matches!(
            ProtectionVault::new(&store).create(),
            Err(VaultError::AlreadyExists)
        ));
        assert!(Zeroizing::new(item.get_secret().unwrap()).as_slice() == value);
        item.delete().unwrap();
    }
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn refuses_duplicate_matching_items_instead_of_choosing_one() {
    use crate::vault::VaultError;
    let root = private_root("duplicate");
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    ProtectionVault::new(&store).create().unwrap();
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = service.get_default_collection().unwrap();
    let path = root.join("account-1");
    let attrs = std::collections::HashMap::from([
        ("application", "dime"),
        ("purpose", "sdk-protection-v1"),
        ("account-store", path.to_str().unwrap()),
    ]);
    collection
        .create_item(
            "duplicate fixture",
            attrs.clone(),
            &[b'b'; 64],
            false,
            "text/plain",
        )
        .unwrap();
    assert!(matches!(
        ProtectionVault::new(&store).read(),
        Err(VaultError::InvalidItem)
    ));
    assert!(matches!(
        ProtectionVault::new(&store).create(),
        Err(VaultError::AlreadyExists)
    ));
    assert_eq!(collection.search_items(attrs).unwrap().len(), 2);
}

fn private_root(name: &str) -> PathBuf {
    assert_eq!(
        std::env::var("DIME_VAULT_FIXTURE").as_deref(),
        Ok("isolated")
    );
    let root = PathBuf::from(std::env::var_os("HOME").unwrap()).join(name);
    std::fs::create_dir(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn reads_exact_generated_secret_after_client_process_restart() {
    if let Some(root) = std::env::var_os("DIME_VAULT_CHILD_ROOT") {
        let store = AccountStore::open(&PathBuf::from(root), AccountId::new(1)).unwrap();
        let value = ProtectionVault::new(&store).read().unwrap();
        let mut expected = Zeroizing::new(Vec::new());
        std::io::stdin()
            .take(65)
            .read_to_end(&mut expected)
            .unwrap();
        assert!(value.0.as_bytes() == expected.as_slice());
        return;
    }
    let root = private_root("restart");
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    let generated = ProtectionVault::new(&store).create().unwrap();
    assert_eq!(generated.0.len(), 64);
    assert!(generated.0.bytes().all(|byte| byte.is_ascii_hexdigit()));
    drop(store);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "signin::vault_tests::reads_exact_generated_secret_after_client_process_restart",
            "--nocapture",
        ])
        .env("DIME_VAULT_CHILD_ROOT", &root)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(generated.0.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(!PathBuf::from(format!("/proc/{}", child.id())).exists());
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn exact_profile_account_namespace_never_creates_on_missing_read() {
    use crate::vault::VaultError;
    let first = private_root("first");
    let second = private_root("second");
    let a = AccountStore::create(&first, AccountId::new(1)).unwrap();
    let b = AccountStore::create(&first, AccountId::new(2)).unwrap();
    let c = AccountStore::create(&second, AccountId::new(1)).unwrap();
    let key = ProtectionVault::new(&a).create().unwrap();
    for store in [&b, &c] {
        assert!(matches!(
            ProtectionVault::new(store).read(),
            Err(VaultError::Missing)
        ));
        assert!(matches!(
            ProtectionVault::new(store).read(),
            Err(VaultError::Missing)
        ));
        let other = ProtectionVault::new(store).create().unwrap();
        assert!(other.0.as_str() != key.0.as_str());
        assert!(ProtectionVault::new(store).read().unwrap().0.as_str() == other.0.as_str());
    }
    assert!(ProtectionVault::new(&a).read().unwrap().0.as_str() == key.0.as_str());
}
