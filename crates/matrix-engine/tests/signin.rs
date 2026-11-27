#![cfg(all(feature = "sdk", unix))]
use matrix_domain::AccountId;
use matrix_engine::signin::{PasswordSignIn, Secret, SignInError, SignInProfile, UnlockSession};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::PermissionsExt;
use std::{
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
                "signin-contract-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn secret_input_is_bounded_in_bytes_and_zeroizing() {
    fn requires_zeroizing<T: zeroize::ZeroizeOnDrop>() {}
    requires_zeroizing::<Secret>();
    assert!(matches!(
        Secret::new(String::new()),
        Err(SignInError::InvalidSecret)
    ));
    assert!(Secret::new("x".repeat(4096)).is_ok());
    assert!(matches!(
        Secret::new("x".repeat(4097)),
        Err(SignInError::InvalidSecret)
    ));
    assert!(matches!(
        Secret::new("è".repeat(2049)),
        Err(SignInError::InvalidSecret)
    ));
}

#[test]
fn sign_in_validates_the_exact_endpoint_and_user_before_network_or_storage() {
    let root = PrivateRoot::new();
    for endpoint in [
        "http://example.invalid",
        "https://user:pass@example.invalid",
        "https://example.invalid/?token=x",
    ] {
        let result = PasswordSignIn::new(
            root.0.clone(),
            AccountId::new(1),
            endpoint.into(),
            "@alice:example.invalid".into(),
            Secret::new("fixture".into()).unwrap(),
            Secret::new("local unlock phrase".into()).unwrap(),
            None,
        );
        assert!(matches!(result, Err(SignInError::InvalidHomeserver)));
    }
    // Lenient input is deliberate: surrounding whitespace is trimmed, and a short user name
    // ("alice", "alice:host") is completed to a full Matrix ID; only unparsable IDs are rejected.
    let new = |endpoint: &str, user: &str| {
        PasswordSignIn::new(
            root.0.clone(),
            AccountId::new(1),
            endpoint.into(),
            user.into(),
            Secret::new("fixture".into()).unwrap(),
            Secret::new("local unlock phrase".into()).unwrap(),
            None,
        )
    };
    assert!(new(" https://example.invalid ", "@alice:example.invalid").is_ok());
    for user in ["alice", "alice:example.invalid", " @alice:example.invalid"] {
        assert!(new("https://example.invalid", user).is_ok(), "{user}");
    }
    for user in ["@alice", ""] {
        assert!(matches!(new("https://example.invalid", user), Err(SignInError::InvalidUserId)), "{user}");
    }
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn unlock_rejects_relative_paths_short_phrases_and_invalid_trust_roots() {
    let root = PrivateRoot::new();
    assert!(matches!(
        UnlockSession::new(
            "relative".into(),
            AccountId::new(1),
            Secret::new("local unlock phrase".into()).unwrap(),
            None
        ),
        Err(SignInError::InvalidStorePath)
    ));
    assert!(matches!(
        UnlockSession::new(
            root.0.clone(),
            AccountId::new(1),
            Secret::new("è".repeat(15)).unwrap(),
            None
        ),
        Err(SignInError::InvalidSecret)
    ));
    for pem in [
        b"invalid CA fixture".to_vec(),
        vec![b'x'; 1048577],
        b"-----BEGIN CERTIFICATE-----\nMAA=\n-----END CERTIFICATE-----\n".to_vec(),
    ] {
        assert!(matches!(
            UnlockSession::new(
                root.0.clone(),
                AccountId::new(1),
                Secret::new("local unlock phrase".into()).unwrap(),
                Some(pem)
            ),
            Err(SignInError::InvalidTrustRoot)
        ));
    }
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn profile_lock_is_exclusive_and_released_without_deleting_state() {
    let root = PrivateRoot::new();
    let path = root.0.join("profile");
    let first = SignInProfile::open(path.clone()).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(matches!(
        SignInProfile::open(path.clone()),
        Err(SignInError::ProfileBusy)
    ));
    drop(first);
    assert!(SignInProfile::open(path.clone()).is_ok());
    assert!(path.join(".profile-lock").is_file());
}

#[test]
fn profile_never_repairs_unsafe_paths_or_file_permissions() {
    let root = PrivateRoot::new();
    let public = root.0.join("public");
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(&public)
        .unwrap();
    assert!(matches!(
        SignInProfile::open(public.clone()),
        Err(SignInError::InvalidStorePath)
    ));
    assert_eq!(
        std::fs::metadata(&public).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&root.0, &link).unwrap();
    assert!(matches!(
        SignInProfile::open(link),
        Err(SignInError::InvalidStorePath)
    ));
    let lock = root.0.join(".profile-lock");
    std::os::unix::fs::symlink(&public, &lock).unwrap();
    assert!(matches!(
        SignInProfile::open(root.0.clone()),
        Err(SignInError::Storage)
    ));
}

#[test]
fn profile_selector_requires_a_private_canonical_bounded_slot_number() {
    let root = PrivateRoot::new();
    let selector = root.0.join("active-account");
    for text in ["", "0", "129", "01", "1\n", "-1", "1/../../", "1"] {
        std::fs::write(&selector, text).unwrap();
        // The last case intentionally has unsafe permissions.
        std::fs::set_permissions(
            &selector,
            std::fs::Permissions::from_mode(if text == "1" { 0o644 } else { 0o600 }),
        )
        .unwrap();
        assert!(matches!(
            SignInProfile::open(root.0.clone()),
            Err(SignInError::Storage)
        ));
    }
    std::fs::set_permissions(&selector, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&selector, root.0.join("other-link")).unwrap();
    assert!(matches!(
        SignInProfile::open(root.0.clone()),
        Err(SignInError::Storage)
    ));
}

#[test]
fn profile_attempt_limit_fails_before_network_or_new_files() {
    let root = PrivateRoot::new();
    for id in 1..=128 {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.0.join(format!("account-{id}")))
            .unwrap();
    }
    let mut profile = SignInProfile::open(root.0.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(profile.password_login(
        "https://no-network.invalid".into(),
        "@alice:example.invalid".into(),
        Secret::new("fixture".into()).unwrap(),
        Secret::new("local unlock phrase".into()).unwrap(),
        None,
    ));
    assert!(matches!(result, Err(SignInError::AttemptLimit)));
    assert!(!root.0.join("account-129").exists());
}

#[test]
fn sealed_session_without_selector_is_not_treated_as_a_new_account() {
    let root = PrivateRoot::new();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(root.0.join("account-1"))
        .unwrap();
    std::fs::write(
        root.0.join("account-1/session.sealed"),
        "fixture-sealed-marker",
    )
    .unwrap();
    assert!(
        matches!(
            SignInProfile::open(root.0.clone()),
            Err(SignInError::Storage)
        ),
        "preserve an orphaned sealed session rather than creating another device"
    );
}

#[test]
fn interrupted_profile_publication_is_not_treated_as_a_new_account() {
    let root = PrivateRoot::new();
    std::fs::write(root.0.join("active-account.pending"), "1").unwrap();
    assert!(
        matches!(
            SignInProfile::open(root.0.clone()),
            Err(SignInError::Storage)
        ),
        "interrupted selector must block before another remote login"
    );
}

#[test]
fn bare_server_name_is_accepted_but_schemes_and_junk_are_not() {
    let root = PrivateRoot::new();
    let new = |endpoint: &str| {
        PasswordSignIn::new(
            root.0.clone(),
            AccountId::new(1),
            endpoint.into(),
            "@alice:example.invalid".into(),
            Secret::new("fixture".into()).unwrap(),
            Secret::new("local unlock phrase".into()).unwrap(),
            None,
        )
    };
    assert!(new("gnobbi.chat").is_ok());
    assert!(new("example.invalid:8448").is_ok());
    for bad in ["gnobbi.chat/path", "gnobbi chat", "", "ftp://x.invalid"] {
        assert!(matches!(new(bad), Err(SignInError::InvalidHomeserver)), "{bad}");
    }
}

fn orphans(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".orphan-"))
        .collect();
    names.sort();
    names
}

