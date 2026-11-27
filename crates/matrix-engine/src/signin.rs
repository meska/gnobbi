//! Consuming sign-in/unlock requests for a backend runtime, never a UI thread.
//! No runtime, sync, recovery setup or remote writes beyond explicit login.
use crate::{
    auth::{AccountSession, AuthError},
    discovery::{Discovery, DiscoveryError, HomeserverUrl, LoginSupport},
    persistence::AccountStore,
    recovery::{RecoveryError, SetupError},
};
use matrix_domain::AccountId;
use std::path::PathBuf;

#[cfg(all(test, feature = "mac-vault", target_os = "macos"))]
#[path = "keychain_tests.rs"]
mod keychain_tests;
#[cfg(all(test, feature = "linux-vault", target_os = "linux"))]
#[path = "vault_tests.rs"]
mod vault_tests;

/// A bounded owned credential; deliberately no Debug/Display/Clone/serialization.
/// The owned buffer is zeroized on every return and cancellation path.
/// SDK-internal copies are outside this wrapper's zeroization guarantee.
///
/// ```compile_fail
/// let secret = matrix_engine::signin::Secret::new("fixture password".into()).unwrap();
/// println!("{secret:?}");
/// ```
/// ```compile_fail
/// let secret = matrix_engine::signin::Secret::new("fixture password".into()).unwrap();
/// let duplicate = secret.clone();
/// ```
pub struct Secret(pub(crate) zeroize::Zeroizing<String>);
impl zeroize::ZeroizeOnDrop for Secret {}
impl Secret {
    pub fn new(value: String) -> Result<Self, SignInError> {
        let value = zeroize::Zeroizing::new(value);
        if value.is_empty() || value.len() > 4096 {
            return Err(SignInError::InvalidSecret);
        }
        Ok(Self(value))
    }
}

/// Protection-secret failures from an OS vault; fixed categories, no details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    Unavailable,
    AlreadyExists,
    Missing,
    InvalidItem,
    Locked,
}

#[cfg(any(test, all(feature = "ios-vault", target_os = "ios"), all(feature = "android-vault", target_os = "android")))]
pub(crate) fn ios_profile_item(path: &std::path::Path) -> Option<String> {
    let worker = path.file_name()?.to_str()?.strip_prefix("account-")?.parse::<u64>().ok()?;
    let root = path.parent()?;
    let name = root.file_name()?.to_str()?;
    let profile = if name == "profile" {
        "profile".to_owned()
    } else if root.parent()?.file_name()? == "accounts" {
        let id = name.parse::<u64>().ok()?;
        if id.to_string() != name { return None; }
        format!("accounts/{id}")
    } else {
        return None;
    };
    Some(format!("profile-id:{profile}:account-{worker}"))
}

#[cfg(any(test, all(feature = "ios-vault", target_os = "ios")))]
pub(crate) fn ios_lookup_order<'a>(profile: &'a str, legacy: &'a str) -> [(&'a str, bool); 3] {
    [(profile, true), (legacy, true), (legacy, false)]
}

#[cfg(any(test, all(feature = "ios-vault", target_os = "ios")))]
pub(crate) fn ios_first_valid<T>(values: impl IntoIterator<Item = T>, mut valid: impl FnMut(&T) -> bool) -> Option<T> {
    values.into_iter().find(|value| valid(value))
}

/// Generated protection secret for one account store. Blocking: call from a backend worker.
pub trait ProtectionSource {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError>;
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError>;
    /// A new store must not overwrite an item left by an earlier install.
    fn available(&self, _store: &std::path::Path) -> Result<bool, VaultError> { Ok(true) }
}

/// Per-account OS vault keyed by the stable account ID (mobile; also the macOS account items).
/// Blocking: call from a backend worker. Errors are fixed categories, never item contents.
pub trait AccountProtectionSource: ProtectionSource {
    /// Generates a fresh secret, stores it for the account; `AlreadyExists` if one is there.
    fn create_account(&self, id: AccountId) -> Result<Secret, VaultError>;
    fn read_account(&self, id: AccountId) -> Result<Secret, VaultError>;
    /// Removes only this account's item; an absent item is fine.
    fn delete_account(&self, id: AccountId) -> Result<(), VaultError>;
    fn delete_profile(&self, id: AccountId, _store: &std::path::Path) -> Result<(), VaultError> {
        self.delete_account(id)
    }
}

/// The 32 random bytes as the 64-hex protection secret (same format as every desktop vault).
// Only the Apple vaults call these; the tests run everywhere.
#[allow(dead_code)]
pub(crate) fn secret_from_random(random: &[u8; 32]) -> Result<Secret, VaultError> {
    use std::fmt::Write;
    let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
    for byte in random {
        write!(&mut *text, "{byte:02x}").map_err(|_| VaultError::Unavailable)?;
    }
    Secret::new(std::mem::take(&mut *text)).map_err(|_| VaultError::Unavailable)
}

/// A stored item must be exactly 64 hex digits.
#[allow(dead_code)]
pub(crate) fn secret_from_item(value: &[u8]) -> Result<Secret, VaultError> {
    if value.len() != 64 || !value.iter().all(u8::is_ascii_hexdigit) {
        return Err(VaultError::InvalidItem);
    }
    let text = std::str::from_utf8(value).map_err(|_| VaultError::InvalidItem)?;
    Secret::new(text.to_owned()).map_err(|_| VaultError::InvalidItem)
}

/// A store held by another process is `ProfileBusy`, not a damaged store.
fn store_error(error: crate::persistence::StoreError) -> SignInError {
    if error == crate::persistence::StoreError::Busy {
        SignInError::ProfileBusy
    } else {
        SignInError::Storage
    }
}

