//! Consuming sign-in/unlock requests for a backend runtime, never a UI thread.
//! No runtime, sync, recovery setup or remote writes beyond explicit login.
use crate::{
    auth::{AccountSession, AuthError},
    discovery::{Discovery, HomeserverUrl, LoginSupport},
    persistence::AccountStore,
};
use matrix_domain::AccountId;
use std::path::PathBuf;

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
pub struct Secret(zeroize::Zeroizing<String>);
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

/// Fixed local categories only: never SDK bodies, paths, URLs or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInError {
    InvalidHomeserver,
    InvalidUserId,
    InvalidSecret,
    InvalidTrustRoot,
    InvalidStorePath,
    Discovery,
    UnsupportedLogin,
    Storage,
    ProfileBusy,
    AlreadySignedIn,
    NoSession,
    AttemptLimit,
    Authentication(AuthError),
}

pub struct PasswordSignIn {
    root: PathBuf,
    account: AccountId,
    homeserver: HomeserverUrl,
    user: String,
    password: Secret,
    phrase: Secret,
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
        let homeserver =
            HomeserverUrl::parse(&homeserver).map_err(|_| SignInError::InvalidHomeserver)?;
        matrix_sdk::ruma::UserId::parse(&user).map_err(|_| SignInError::InvalidUserId)?;
        Ok(Self {
            root,
            account,
            homeserver,
            user,
            password,
            phrase,
            trust_root,
        })
    }

    pub async fn run(self) -> Result<AccountSession, SignInError> {
        let discovery = match self.trust_root.as_deref() {
            Some(pem) => Discovery::with_trusted_root(pem),
            None => Discovery::new(),
        }
        .map_err(|_| SignInError::Discovery)?;
        let result = discovery
            .probe_url(self.homeserver)
            .await
            .map_err(|_| SignInError::Discovery)?;
        if !result
            .login_flows
            .iter()
            .any(|flow| flow.support() == LoginSupport::SdkPasswordOnly)
        {
            return Err(SignInError::UnsupportedLogin);
        }
        let mut http = crate::auth::secure_http_builder();
        if let Some(pem) = self.trust_root.as_deref() {
            http = http.add_root_certificate(
                reqwest::Certificate::from_pem(pem).map_err(|_| SignInError::InvalidTrustRoot)?,
            );
        }
        let http = http.build().map_err(|_| SignInError::Discovery)?;
        AccountStore::create(&self.root, self.account)
            .map_err(|_| SignInError::Storage)?
            .password_login_with_http(
                result.homeserver,
                &self.user,
                &self.password.0,
                &self.phrase.0,
                http,
            )
            .await
            .map_err(SignInError::Authentication)
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
            AccountStore::open(&self.root, self.account).map_err(|_| SignInError::Storage)?;
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
    Unlock,
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
    Failed(SignInError),
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
}
pub struct SignInUi {
    pub events: crate::Receiver<SignInEvent>,
    pub commands: crate::Sender<SignInCommand>,
}
/// A pending backend operation, not a runtime or a thread. Run inside the
/// cancellable authentication future of RoomListWorker, never on the UI thread.
pub struct PendingSignIn {
    root: PathBuf,
    trust_root: Option<Vec<u8>>,
    events: crate::Sender<SignInEvent>,
    commands: crate::Receiver<SignInCommand>,
}
pub fn interactive(root: PathBuf, trust_root: Option<Vec<u8>>) -> (SignInUi, PendingSignIn) {
    let capacity = std::num::NonZeroUsize::new(1).unwrap();
    let (commands, requests) = crate::bounded(capacity);
    let (events, updates) = crate::bounded(capacity);
    (
        SignInUi {
            events: updates,
            commands,
        },
        PendingSignIn {
            root,
            trust_root,
            events,
            commands: requests,
        },
    )
}
impl PendingSignIn {
    pub async fn run(mut self) -> Result<AccountSession, SignInError> {
        let mut profile = match SignInProfile::open(self.root) {
            Ok(profile) => profile,
            Err(error) => {
                let _ = self.events.send(SignInEvent::Failed(error)).await;
                return Err(error);
            }
        };
        let mode = if profile.active_account().is_some() {
            SignInMode::Unlock
        } else {
            SignInMode::Password
        };
        self.events
            .send(SignInEvent::Prompt(SignInPrompt { mode, error: None }))
            .await
            .map_err(|_| SignInError::NoSession)?;
        while let Some(command) = self.commands.recv().await {
            let result = match command {
                SignInCommand::Password {
                    homeserver,
                    user,
                    password,
                    phrase,
                } if mode == SignInMode::Password => {
                    profile
                        .password_login(homeserver, user, password, phrase, self.trust_root.clone())
                        .await
                }
                SignInCommand::Unlock { phrase } if mode == SignInMode::Unlock => {
                    profile.unlock(phrase, self.trust_root.clone()).await
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
    _lock: std::fs::File,
}
impl SignInProfile {
    /// Backend-only filesystem work. The explicitly selected parent must exist.
    pub fn open(root: PathBuf) -> Result<Self, SignInError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if !root.is_absolute() {
            return Err(SignInError::InvalidStorePath);
        }
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&root)
                    .map_err(|_| SignInError::Storage)?;
            }
            Err(_) => return Err(SignInError::Storage),
            Ok(_) => (),
        }
        AccountStore::private_directory(&root).map_err(|_| SignInError::InvalidStorePath)?;
        let lock_path = root.join(".profile-lock");
        match std::fs::symlink_metadata(&lock_path) {
            Ok(_) => profile_regular_file(&lock_path, 0)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(SignInError::Storage),
        }
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)
            .map_err(|_| SignInError::Storage)?;
        lock.try_lock().map_err(|_| SignInError::ProfileBusy)?;
        match std::fs::symlink_metadata(root.join("active-account.pending")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(SignInError::Storage),
        }
        let active_path = root.join("active-account");
        let active = match std::fs::symlink_metadata(&active_path) {
            Ok(_) => {
                profile_regular_file(&active_path, 3)?;
                let text =
                    std::fs::read_to_string(active_path).map_err(|_| SignInError::Storage)?;
                let id: u64 = text.parse().map_err(|_| SignInError::Storage)?;
                if !(1..=128).contains(&id) || text != id.to_string() {
                    return Err(SignInError::Storage);
                }
                Some(AccountId::new(id))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(SignInError::Storage),
        };
        if active.is_none() {
            ensure_no_sealed_attempt(&root)?;
        }
        Ok(Self {
            root,
            active,
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
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if self.active.is_some() {
            return Err(SignInError::AlreadySignedIn);
        }
        match std::fs::symlink_metadata(self.root.join("active-account.pending")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(SignInError::Storage),
        }
        ensure_no_sealed_attempt(&self.root)?;
        let mut available = None;
        for id in 1..=128 {
            match std::fs::symlink_metadata(self.root.join(format!("account-{id}"))) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    available = Some(AccountId::new(id));
                    break;
                }
                Err(_) => return Err(SignInError::Storage),
                Ok(_) => (),
            }
        }
        let account = available.ok_or(SignInError::AttemptLimit)?;
        let session = PasswordSignIn::new(
            self.root.clone(),
            account,
            homeserver,
            user,
            password,
            phrase,
            trust_root,
        )?
        .run()
        .await?;
        // No secret in this small selector. Publish only after sealed login and
        // remote whoami, and never overwrite another profile's selection.
        let pending = self.root.join("active-account.pending");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&pending)
            .map_err(|_| SignInError::Storage)?;
        file.write_all(account.as_u64().to_string().as_bytes())
            .map_err(|_| SignInError::Storage)?;
        file.sync_all().map_err(|_| SignInError::Storage)?;
        std::fs::hard_link(&pending, self.root.join("active-account"))
            .map_err(|_| SignInError::Storage)?;
        std::fs::remove_file(pending).map_err(|_| SignInError::Storage)?;
        std::fs::File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| SignInError::Storage)?;
        self.active = Some(account);
        Ok(session)
    }

    pub async fn unlock(
        &mut self,
        phrase: Secret,
        trust_root: Option<Vec<u8>>,
    ) -> Result<AccountSession, SignInError> {
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

fn ensure_no_sealed_attempt(root: &std::path::Path) -> Result<(), SignInError> {
    for id in 1..=128 {
        match std::fs::symlink_metadata(root.join(format!("account-{id}/session.sealed"))) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(SignInError::Storage),
        }
    }
    Ok(())
}

fn profile_regular_file(path: &std::path::Path, max_len: u64) -> Result<(), SignInError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path).map_err(|_| SignInError::Storage)?;
    if !meta.is_file()
        || meta.permissions().mode() & 0o777 != 0o600
        || meta.nlink() != 1
        || meta.len() > max_len
    {
        return Err(SignInError::Storage);
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