fn private_file(root: &std::path::Path, name: &str, text: &str) {
    std::fs::write(root.join(name), text).unwrap();
    std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn selector_of_a_missing_account_is_set_aside_and_the_profile_opens_empty() {
    let root = PrivateRoot::new();
    private_file(&root.0, "active-account", "1");
    private_file(&root.0, "protection-mode", "vault-v1");
    assert!(SignInProfile::open(root.0.clone()).unwrap().active_account().is_some());
    matrix_engine::signin::set_aside_orphans(&root.0);
    assert!(!root.0.join("active-account").exists());
    assert_eq!(orphans(&root.0).len(), 1, "set aside, not deleted");
    assert!(root.0.join("protection-mode").exists(), "a valid vault flag stays");
    assert!(SignInProfile::open(root.0.clone()).unwrap().active_account().is_none());
}

#[test]
fn damaged_selector_and_mode_without_any_store_are_set_aside() {
    for selector in ["", "0", "129", "01", "1\n", "x"] {
        let root = PrivateRoot::new();
        private_file(&root.0, "active-account", selector);
        private_file(&root.0, "protection-mode", "legacy");
        assert!(matches!(SignInProfile::open(root.0.clone()), Err(SignInError::Storage)));
        matrix_engine::signin::set_aside_orphans(&root.0);
        assert_eq!(orphans(&root.0).len(), 2, "{selector:?}");
        assert!(SignInProfile::open(root.0.clone()).is_ok());
    }
}

#[test]
fn set_aside_never_touches_a_store_a_sealed_session_or_a_pending_selector() {
    let root = PrivateRoot::new();
    std::fs::DirBuilder::new().mode(0o700).create(root.0.join("account-2")).unwrap();
    private_file(&root.0, "active-account", "1");
    private_file(&root.0, "protection-mode", "legacy");
    private_file(&root.0, "active-account.pending", "2");
    matrix_engine::signin::set_aside_orphans(&root.0);
    // id 1 has no folder, so its selector is an orphan; the damaged mode is kept because a store exists.
    let set_aside = orphans(&root.0);
    assert!(set_aside.len() == 1 && set_aside[0].starts_with("active-account.orphan-"), "{set_aside:?}");
    assert!(root.0.join("account-2").is_dir() && root.0.join("protection-mode").exists());
    assert!(root.0.join("active-account.pending").exists());
}