/// Fixed local categories only: never SDK bodies, paths, URLs or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInError {
    InvalidHomeserver,
    InvalidUserId,
    InvalidSecret,
    InvalidTrustRoot,
    InvalidStorePath,
    Discovery,
    /// The server certificate was rejected (unknown issuer, wrong host, expired).
    Tls,
    /// The address answered with an HTTP error: not a Matrix endpoint (TLS was fine).
    ServerStatus,
    UnsupportedLogin,
    NoSlidingSync,
    Storage,
    ProfileBusy,
    AlreadySignedIn,
    NoSession,
    AttemptLimit,
    /// Profile protection mode does not match the requested operation.
    ProtectionMismatch,
    Vault(VaultError),
    Authentication(AuthError),
}

pub struct PasswordSignIn {
    root: PathBuf,
    account: AccountId,
    homeserver: HomeserverUrl,
    bare_name: bool,
    user: String,
    password: Secret,
    phrase: Option<Secret>,
    trust_root: Option<Vec<u8>>,
}
impl PasswordSignIn {
    /// Explicit HTTPS endpoint, full Matrix ID and separate local unlock phrase.
    /// Root must already exist with private permissions; never inspect a personal
    /// config or guess a store. Existing account slots are never overwritten.
    pub fn new(
        root: PathBuf,
        account: AccountId,
        homeserver: String,
        user: String,
        password: Secret,
        phrase: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<Self, SignInError> {
        validate_store(&root, &phrase, trust_root.as_deref())?;
        let homeserver = homeserver.trim().to_owned();
        // No scheme: a server name (delegation via .well-known); anything else must be a valid HTTPS URL.
        let bare_name = !homeserver.contains("://");
        if bare_name {
            matrix_sdk::ruma::ServerName::parse(&homeserver)
                .map_err(|_| SignInError::InvalidHomeserver)?;
        }
        let homeserver = HomeserverUrl::parse(&if bare_name {
            format!("https://{homeserver}")
        } else {
            homeserver
        })
        .map_err(|_| SignInError::InvalidHomeserver)?;
        let user = user.trim();
        let user = if user.contains(':') {
            if user.starts_with('@') { user.to_owned() } else { format!("@{user}") }
        } else if !user.starts_with('@') && !user.is_empty() {
            // shortcut: URL host may differ from Matrix server name; use a full ID in that case.
            let url = url::Url::parse(homeserver.as_str()).map_err(|_| SignInError::InvalidHomeserver)?;
            let host = url.host_str().ok_or(SignInError::InvalidHomeserver)?;
            let server = match url.port() { Some(port) => format!("{host}:{port}"), None => host.to_owned() };
            format!("@{user}:{server}")
        } else {
            user.to_owned()
        };
        matrix_sdk::ruma::UserId::parse(&user).map_err(|_| SignInError::InvalidUserId)?;
        Ok(Self {
            root,
            account,
            homeserver,
            bare_name,
            user,
            password,
            phrase: Some(phrase),
            trust_root,
        })
    }

    fn new_vault(
        root: PathBuf,
        account: AccountId,
        homeserver: String,
        user: String,
        password: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<Self, SignInError> {
        let mut login = Self::new(
            root,
            account,
            homeserver,
            user,
            password,
            Secret::new("x".repeat(16))?,
            trust_root,
        )?;
        login.phrase = None;
        Ok(login)
    }

    pub async fn run(self) -> Result<AccountSession, SignInError> {
        self.run_with(None).await
    }

    async fn run_with(
        self,
        source: Option<&dyn ProtectionSource>,
    ) -> Result<AccountSession, SignInError> {
        let discovery = match self.trust_root.as_deref() {
            Some(pem) => Discovery::with_trusted_root(pem),
            None => Discovery::new(),
        }
        .map_err(|_| SignInError::Discovery)?;
        let result = discovery
            .resolve(self.homeserver, self.bare_name)
            .await
            .map_err(|error| match error {
                DiscoveryError::HttpStatus(_) => SignInError::ServerStatus,
                DiscoveryError::Tls => SignInError::Tls,
                _ => SignInError::Discovery,
            })?;
        if !result
            .login_flows
            .iter()
            .any(|flow| flow.support() == LoginSupport::SdkPasswordOnly)
        {
            return Err(SignInError::UnsupportedLogin);
        }
        if !result.sliding_sync {
            return Err(SignInError::NoSlidingSync);
        }
        let mut http = crate::auth::secure_http_builder();
        if let Some(pem) = self.trust_root.as_deref() {
            let cert = reqwest::Certificate::from_pem(pem).map_err(|_| SignInError::InvalidTrustRoot)?;
            http = crate::auth::with_trust_roots(http, vec![cert]);
        }
        let http = http.build().map_err(|_| SignInError::Discovery)?;
        let store =
            AccountStore::create(&self.root, self.account).map_err(|_| SignInError::Storage)?;
        let dir = store.path.clone();
        let mut vault_secret = false;
        let result = async {
            let phrase = match (self.phrase, source) {
                (Some(phrase), None) => phrase,
                (None, Some(source)) => {
                    let phrase = source.create(&store).map_err(SignInError::Vault)?;
                    vault_secret = true;
                    phrase
                }
                _ => return Err(SignInError::ProtectionMismatch),
            };
            store
                .password_login_with_http(
                    result.homeserver,
                    &self.user,
                    &self.password.0,
                    &phrase.0,
                    http,
                )
                .await
                .map_err(SignInError::Authentication)
        }
        .await;
        // A refused attempt must leave no account-N directory and no vault item behind.
        if result.is_err() && !dir.join("session.sealed").exists() {
            if vault_secret {
                let _ = forget_secrets(self.account, std::slice::from_ref(&dir));
            }
            let _ = std::fs::remove_dir_all(&dir);
            let mut video = dir.file_name().unwrap_or_default().to_owned();
            video.push("-video");
            let _ = std::fs::remove_dir_all(dir.with_file_name(video));
        }
        result
    }
}

pub struct UnlockSession {
    root: PathBuf,
    account: AccountId,
    phrase: Secret,
    trust_root: Option<Vec<u8>>,
}
impl UnlockSession {
    pub fn new(
        root: PathBuf,
        account: AccountId,
        phrase: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<Self, SignInError> {
        validate_store(&root, &phrase, trust_root.as_deref())?;
        Ok(Self {
            root,
            account,
            phrase,
            trust_root,
        })
    }

    pub async fn run(self) -> Result<AccountSession, SignInError> {
        let store =
            AccountStore::open(&self.root, self.account).map_err(store_error)?;
        match self.trust_root.as_deref() {
            Some(pem) => store.restore_with_trusted_root(&self.phrase.0, pem).await,
            None => store.restore(&self.phrase.0).await,
        }
        .map_err(SignInError::Authentication)
    }
}

/// Payload-free status for an interactive frontend. No server bodies or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInMode {
    Password,
    /// Legacy profile protected by a local phrase.
    Unlock,
    /// Vault profile: the protection secret comes from the OS vault, no phrase.
    UnlockVault,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignInPrompt {
    pub mode: SignInMode,
    pub error: Option<SignInError>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInEvent {
    Prompt(SignInPrompt),
    Authenticated(AccountId),
    /// After a vault sign-in only: optional encrypted-history recovery.
    Recovery(RecoveryState),
    Failed(SignInError),
}
/// Payload-free post-login state; no key backup, backup creation or device trust happens here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryState {
    Offered,
    /// A recovery key is being checked and room keys downloaded.
    Pending,
    /// Keys were imported and downloaded; this does not prove every old message exists.
    Recovered,
    Failed(RecoveryFailure),
    Deferred,
    /// The account has no secret storage: the recovery key route does not exist for it.
    Unsupported,
}
impl RecoveryState {
    /// The state a finished recovery attempt ends in; shared by sign-in and the later «Recupera chiavi».
    pub fn of_outcome(outcome: Result<(), RecoveryError>) -> Self {
        match outcome {
            Ok(()) => Self::Recovered,
            Err(RecoveryError::NoSecretStorage) => Self::Unsupported,
            Err(RecoveryError::WrongKey | RecoveryError::InvalidSecret) => {
                Self::Failed(RecoveryFailure::InvalidKey)
            }
            Err(RecoveryError::Transport) => Self::Failed(RecoveryFailure::Network),
            Err(RecoveryError::Deadline) => Self::Failed(RecoveryFailure::Timeout),
            Err(RecoveryError::BackupUnavailable) => Self::Failed(RecoveryFailure::BackupUnavailable),
            Err(_) => Self::Failed(RecoveryFailure::Other),
        }
    }
}
/// Fixed failure categories of one recovery attempt; retry or defer stay possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryFailure {
    /// Malformed or wrong key.
    InvalidKey,
    Network,
    Timeout,
    BackupUnavailable,
    Other,
}
/// Outcome of a first-time recovery setup, on its own channel because the key must not be
/// `Debug`, `Clone` or `Copy`. Delivered once; nothing else carries it.
pub enum SetupEvent {
    Pending,
    Ready(crate::recovery::RecoverySecret),
    Failed(SetupFailure),
    /// The homeserver offers no key backup.
    Unsupported,
}
/// Fixed failure categories of one setup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupFailure {
    AlreadyExists,
    Partial,
    IdentityRequired,
    AuthRequired,
    Network,
    Timeout,
    Other,
}
pub enum SignInCommand {
    Password {
        homeserver: String,
        user: String,
        password: Secret,
        phrase: Secret,
    },
    Unlock {
        phrase: Secret,
    },
    /// New profile on a vault route: no local phrase, the protection secret lives in the OS vault.
    VaultPassword {
        homeserver: String,
        user: String,
        password: Secret,
    },
    /// Restore a vault profile; the secret is read from the OS vault.
    UnlockVault,
    /// An existing Matrix recovery key (or passphrase); dropped and zeroized after the attempt.
    EnterRecovery(Secret),
    /// Create secret storage and key backup; refused when either already exists.
    SetupRecovery,
    DeferRecovery,
}
pub struct SignInUi {
    pub events: crate::Receiver<SignInEvent>,
    pub setup: crate::Receiver<SetupEvent>,
    pub commands: crate::Sender<SignInCommand>,
}
/// A pending backend operation, not a runtime or a thread. Run inside the
/// cancellable authentication future of RoomListWorker, never on the UI thread.
pub struct PendingSignIn {
    root: PathBuf,
    trust_root: Option<Vec<u8>>,
    source: Option<std::sync::Arc<dyn ProtectionSource + Send + Sync>>,
    vault_timeout: std::time::Duration,
    password_login: bool,
    events: crate::Sender<SignInEvent>,
    setup: crate::Sender<SetupEvent>,
    commands: crate::Receiver<SignInCommand>,
}
pub fn interactive(root: PathBuf, trust_root: Option<Vec<u8>>) -> (SignInUi, PendingSignIn) {
    route(root, trust_root, None)
}
/// Vault route: new profiles need no local phrase.
pub fn interactive_with_source(
    root: PathBuf,
    trust_root: Option<Vec<u8>>,
    source: Box<dyn ProtectionSource + Send + Sync>,
) -> (SignInUi, PendingSignIn) {
    route(root, trust_root, Some(source))
}
/// The OS vault of this platform, if compiled in.
pub fn platform_source() -> Option<Box<dyn ProtectionSource + Send + Sync>> {
    #[cfg(all(feature = "linux-vault", target_os = "linux"))]
    return Some(Box::new(crate::vault::SecretService));
    #[cfg(all(feature = "mac-vault", target_os = "macos"))]
    return Some(Box::new(crate::keychain::MacKeychain::default()));
    #[cfg(all(feature = "ios-vault", target_os = "ios"))]
    return Some(Box::new(crate::keychain::IosKeychain));
    #[cfg(all(feature = "android-vault", target_os = "android"))]
    return Some(Box::new(crate::android_vault::AndroidKeystore));
    #[cfg(all(feature = "windows-vault", windows))]
    return Some(Box::new(crate::wincred::CredentialManager));
    #[allow(unreachable_code)]
    None
}
/// Deletes from the OS vault everything kept for one account: its stable-ID item and the
/// path-keyed item of each of its stores. Already absent items are fine; blocking.
pub fn forget_secrets(id: AccountId, stores: &[PathBuf]) -> Result<(), VaultError> {
    #[cfg(all(feature = "linux-vault", target_os = "linux"))]
    {
        crate::vault::delete_account_secret(id)?;
        stores
            .iter()
            .try_for_each(|s| crate::vault::delete_store_secret(s))
    }
    #[cfg(all(feature = "ios-vault", target_os = "ios"))]
    {
        let _ = id;
        let keychain = crate::keychain::AccountKeychain::new();
        stores.iter().try_for_each(|s| keychain.delete_profile_store(s))
    }
    #[cfg(all(feature = "android-vault", target_os = "android"))]
    {
        let _ = id;
        stores.iter().try_for_each(|s| crate::android_vault::delete_profile_store(s))
    }
    #[cfg(all(feature = "mac-vault", target_os = "macos"))]
    {
        let keychain = crate::keychain::AccountKeychain::new();
        keychain.delete(id)?;
        stores.iter().try_for_each(|s| keychain.delete_store(s))
    }
    #[cfg(all(feature = "windows-vault", windows))]
    {
        crate::wincred::delete_account_secret(id)?;
        stores
            .iter()
            .try_for_each(|s| crate::wincred::delete_store_secret(s))
    }
    #[cfg(not(any(
        all(feature = "linux-vault", target_os = "linux"),
        all(feature = "mac-vault", target_os = "macos"),
        all(feature = "ios-vault", target_os = "ios"),
        all(feature = "android-vault", target_os = "android"),
        all(feature = "windows-vault", windows)
    )))]
    {
        let _ = (id, stores);
        Ok(())
    }
}
fn route(
    root: PathBuf,
    trust_root: Option<Vec<u8>>,
    source: Option<Box<dyn ProtectionSource + Send + Sync>>,
) -> (SignInUi, PendingSignIn) {
    let capacity = std::num::NonZeroUsize::new(1).unwrap();
    let (commands, requests) = crate::bounded(capacity);
    let (events, updates) = crate::bounded(capacity);
    let (setup_events, setup) = crate::bounded(capacity);
    (
        SignInUi {
            events: updates,
            setup,
            commands,
        },
        PendingSignIn {
            root,
            trust_root,
            source: source.map(Into::into),
            vault_timeout: VAULT_READ_TIMEOUT,
            password_login: false,
            events,
            setup: setup_events,
            commands: requests,
        },
    )
}
/// Upper bound for one OS-vault read; a locked or absent keyring must not stall start-up.
#[cfg(not(target_os = "ios"))]
const VAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
// Migrating ambiguous legacy items may require several expensive authenticated decryptions.
#[cfg(target_os = "ios")]
const VAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
impl PendingSignIn {
    /// Ask for credentials while keeping the old sealed profile intact until login succeeds.
    pub fn with_password_login(mut self) -> Self {
        self.password_login = true;
        self
    }
    pub fn with_vault_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.vault_timeout = timeout;
        self
    }
    /// Authenticated vault session: offer recovery, hand the session over once deferred.
    /// Never touches key backup or device trust.
    async fn recovery_step(
        &mut self,
        mut session: AccountSession,
    ) -> Result<AccountSession, SignInError> {
        let closed = |_| SignInError::NoSession;
        self.events
            .send(SignInEvent::Authenticated(session.account_id()))
            .await
            .map_err(closed)?;
        self.events
            .send(SignInEvent::Recovery(RecoveryState::Offered))
            .await
            .map_err(closed)?;
        while let Some(command) = self.commands.recv().await {
            match command {
                SignInCommand::DeferRecovery => {
                    self.events
                        .send(SignInEvent::Recovery(RecoveryState::Deferred))
                        .await
                        .map_err(closed)?;
                    return Ok(session);
                }
                SignInCommand::EnterRecovery(key) => {
                    self.events
                        .send(SignInEvent::Recovery(RecoveryState::Pending))
                        .await
                        .map_err(closed)?;
                    // The key lives only in this call; dropping the future zeroizes it.
                    let outcome = session.recover_all_room_keys(&key.0).await;
                    drop(key);
                    let state = RecoveryState::of_outcome(outcome);
                    self.events
                        .send(SignInEvent::Recovery(state))
                        .await
                        .map_err(closed)?;
                    if state == RecoveryState::Recovered {
                        return Ok(session);
                    }
                }
                SignInCommand::SetupRecovery => {
                    self.setup
                        .send(SetupEvent::Pending)
                        .await
                        .map_err(|_| SignInError::NoSession)?;
                    let event = match session.setup_recovery().await {
                        Ok(key) => SetupEvent::Ready(key),
                        Err(SetupError::Unsupported) => SetupEvent::Unsupported,
                        Err(error) => SetupEvent::Failed(match error {
                            SetupError::AlreadyExists => SetupFailure::AlreadyExists,
                            SetupError::Partial => SetupFailure::Partial,
                            SetupError::IdentityRequired => SetupFailure::IdentityRequired,
                            SetupError::AuthRequired => SetupFailure::AuthRequired,
                            SetupError::Transport => SetupFailure::Network,
                            SetupError::Deadline => SetupFailure::Timeout,
                            _ => SetupFailure::Other,
                        }),
                    };
                    self.setup
                        .send(event)
                        .await
                        .map_err(|_| SignInError::NoSession)?;
                }
                _ => (),
            }
        }
        Err(SignInError::NoSession)
    }

    pub async fn run(mut self) -> Result<AccountSession, SignInError> {
        set_aside_orphans(&self.root);
        let mut profile = match SignInProfile::open(self.root.clone()) {
            Ok(profile) => profile,
            Err(error) => {
                let _ = self.events.send(SignInEvent::Failed(error)).await;
                return Err(error);
            }
        };
        let mode = if self.password_login || profile.active_account().is_none() {
            SignInMode::Password
        } else if profile.vault_mode && self.source.is_some() {
            SignInMode::UnlockVault
        } else {
            SignInMode::Unlock
        };
        // Vault profile: read the key by itself; the manual prompt only appears if that fails.
        let mut first_error = None;
        if mode == SignInMode::UnlockVault {
            let source = self.source.clone().unwrap();
            match profile
                .unlock_vault_timed(source, self.trust_root.clone(), self.vault_timeout)
                .await
            {
                Ok(session) => {
                    self.events
                        .send(SignInEvent::Authenticated(session.account_id()))
                        .await
                        .map_err(|_| SignInError::NoSession)?;
                    return Ok(session);
                }
                Err(error) => first_error = Some(error),
            }
        }
        self.events
            .send(SignInEvent::Prompt(SignInPrompt {
                mode,
                error: first_error,
            }))
            .await
            .map_err(|_| SignInError::NoSession)?;
        while let Some(command) = self.commands.recv().await {
            let result = match command {
                SignInCommand::Password {
                    homeserver,
                    user,
                    password,
                    phrase,
                } if mode == SignInMode::Password && self.source.is_none() => {
                    profile
                        .password_login(homeserver, user, password, phrase, self.trust_root.clone())
                        .await
                }
                SignInCommand::Unlock { phrase } if mode == SignInMode::Unlock => {
                    profile.unlock(phrase, self.trust_root.clone()).await
                }
                SignInCommand::VaultPassword {
                    homeserver,
                    user,
                    password,
                } if mode == SignInMode::Password && self.source.is_some() => {
                    let source = self.source.as_deref().unwrap();
                    let session = if self.password_login {
                        profile.replace_vault_login(source, homeserver, user, password, self.trust_root.clone()).await
                    } else {
                        profile.password_login_vault(source, homeserver, user, password, self.trust_root.clone()).await
                    };
                    if let Ok(session) = session {
                        return self.recovery_step(session).await;
                    }
                    session
                }
                SignInCommand::UnlockVault
                    if matches!(mode, SignInMode::Unlock | SignInMode::UnlockVault) =>
                {
                    match self.source.clone() {
                        Some(source) => {
                            profile
                                .unlock_vault_timed(
                                    source,
                                    self.trust_root.clone(),
                                    self.vault_timeout,
                                )
                                .await
                        }
                        None => Err(SignInError::ProtectionMismatch),
                    }
                }
                SignInCommand::Password { .. } if mode == SignInMode::Password => {
                    Err(SignInError::ProtectionMismatch)
                }
                _ => Err(SignInError::NoSession),
            };
            match result {
                Ok(session) => {
                    self.events
                        .send(SignInEvent::Authenticated(session.account_id()))
                        .await
                        .map_err(|_| SignInError::NoSession)?;
                    return Ok(session);
                }
                Err(error) => self
                    .events
                    .send(SignInEvent::Prompt(SignInPrompt {
                        mode,
                        error: Some(error),
                    }))
                    .await
                    .map_err(|_| SignInError::NoSession)?,
            }
        }
        Err(SignInError::NoSession)
    }
}

