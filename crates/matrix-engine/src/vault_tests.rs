use crate::{
    persistence::AccountStore,
    vault::{
        ProtectionVault, VaultError, create_account_secret, delete_account_secret,
        delete_store_secret, migrate_store_secret, read_account_secret,
    },
};
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
        ("application", "gnobbi"),
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
        ("application", "gnobbi"),
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

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn per_account_items_are_isolated_and_the_old_item_migrates() {
    let (one, two) = (AccountId::new(1), AccountId::new(2));
    let a = create_account_secret(one).unwrap();
    let b = create_account_secret(two).unwrap();
    assert!(a.0.as_str() != b.0.as_str());
    assert!(matches!(
        create_account_secret(one),
        Err(VaultError::AlreadyExists)
    ));
    delete_account_secret(one).unwrap();
    assert!(matches!(read_account_secret(one), Err(VaultError::Missing)));
    assert_eq!(read_account_secret(two).unwrap().0.as_str(), b.0.as_str());
    delete_account_secret(one).unwrap();
    // The path-keyed item of the old install is not an account item and survives deletes.
    let root = private_root("migrate");
    let store = AccountStore::create(&root, AccountId::new(1)).unwrap();
    let old = ProtectionVault::new(&store).create().unwrap();
    assert!(matches!(read_account_secret(one), Err(VaultError::Missing)));
    migrate_store_secret(&store, one).unwrap();
    assert_eq!(read_account_secret(one).unwrap().0.as_str(), old.0.as_str());
    assert_eq!(read_account_secret(two).unwrap().0.as_str(), b.0.as_str());
    assert_eq!(
        ProtectionVault::new(&store).read().unwrap().0.as_str(),
        old.0.as_str()
    );
    migrate_store_secret(&store, one).unwrap();
    delete_account_secret(two).unwrap();
    assert!(matches!(read_account_secret(two), Err(VaultError::Missing)));
    assert_eq!(read_account_secret(one).unwrap().0.as_str(), old.0.as_str());
}

#[test]
#[ignore = "requires disposable Secret Service on an isolated bus"]
fn forgetting_an_account_removes_its_two_items_and_no_other() {
    let (one, two) = (AccountId::new(11), AccountId::new(12));
    let (root_one, root_two) = (private_root("forget-one"), private_root("forget-two"));
    let store_one = AccountStore::create(&root_one, AccountId::new(1)).unwrap();
    let store_two = AccountStore::create(&root_two, AccountId::new(1)).unwrap();
    create_account_secret(one).unwrap();
    let kept = create_account_secret(two).unwrap();
    ProtectionVault::new(&store_one).create().unwrap();
    let kept_store = ProtectionVault::new(&store_two).create().unwrap();
    crate::signin::forget_secrets(one, std::slice::from_ref(&store_one.path)).unwrap();
    assert!(matches!(read_account_secret(one), Err(VaultError::Missing)));
    assert!(matches!(
        ProtectionVault::new(&store_one).read(),
        Err(VaultError::Missing)
    ));
    assert_eq!(
        read_account_secret(two).unwrap().0.as_str(),
        kept.0.as_str()
    );
    assert_eq!(
        ProtectionVault::new(&store_two).read().unwrap().0.as_str(),
        kept_store.0.as_str()
    );
    crate::signin::forget_secrets(one, std::slice::from_ref(&store_one.path)).unwrap();
    delete_account_secret(two).unwrap();
    delete_store_secret(&store_two.path).unwrap();
}
