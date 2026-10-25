#![cfg(feature = "sdk")]
use super::{AccountStore, StoreError};
use matrix_domain::AccountId;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::{fs, path::PathBuf};
const PASSPHRASE: &str = "long disposable fixture passphrase";

struct TestRoot(PathBuf);
impl TestRoot {
    fn new(label: &str) -> Self {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("p017-{label}-{}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn account_store_has_one_owner_until_drop() {
    let root = TestRoot::new("exclusive");
    let owner = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    assert!(
        AccountStore::open(&root.0, AccountId::new(17)).is_err(),
        "a second owner must not mutate the same SQLite/credential store"
    );
    drop(owner);
    assert!(AccountStore::open(&root.0, AccountId::new(17)).is_ok());
}

#[test]
fn encrypted_store_roundtrip_binds_local_account() {
    let root = TestRoot::new("roundtrip");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    let secret = serde_json::json!({"token": "synthetic-p017-test-token"});
    store.seal(PASSPHRASE, &secret).unwrap();
    drop(store);
    let reopened = AccountStore::open(&root.0, AccountId::new(17)).unwrap();
    assert!(reopened.unseal(PASSPHRASE).unwrap() == secret);
    let raw = fs::read(root.0.join("account-17/session.sealed")).unwrap();
    assert!(
        !raw.windows(b"synthetic-p017-test-token".len())
            .any(|w| w == b"synthetic-p017-test-token")
    );
    assert_eq!(
        fs::metadata(&reopened.path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(reopened.path.join("session.sealed"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn wrong_passphrase_does_not_change_encrypted_session() {
    let root = TestRoot::new("wrong-key");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    store
        .seal(PASSPHRASE, &serde_json::json!({"token": "synthetic"}))
        .unwrap();
    let before = fs::read(store.path.join("session.sealed")).unwrap();
    assert!(matches!(
        store.unseal("wrong disposable fixture passphrase"),
        Err(StoreError::LockedOrCorrupt)
    ));
    assert_eq!(before, fs::read(store.path.join("session.sealed")).unwrap());
    assert!(store.unseal(PASSPHRASE).is_ok());
}

#[test]
fn swapped_account_record_is_rejected_even_with_same_passphrase() {
    let root = TestRoot::new("swapped");
    let alice = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    let bob = AccountStore::create(&root.0, AccountId::new(18)).unwrap();
    alice
        .seal(PASSPHRASE, &serde_json::json!({"token": "synthetic"}))
        .unwrap();
    fs::copy(
        alice.path.join("session.sealed"),
        bob.path.join("session.sealed"),
    )
    .unwrap();
    assert!(matches!(
        bob.unseal(PASSPHRASE),
        Err(StoreError::AccountMismatch)
    ));
}

#[test]
fn ciphertext_tampering_fails_closed() {
    let root = TestRoot::new("tamper");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    store
        .seal(PASSPHRASE, &serde_json::json!({"token": "synthetic"}))
        .unwrap();
    let path = store.path.join("session.sealed");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let payload_bytes: Vec<u8> = serde_json::from_value(record["payload"].clone()).unwrap();
    let mut payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
    let value = payload["ciphertext"][0].as_u64().unwrap();
    payload["ciphertext"][0] = serde_json::json!(value ^ 1);
    record["payload"] = serde_json::json!(serde_json::to_vec(&payload).unwrap());
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(matches!(
        store.unseal(PASSPHRASE),
        Err(StoreError::LockedOrCorrupt)
    ));
}

#[test]
fn existing_record_is_never_overwritten() {
    let root = TestRoot::new("no-overwrite");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    store.seal(PASSPHRASE, &serde_json::json!("first")).unwrap();
    assert_eq!(
        store.seal(PASSPHRASE, &serde_json::json!("second")),
        Err(StoreError::AlreadyExists)
    );
    assert!(store.unseal(PASSPHRASE).unwrap() == "first");
    assert!(!store.path.join("session.pending").exists());
    assert!(matches!(
        AccountStore::create(&root.0, AccountId::new(17)),
        Err(StoreError::AlreadyExists)
    ));
}

#[test]
fn weak_passphrase_never_publishes_credential() {
    let root = TestRoot::new("weak");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    assert_eq!(
        store.seal("", &serde_json::json!("synthetic")),
        Err(StoreError::WeakPassphrase)
    );
    assert_eq!(
        fs::read_dir(&store.path)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from(".lock")]
    );
}

#[test]
fn unsafe_root_or_session_permissions_fail_closed() {
    let root = TestRoot::new("permissions");
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        AccountStore::create(&root.0, AccountId::new(17)),
        Err(StoreError::UnsafePath)
    ));
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    store
        .seal(PASSPHRASE, &serde_json::json!("synthetic"))
        .unwrap();
    fs::set_permissions(
        store.path.join("session.sealed"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(matches!(
        store.unseal(PASSPHRASE),
        Err(StoreError::UnsafePath)
    ));
}

#[test]
fn symlinked_account_directory_is_rejected() {
    let root = TestRoot::new("symlink-dir");
    let target = root.0.join("private-target");
    fs::DirBuilder::new().mode(0o700).create(&target).unwrap();
    std::os::unix::fs::symlink(target, root.0.join("account-17")).unwrap();
    assert!(matches!(
        AccountStore::open(&root.0, AccountId::new(17)),
        Err(StoreError::UnsafePath)
    ));
}

#[test]
fn public_persistence_methods_reject_invalid_inputs_without_sdk_files() {
    let root = TestRoot::new("public-invalid");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
        let url = crate::discovery::HomeserverUrl::parse("https://fixture.invalid").unwrap();
        let result = store
            .password_login(url, "alice", "unused", PASSPHRASE)
            .await;
        assert!(matches!(result, Err(crate::auth::AuthError::InvalidUserId)));
        assert_eq!(
            fs::read_dir(root.0.join("account-17"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<Vec<_>>(),
            vec![std::ffi::OsString::from(".lock")]
        );
        let store = AccountStore::open(&root.0, AccountId::new(17)).unwrap();
        let url = crate::discovery::HomeserverUrl::parse("https://fixture.invalid").unwrap();
        let result = store
            .password_login(url, "@alice:fixture.invalid", "unused", "")
            .await;
        assert!(matches!(result, Err(crate::auth::AuthError::Storage)));
        let store = AccountStore::open(&root.0, AccountId::new(17)).unwrap();
        assert!(matches!(
            store.restore(PASSPHRASE).await,
            Err(crate::auth::AuthError::Storage)
        ));
        assert_eq!(
            fs::read_dir(root.0.join("account-17"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<Vec<_>>(),
            vec![std::ffi::OsString::from(".lock")]
        );
    });
}

#[test]
fn hardlinked_store_file_is_rejected_before_sdk_open() {
    let root = TestRoot::new("hardlink-file");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    let target = root.0.join("fixture-target");
    fs::write(&target, b"fixture never modified").unwrap();
    fs::hard_link(&target, store.path.join("matrix-sdk-state.sqlite3")).unwrap();
    assert!(matches!(
        AccountStore::open(&root.0, AccountId::new(17)),
        Err(StoreError::UnsafePath)
    ));
    assert_eq!(fs::read(target).unwrap(), b"fixture never modified");
}

#[test]
fn symlinked_store_file_is_rejected_before_sdk_open() {
    let root = TestRoot::new("symlink-file");
    let store = AccountStore::create(&root.0, AccountId::new(17)).unwrap();
    let target = root.0.join("fixture-target");
    fs::write(&target, b"fixture never modified").unwrap();
    std::os::unix::fs::symlink(&target, store.path.join("matrix-sdk-state.sqlite3")).unwrap();
    assert!(matches!(
        AccountStore::open(&root.0, AccountId::new(17)),
        Err(StoreError::UnsafePath)
    ));
    assert_eq!(fs::read(target).unwrap(), b"fixture never modified");
}

#[test]
fn profile_made_before_event_cache_opens_with_its_data_and_cache_on() {
    let root = TestRoot::new("event-cache");
    let store = AccountStore::create(&root.0, AccountId::new(270)).unwrap();
    let url = crate::discovery::HomeserverUrl::parse("https://example.invalid").unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        // A profile as created before the event cache was switched on.
        let old = matrix_sdk::Client::builder()
            .homeserver_url(url.as_str())
            .sqlite_store(&store.path, Some(PASSPHRASE))
            .build()
            .await
            .unwrap();
        assert!(!old.event_cache().has_subscribed());
        old.state_store()
            .set_custom_value(b"kept", b"before".to_vec())
            .await
            .unwrap();
        drop(old);
        let client = store
            .client(&url, PASSPHRASE, reqwest::Client::new())
            .await
            .expect("old profile opens");
        assert!(client.event_cache().has_subscribed());
        assert!(fs::read_dir(&store.path).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("event-cache")));
        assert_eq!(
            client.state_store().get_custom_value(b"kept").await.unwrap(),
            Some(b"before".to_vec())
        );
    });
}