/// Single-account GUI profile. Holds a local exclusive lock while choosing or
/// unlocking a session; the AccountSession subsequently holds its own store lock.
/// Failed attempts are retained in separate bounded slots, never reused/deleted.
pub struct SignInProfile {
    root: PathBuf,
    active: Option<AccountId>,
    vault_mode: bool,
    /// Mobile: the one account ID this profile root may hold (the ID is global, so the vault
    /// item key never depends on the root). None = first free `account-N`.
    fixed: Option<AccountId>,
    _lock: std::fs::File,
}
impl SignInProfile {
    /// Like `open`, but a first sign-in creates `account-<account>` (1..=128) instead of the first free one.
    pub fn open_as(root: PathBuf, account: AccountId) -> Result<Self, SignInError> {
        if !(1..=128).contains(&account.as_u64()) {
            return Err(SignInError::AttemptLimit);
        }
        let mut profile = Self::open(root)?;
        profile.fixed = Some(account);
        Ok(profile)
    }

    /// Backend-only filesystem work. The explicitly selected parent must exist.
    pub fn open(root: PathBuf) -> Result<Self, SignInError> {
        if !root.is_absolute() {
            return Err(fail_path(line!()));
        }
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::fsx::create_private_dir(&root)
                    .map_err(|_| fail(line!()))?;
            }
            Err(_) => return Err(fail(line!())),
            Ok(_) => (),
        }
        AccountStore::private_directory(&root).map_err(|_| fail_path(line!()))?;
        let lock_path = root.join(".profile-lock");
        match std::fs::symlink_metadata(&lock_path) {
            Ok(_) => profile_regular_file(&lock_path, 0)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(fail(line!())),
        }
        let lock = crate::fsx::private_file(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false),
        )
        .open(lock_path)
            .map_err(|_| fail(line!()))?;
        lock.try_lock().map_err(|_| SignInError::ProfileBusy)?;
        match std::fs::symlink_metadata(root.join("active-account.pending")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(fail(line!())),
        }
        let active_path = root.join("active-account");
        let active = match std::fs::symlink_metadata(&active_path) {
            Ok(_) => {
                profile_regular_file(&active_path, 3)?;
                let text =
                    std::fs::read_to_string(active_path).map_err(|_| fail(line!()))?;
                let id: u64 = text.parse().map_err(|_| fail(line!()))?;
                if !(1..=128).contains(&id) || text != id.to_string() {
                    return Err(fail(line!()));
                }
                Some(AccountId::new(id))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(fail(line!())),
        };
        if active.is_none() {
            ensure_no_sealed_attempt(&root)?;
        }
        // Non-secret binding; absent means the legacy phrase profile, anything else is damage.
        let mode_path = root.join("protection-mode");
        let vault_mode = match std::fs::symlink_metadata(&mode_path) {
            Ok(_) => {
                profile_regular_file(&mode_path, 8)?;
                if std::fs::read(&mode_path).map_err(|_| fail(line!()))? != b"vault-v1" {
                    return Err(fail(line!()));
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(fail(line!())),
        };
        Ok(Self {
            root,
            active,
            vault_mode,
            fixed: None,
            _lock: lock,
        })
    }

    pub fn active_account(&self) -> Option<AccountId> {
        self.active
    }

    pub async fn password_login(
        &mut self,
        homeserver: String,
        user: String,
        password: Secret,
        phrase: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
        if self.vault_mode {
            return Err(SignInError::ProtectionMismatch);
        }
        self.login(None, homeserver, user, password, Some(phrase), trust_root, false)
            .await
    }

    /// New vault-backed profile: the protection secret is generated and kept only in the OS vault.
    pub async fn password_login_vault(
        &mut self,
        source: &dyn ProtectionSource,
        homeserver: String,
        user: String,
        password: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
        self.login(Some(source), homeserver, user, password, None, trust_root, false)
            .await
    }

    async fn replace_vault_login(
        &mut self,
        source: &dyn ProtectionSource,
        homeserver: String,
        user: String,
        password: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
        self.login(Some(source), homeserver, user, password, None, trust_root, true).await
    }

    #[allow(clippy::too_many_arguments)] // private; the three public entry points differ only in these fixed inputs
    async fn login(
        &mut self,
        source: Option<&dyn ProtectionSource>,
        homeserver: String,
        user: String,
        password: Secret,
        phrase: Option<Secret>,
        trust_root: Option<Vec<u8>>,
        replace: bool,
    ) -> Result<AccountSession, SignInError> {
        use std::io::Write;
        if self.active.is_some() && !replace {
            return Err(SignInError::AlreadySignedIn);
        }
        match std::fs::symlink_metadata(self.root.join("active-account.pending")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(SignInError::Storage),
        }
        if !replace || self.active.is_none() {
            ensure_no_sealed_attempt(&self.root)?;
        }
        let account = pick_account(&self.root, self.fixed, source)?;
        let login = match phrase {
            Some(phrase) => PasswordSignIn::new(
                self.root.clone(),
                account,
                homeserver,
                user,
                password,
                phrase,
                trust_root,
            )?,
            None => PasswordSignIn::new_vault(
                self.root.clone(),
                account,
                homeserver,
                user,
                password,
                trust_root,
            )?,
        };
        let session = login.run_with(source).await?;
        if source.is_some() && !self.vault_mode {
            let mode = self.root.join("protection-mode");
            let mut file = crate::fsx::private_file(std::fs::OpenOptions::new().write(true).create_new(true))
                .open(&mode)
                .map_err(|_| SignInError::Storage)?;
            file.write_all(b"vault-v1")
                .map_err(|_| SignInError::Storage)?;
            file.sync_all().map_err(|_| SignInError::Storage)?;
            self.vault_mode = true;
        }
        // No secret in this small selector. Publish only after sealed login and
        // remote whoami, and never overwrite another profile's selection.
        let pending = self.root.join("active-account.pending");
        let mut file = crate::fsx::private_file(std::fs::OpenOptions::new().write(true).create_new(true))
            .open(&pending)
            .map_err(|_| SignInError::Storage)?;
        file.write_all(account.as_u64().to_string().as_bytes())
            .map_err(|_| SignInError::Storage)?;
        file.sync_all().map_err(|_| SignInError::Storage)?;
        if replace && self.active.is_some() {
            std::fs::rename(&pending, self.root.join("active-account")).map_err(|_| SignInError::Storage)?;
        } else {
            crate::fsx::publish_new(&pending, &self.root.join("active-account"))
                .map_err(|_| SignInError::Storage)?;
            crate::fsx::remove_pending(&pending).map_err(|_| SignInError::Storage)?;
        }
        crate::fsx::sync_dir(&self.root).map_err(|_| SignInError::Storage)?;
        self.active = Some(account);
        Ok(session)
    }

    pub async fn unlock_vault(
        &mut self,
        source: &dyn ProtectionSource,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
        if !self.vault_mode {
            return Err(SignInError::ProtectionMismatch);
        }
        let account = self.active.ok_or(SignInError::NoSession)?;
        let store = AccountStore::open(&self.root, account).map_err(|_| SignInError::Storage)?;
        let phrase = source.read(&store).map_err(SignInError::Vault)?;
        drop(store);
        UnlockSession::new(self.root.clone(), account, phrase, trust_root)?
            .run()
            .await
    }

    /// Like `unlock_vault`, but the blocking vault read runs on a blocking thread and is
    /// abandoned after `timeout` (reported as `Vault(Unavailable)`).
    pub async fn unlock_vault_timed(
        &mut self,
        source: std::sync::Arc<dyn ProtectionSource + Send + Sync>,
        trust_root: Option<Vec<u8>>,
        timeout: std::time::Duration,
    ) -> Result<AccountSession, SignInError> {
        if !self.vault_mode {
            return Err(SignInError::ProtectionMismatch);
        }
        let account = self.active.ok_or(SignInError::NoSession)?;
        let root = self.root.clone();
        let read = tokio::task::spawn_blocking(move || {
            let store = AccountStore::open(&root, account).map_err(store_error)?;
            source.read(&store).map_err(SignInError::Vault)
        });
        let phrase = match tokio::time::timeout(timeout, read).await {
            Ok(Ok(phrase)) => phrase?,
            _ => return Err(SignInError::Vault(VaultError::Unavailable)),
        };
        UnlockSession::new(self.root.clone(), account, phrase, trust_root)?
            .run()
            .await
    }

    pub async fn unlock(
        &mut self,
        phrase: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
        if self.vault_mode {
            return Err(SignInError::ProtectionMismatch);
        }
        UnlockSession::new(
            self.root.clone(),
            self.active.ok_or(SignInError::NoSession)?,
            phrase,
            trust_root,
        )?
        .run()
        .await
    }
}

/// A selector that points to no store (leftover of an app-container reset: the App Group outlives it) blocks every
/// start with «profile unavailable». Provable orphans are renamed `*.orphan-<ts>`, never deleted; a store, a sealed
/// session or a pending selector is never touched. Failures are only logged: `open` then reports what it finds.
pub fn set_aside_orphans(root: &std::path::Path) {
    let exists = |name: String| std::fs::symlink_metadata(root.join(name)).is_ok();
    let stores = (1..=128).any(|id| exists(format!("account-{id}")));
    let selector = root.join("active-account");
    let orphan_selector = match std::fs::symlink_metadata(&selector) {
        Ok(meta) if meta.is_file() => match std::fs::read_to_string(&selector).ok().and_then(|t| t.parse::<u64>().ok()) {
            Some(id) if (1..=128).contains(&id) => !exists(format!("account-{id}")),
            _ => !stores,
        },
        _ => false,
    };
    let mode = root.join("protection-mode");
    let orphan_mode = !stores
        && std::fs::symlink_metadata(&mode).is_ok()
        && (profile_regular_file(&mode, 8).is_err() || std::fs::read(&mode).ok().as_deref() != Some(b"vault-v1"));
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    for (orphan, path) in [(orphan_selector, selector), (orphan_mode, mode)] {
        if !orphan {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let target = (0..).map(|n| root.join(format!("{name}.orphan-{ts}{}", if n == 0 { String::new() } else { format!("-{n}") }))).find(|t| std::fs::symlink_metadata(t).is_err()).unwrap();
        match std::fs::rename(&path, &target) {
            Ok(()) => eprintln!("Gnobbi: set aside orphan {name}"),
            Err(_) => eprintln!("Gnobbi: could not set aside orphan {name}"),
        }
    }
}

fn ensure_no_sealed_attempt(root: &std::path::Path) -> Result<(), SignInError> {
    for id in 1..=128 {
        match std::fs::symlink_metadata(root.join(format!("account-{id}/session.sealed"))) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(SignInError::Storage),
        }
    }
    Ok(())
}

/// The failing check goes to the log (no path, no secret): the screen only says "local profile unavailable".
fn fail(line: u32) -> SignInError {
    eprintln!("Gnobbi: profile open failed (signin.rs:{line}, Storage)");
    SignInError::Storage
}
fn fail_path(line: u32) -> SignInError {
    eprintln!("Gnobbi: profile open failed (signin.rs:{line}, InvalidStorePath)");
    SignInError::InvalidStorePath
}

fn profile_regular_file(path: &std::path::Path, max_len: u64) -> Result<(), SignInError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| fail(line!()))?;
    if !meta.is_file()
        || !crate::fsx::mode_is(&meta, 0o600)
        || !crate::fsx::single_link(&meta)
        || meta.len() > max_len
    {
        return Err(fail(line!()));
    }
    Ok(())
}

