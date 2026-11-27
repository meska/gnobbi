//! Real macOS Keychain API against a DISPOSABLE keychain file; owner keychains are never opened.
use crate::{
    keychain::{AccountKeychain, Keychain, VaultError},
    persistence::AccountStore,
};
use matrix_domain::AccountId;
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn private_dir(tag: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("dime-kc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// Throwaway keychain file with a random one-use password; deleted on drop.
struct Disposable {
    dir: PathBuf,
    path: PathBuf,
    password: String,
}
impl Disposable {
    fn new(tag: &str) -> Self {
        let dir = private_dir(tag);
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).unwrap();
        let password: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let path = dir.join("test.keychain-db");
        CreateOptions::new()
            .password(&password)
            .prompt_user(false)
            .create(&path)
            .unwrap();
        Self {
            dir,
            path,
            password,
        }
    }
    fn open(path: &Path, password: &str) -> SecKeychain {
        let mut k = SecKeychain::open(path).unwrap();
        k.unlock(Some(password)).unwrap();
        k
    }
    fn keychain(&self) -> SecKeychain {
        Self::open(&self.path, &self.password)
    }
    fn lock(&self) {
        // The path is the only argument; no secret reaches the command line.
        assert!(
            Command::new("security")
                .args(["lock-keychain"])
                .arg(&self.path)
                .status()
                .unwrap()
                .success()
        );
    }
}
impl Drop for Disposable {
    fn drop(&mut self) {
        let _ = Command::new("security")
            .arg("delete-keychain")
            .arg(&self.path)
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn create_then_read_returns_the_same_secret_and_delete_removes_it() {
    let kc_file = Disposable::new("rw");
    let store = AccountStore::create(&private_dir("rw-store"), AccountId::new(1)).unwrap();
    let kc = Keychain::in_keychain(&store, kc_file.keychain());
    assert!(matches!(kc.read(), Err(VaultError::Missing)));
    let made = kc.create().unwrap();
    let read = kc.read().unwrap();
    assert_eq!(made.0.as_str(), read.0.as_str());
    assert_eq!(read.0.len(), 64);
    assert!(matches!(kc.create(), Err(VaultError::AlreadyExists)));
    assert_eq!(kc.read().unwrap().0.as_str(), made.0.as_str());
    kc.delete().unwrap();
    assert!(matches!(kc.read(), Err(VaultError::Missing)));
    assert!(matches!(kc.delete(), Err(VaultError::Missing)));
}

#[test]
fn item_exists_sees_an_orphan_without_reading_or_touching_it() {
    let kc_file = Disposable::new("avail");
    let store = AccountStore::create(&private_dir("avail-store"), AccountId::new(1)).unwrap();
    let account = store.path.to_str().unwrap().to_owned();
    let exists = || Keychain::item_exists(Some(kc_file.keychain()), &account);
    assert_eq!(exists(), Ok(false));
    let kc = Keychain::in_keychain(&store, kc_file.keychain());
    let made = kc.create().unwrap();
    assert_eq!(exists(), Ok(true));
    assert_eq!(kc.read().unwrap().0.as_str(), made.0.as_str());
    kc.delete().unwrap();
    assert_eq!(exists(), Ok(false));
}

#[test]
fn accounts_and_profiles_get_separate_secrets() {
    let kc_file = Disposable::new("sep");
    let (a_root, b_root) = (private_dir("sep-a"), private_dir("sep-b"));
    let a1 = AccountStore::create(&a_root, AccountId::new(1)).unwrap();
    let a2 = AccountStore::create(&a_root, AccountId::new(2)).unwrap();
    let b1 = AccountStore::create(&b_root, AccountId::new(1)).unwrap();
    let stores = [&a1, &a2, &b1];
    let secrets: Vec<String> = stores
        .iter()
        .map(|s| {
            let kc = Keychain::in_keychain(s, kc_file.keychain());
            kc.create().unwrap().0.as_str().to_owned()
        })
        .collect();
    assert!(secrets[0] != secrets[1] && secrets[0] != secrets[2] && secrets[1] != secrets[2]);
    for (store, want) in stores.iter().zip(&secrets) {
        let kc = Keychain::in_keychain(store, kc_file.keychain());
        assert_eq!(kc.read().unwrap().0.as_str(), want);
    }
}

#[test]
fn locked_keychain_never_prompts_and_keeps_the_item() {
    let kc_file = Disposable::new("lock");
    let store = AccountStore::create(&private_dir("lock-store"), AccountId::new(1)).unwrap();
    let empty = AccountStore::create(&private_dir("lock-empty"), AccountId::new(1)).unwrap();
    let made = Keychain::in_keychain(&store, kc_file.keychain())
        .create()
        .unwrap();
    kc_file.lock();
    let locked = SecKeychain::open(&kc_file.path).unwrap(); // opened, not unlocked
    let got = Keychain::in_keychain(&store, locked.clone())
        .read()
        .map(|_| ());
    assert_eq!(got, Err(VaultError::Locked));
    assert!(matches!(
        Keychain::in_keychain(&empty, locked).create(),
        Err(VaultError::Locked)
    ));
    let kc = Keychain::in_keychain(&store, kc_file.keychain());
    assert_eq!(kc.read().unwrap().0.as_str(), made.0.as_str());
}

/// Two processes: the first creates, a fresh second process reads the identical secret.
#[test]
fn secret_survives_an_actual_process_restart() {
    if let Ok(phase) = std::env::var("DIME_KC_PHASE") {
        let root = PathBuf::from(std::env::var("DIME_KC_ROOT").unwrap());
        let password = std::env::var("DIME_KC_PW").unwrap();
        let store_dir = root.join("store");
        let store = AccountStore::create(&store_dir, AccountId::new(1))
            .or_else(|_| AccountStore::open(&store_dir, AccountId::new(1)))
            .unwrap();
        let kc = Keychain::in_keychain(
            &store,
            Disposable::open(&root.join("test.keychain-db"), &password),
        );
        let out = root.join("secret.txt");
        if phase == "create" {
            std::fs::write(&out, kc.create().unwrap().0.as_str()).unwrap();
        } else {
            let want = std::fs::read_to_string(&out).unwrap();
            assert_eq!(kc.read().unwrap().0.as_str(), want);
        }
        return;
    }
    let kc_file = Disposable::new("restart");
    let store_dir = kc_file.dir.join("store");
    std::fs::create_dir(&store_dir).unwrap();
    std::fs::set_permissions(&store_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    for phase in ["create", "read"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "signin::keychain_tests::secret_survives_an_actual_process_restart",
                "--exact",
                "--test-threads=1",
            ])
            .env("DIME_KC_PHASE", phase)
            .env("DIME_KC_ROOT", &kc_file.dir)
            .env("DIME_KC_PW", &kc_file.password)
            .status()
            .unwrap();
        assert!(status.success(), "phase {phase}");
    }
}

/// Characterizes the real default (login) keychain from a session that cannot unlock it
/// (e.g. plain ssh). Only a unique throwaway account is used; if the write unexpectedly
/// succeeds the item is deleted again and the test fails, so nothing is left behind.
#[test]
#[ignore = "environment-dependent: run only from a background/ssh session"]
fn default_keychain_in_a_background_session_reports_locked_without_prompting() {
    let store = AccountStore::create(&private_dir("default"), AccountId::new(1)).unwrap();
    let kc = Keychain::new(&store);
    let got = kc.create().map(|_| ());
    if got.is_ok() {
        kc.delete().unwrap();
    }
    assert_eq!(got, Err(VaultError::Locked));
}

#[test]
fn per_account_items_are_isolated_and_the_old_item_migrates() {
    let kc_file = Disposable::new("acct");
    let k = || AccountKeychain::in_keychain(kc_file.keychain());
    let (one, two) = (AccountId::new(1), AccountId::new(2));
    let a = k().create(one).unwrap().0.as_str().to_owned();
    let b = k().create(two).unwrap().0.as_str().to_owned();
    assert!(a != b);
    assert!(matches!(k().create(one), Err(VaultError::AlreadyExists)));
    k().delete(one).unwrap();
    assert!(matches!(k().read(one), Err(VaultError::Missing)));
    assert_eq!(k().read(two).unwrap().0.as_str(), b);
    k().delete(one).unwrap();
    // Migration: the path-keyed item of the old install becomes the first account's item.
    let store = AccountStore::create(&private_dir("acct-store"), AccountId::new(1)).unwrap();
    let old = Keychain::in_keychain(&store, kc_file.keychain())
        .create()
        .unwrap();
    k().migrate_store_secret(&store, one).unwrap();
    assert_eq!(k().read(one).unwrap().0.as_str(), old.0.as_str());
    assert_eq!(k().read(two).unwrap().0.as_str(), b);
    assert_eq!(
        Keychain::in_keychain(&store, kc_file.keychain())
            .read()
            .unwrap()
            .0
            .as_str(),
        old.0.as_str()
    );
}