fn validate_store(
    root: &std::path::Path,
    phrase: &Secret,
    pem: Option<&[u8]>,
) -> Result<(), SignInError> {
    if !root.is_absolute() {
        return Err(SignInError::InvalidStorePath);
    }
    if phrase.0.chars().count() < 16 {
        return Err(SignInError::InvalidSecret);
    }
    if let Some(pem) = pem
        && Discovery::with_trusted_root(pem).is_err()
    {
        return Err(SignInError::InvalidTrustRoot);
    }
    Ok(())
}

/// First `account-N` with neither a folder nor a vault item (an item outlives an uninstall).
fn pick_account(
    root: &std::path::Path,
    fixed: Option<AccountId>,
    source: Option<&dyn ProtectionSource>,
) -> Result<AccountId, SignInError> {
    for id in fixed.map_or(1..=128, |a| a.as_u64()..=a.as_u64()) {
        let path = root.join(format!("account-{id}"));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(source) = source
                    && !source.available(&path).map_err(SignInError::Vault)?
                {
                    continue;
                }
                return Ok(AccountId::new(id));
            }
            Err(_) => return Err(SignInError::Storage),
            Ok(_) => (),
        }
    }
    Err(SignInError::AttemptLimit)
}

#[cfg(test)]
mod pick_account_tests {
    use super::*;

    /// Reports an item already present for the listed ids.
    struct Orphans(Vec<u64>);
    impl ProtectionSource for Orphans {
        fn create(&self, _: &AccountStore) -> Result<Secret, VaultError> {
            unreachable!("picking an account never creates")
        }
        fn read(&self, _: &AccountStore) -> Result<Secret, VaultError> {
            unreachable!()
        }
        fn available(&self, path: &std::path::Path) -> Result<bool, VaultError> {
            Ok(!self.0.iter().any(|id| path.ends_with(format!("account-{id}"))))
        }
    }

    #[test]
    fn an_orphan_vault_item_makes_the_id_unavailable() {
        let root = std::path::Path::new("/nonexistent/gnobbi-pick-account");
        let pick = |orphans: Vec<u64>, fixed| pick_account(root, fixed, Some(&Orphans(orphans)));
        assert_eq!(pick(vec![], None), Ok(AccountId::new(1)));
        assert_eq!(pick(vec![1], None), Ok(AccountId::new(2)));
        assert_eq!(pick(vec![1, 2], None), Ok(AccountId::new(3)));
        assert_eq!(pick((1..=128).collect(), None), Err(SignInError::AttemptLimit));
        // shortcut: a fixed id with an orphan reports AttemptLimit; no caller fixes an id yet, give it its own error when one does.
        assert_eq!(pick(vec![5], Some(AccountId::new(5))), Err(SignInError::AttemptLimit));
    }
}

#[cfg(test)]
mod vault_format_tests {
    use super::*;

    #[test]
    fn ios_lookup_order_is_profile_then_shared_and_default_legacy() {
        assert_eq!(ios_lookup_order("profile", "legacy"), [("profile", true), ("legacy", true), ("legacy", false)]);
    }

    #[test]
    fn ios_profile_items_are_isolated_even_with_the_same_worker_id() {
        let a = ios_profile_item(std::path::Path::new("/old/container/accounts/2/account-1")).unwrap();
        let b = ios_profile_item(std::path::Path::new("/new/container/accounts/3/account-1")).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, ios_profile_item(std::path::Path::new("/new/container/accounts/2/account-1")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn ios_legacy_is_assigned_only_to_the_store_it_decrypts() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("gnobbi-ios-legacy-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("accounts/2")).unwrap();
        std::fs::create_dir_all(root.join("accounts/3")).unwrap();
        for path in [root.clone(), root.join("accounts"), root.join("accounts/2"), root.join("accounts/3")] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let first = AccountStore::create(&root.join("accounts/2"), AccountId::new(1)).unwrap();
        let second = AccountStore::create(&root.join("accounts/3"), AccountId::new(1)).unwrap();
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        first.seal(a, &serde_json::json!({"test": 1})).unwrap();
        second.seal(b, &serde_json::json!({"test": 2})).unwrap();
        assert!(first.unseal(b).is_err());
        assert!(second.unseal(b).is_ok());
        assert_eq!(ios_first_valid([a, b], |candidate| second.unseal(candidate).is_ok()), Some(b));
        assert_eq!(ios_first_valid([b], |candidate| first.unseal(candidate).is_ok()), None);
        let mut names = std::collections::HashMap::new();
        names.insert(ios_profile_item(&first.path).unwrap(), a);
        names.insert(ios_profile_item(&second.path).unwrap(), b);
        names.remove(&ios_profile_item(&first.path).unwrap());
        assert_eq!(names.get(&ios_profile_item(&second.path).unwrap()), Some(&b));
        drop((first, second));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn random_bytes_become_64_lowercase_hex() {
        let mut random = [0u8; 32];
        random[0] = 0xab;
        random[31] = 0x01;
        let secret = secret_from_random(&random).unwrap();
        assert_eq!(&*secret.0, &format!("ab{}01", "00".repeat(30)));
    }

    #[test]
    fn stored_item_must_be_exactly_64_hex() {
        let good = "0123456789abcdefABCDEF".repeat(3)[..64].to_owned();
        assert!(secret_from_item(good.as_bytes()).is_ok());
        for bad in [&good[..63], &format!("{good}0"), &format!("{}g", &good[..63]), ""] {
            assert_eq!(secret_from_item(bad.as_bytes()).err(), Some(VaultError::InvalidItem));
        }
        assert_eq!(secret_from_item(&[0xff; 64]).err(), Some(VaultError::InvalidItem));
    }
}

#[cfg(test)]
mod homeserver_input_tests {
    use super::*;

    fn homeserver_of(typed: &str) -> Result<String, SignInError> {
        PasswordSignIn::new(
            "/tmp/gnobbi-homeserver-input".into(),
            AccountId::new(1),
            typed.into(),
            "@alice:example.org".into(),
            Secret::new("password".into())?,
            Secret::new("x".repeat(16))?,
            None,
        )
        .map(|login| login.homeserver.as_str().to_owned())
    }

    #[test]
    fn signin_accepts_localpart_and_full_ids() {
        let login = |user: &str| PasswordSignIn::new(
            "/tmp/gnobbi-homeserver-input".into(), AccountId::new(1),
            "https://example.org".into(), user.into(), Secret::new("password".into()).unwrap(),
            Secret::new("x".repeat(16)).unwrap(), None,
        ).map(|login| login.user);
        assert_eq!(login("marco").unwrap(), "@marco:example.org");
        assert_eq!(login("marco:example.org").unwrap(), "@marco:example.org");
        assert_eq!(login("@marco:example.org").unwrap(), "@marco:example.org");
        assert_eq!(login("  @marco:other.example  ").unwrap(), "@marco:other.example");
        assert!(matches!(login("@marco"), Err(SignInError::InvalidUserId)));
        assert!(matches!(login(""), Err(SignInError::InvalidUserId)));
    }

    #[test]
    fn scheme_case_and_surrounding_spaces_do_not_matter() {
        for typed in [
            "https://matrix.example.org",
            "Https://matrix.example.org",
            "HTTPS://matrix.example.org",
            "  Https://matrix.example.org \n",
        ] {
            assert_eq!(homeserver_of(typed).unwrap(), "https://matrix.example.org/", "{typed:?}");
        }
        assert_eq!(homeserver_of(" matrix.example.org ").unwrap(), "https://matrix.example.org/");
        assert!(matches!(homeserver_of("http://matrix.example.org"), Err(SignInError::InvalidHomeserver)));
        assert!(matches!(homeserver_of("Https://"), Err(SignInError::InvalidHomeserver)));
    }
}
