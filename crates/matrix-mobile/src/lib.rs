// Same reason as matrix-engine: the worker future is generic over the login future and nests deeply.
#![recursion_limit = "256"]
//! UniFFI bridge for the mobile apps (Swift/Kotlin). A thin layer over `matrix-engine`: sign-in,
//! session restore, the room-list projection with its sync status, and plain-text sends.
//!
//! The engine worker owns the SDK client and its own runtime thread; this crate never touches the SDK.
//! The protection secret of an account store lives in the platform keychain/keystore: on iOS the
//! engine's `ios-vault` Keychain (pass `None` as `secrets`); anywhere else only through the
//! [`SecretStore`] callback the app implements (no desktop vault is compiled in).
//!
//! Several accounts live side by side in one `data_dir`, each isolated from the others:
//! * store and profile: `<data_dir>/accounts/<id>/` (its own profile lock and `account-<id>` store);
//! * vault item: keyed by the same `<id>` (1..=128, allocated at sign-in, free again after logout);
//! * runtime: one [`MobileSession`] (engine worker, thread, listener) per signed-in account.
//!
//! Flow: [`list_saved_accounts`] → [`restore_session`] for each one to reopen (no password), or
//! [`sign_in`] to add one; [`MobileSession::logout`] revokes and forgets exactly that account.
//! No call logs or returns a password, token or protection secret.
//! Updates are pushed through [`SyncListener`], always from one dedicated thread.
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    Receiver, RoomListDiff, RoomListEvent, RoomSyncStatus, Sender,
    auth::{AccountSession, AuthError},
    persistence::{AccountStore, StoreError},
    room_sync::RoomListWorker,
    sending::{SendError, SendEvent, SendRequest, fresh_transaction_id},
    signin::{
        AccountProtectionSource, ProtectionSource, Secret, SignInError, SignInProfile, VaultError,
    },
    WorkerControl,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

uniffi::setup_scaffolding!();

/// Fixed categories only: no server text, paths or credentials cross the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum MobileError {
    InvalidInput,
    InvalidHomeserver,
    InvalidUserId,
    Network,
    /// The server lacks password login or sliding sync.
    Unsupported,
    /// Wrong credentials or a session the server no longer accepts.
    Rejected,
    Storage,
    /// The platform keychain/keystore callback failed.
    SecretStore,
    Busy,
    AlreadySignedIn,
    /// Nothing to restore: sign in first.
    NoSession,
    Encryption,
    /// The session has stopped.
    Closed,
}
impl std::fmt::Display for MobileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for MobileError {}

impl From<SignInError> for MobileError {
    fn from(error: SignInError) -> Self {
        match error {
            SignInError::InvalidHomeserver => Self::InvalidHomeserver,
            SignInError::InvalidUserId => Self::InvalidUserId,
            SignInError::InvalidSecret | SignInError::InvalidTrustRoot => Self::InvalidInput,
            SignInError::Discovery => Self::Network,
            SignInError::ServerStatus => Self::InvalidHomeserver,
            SignInError::UnsupportedLogin | SignInError::NoSlidingSync => Self::Unsupported,
            SignInError::InvalidStorePath
            | SignInError::Storage
            | SignInError::AttemptLimit
            | SignInError::ProtectionMismatch => Self::Storage,
            SignInError::ProfileBusy => Self::Busy,
            SignInError::AlreadySignedIn => Self::AlreadySignedIn,
            SignInError::NoSession => Self::NoSession,
            SignInError::Vault(_) => Self::SecretStore,
            SignInError::Authentication(AuthError::Transport) => Self::Network,
            SignInError::Authentication(AuthError::InvalidUserId) => Self::InvalidUserId,
            SignInError::Authentication(AuthError::Storage) => Self::Storage,
            SignInError::Authentication(_) => Self::Rejected,
        }
    }
}
impl From<SendError> for MobileError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Encryption => Self::Encryption,
            SendError::Transport | SendError::Unavailable => Self::Network,
            SendError::WrongScope | SendError::InvalidInput | SendError::TooLarge => {
                Self::InvalidInput
            }
        }
    }
}

/// Where the protection secret of an account store is kept when the engine has no vault of its
/// own for the platform (Android Keystore). `account` is the account ID of this crate, unique in
/// the `data_dir`. The app generates the secret (at least 16 random characters) on `create`.
/// Calls are blocking and come from a background thread, never the main one. On iOS pass `None`
/// instead of a `SecretStore`: the engine uses the Keychain directly.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn create(&self, account: u64) -> Result<String, MobileError>;
    fn read(&self, account: u64) -> Result<String, MobileError>;
    /// Removes only this account's secret; an absent one is not an error.
    fn delete(&self, account: u64) -> Result<(), MobileError>;
}
struct ForeignSource(Arc<dyn SecretStore>);
impl ProtectionSource for ForeignSource {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        self.create_account(store.account_id())
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        self.read_account(store.account_id())
    }
}
impl AccountProtectionSource for ForeignSource {
    fn create_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        self.secret(self.0.create(id.as_u64()))
    }
    fn read_account(&self, id: AccountId) -> Result<Secret, VaultError> {
        self.secret(self.0.read(id.as_u64()))
    }
    fn delete_account(&self, id: AccountId) -> Result<(), VaultError> {
        self.0.delete(id.as_u64()).map_err(|_| VaultError::Unavailable)
    }
}
type Vault = Arc<dyn AccountProtectionSource + Send + Sync>;
/// The callback if given, else the platform vault of the engine (iOS Keychain).
fn vault(secrets: Option<Arc<dyn SecretStore>>) -> Result<Vault, MobileError> {
    match secrets {
        Some(store) => Ok(Arc::new(ForeignSource(store))),
        #[cfg(target_os = "ios")]
        None => Ok(Arc::new(matrix_engine::keychain::IosKeychain)),
        #[cfg(not(target_os = "ios"))]
        None => Err(MobileError::SecretStore),
    }
}

/// `<data_dir>/accounts/<id>`: the whole profile (lock, selector, store) of one account.
fn profile_root(data_dir: &Path, account: u64) -> PathBuf {
    data_dir.join("accounts").join(account.to_string())
}
fn valid_id(account: u64) -> bool {
    (1..=128).contains(&account)
}
/// Accounts with a completed sign-in, ascending. A directory without the `active-account`
/// selector (a refused or interrupted sign-in) is not listed.
fn saved_ids(data_dir: &Path) -> Result<Vec<u64>, MobileError> {
    let entries = match std::fs::read_dir(data_dir.join("accounts")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(MobileError::Storage),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| MobileError::Storage)?;
        let id = entry.file_name().to_str().and_then(|name| {
            let id = name.parse::<u64>().ok()?;
            (valid_id(id) && name == id.to_string()).then_some(id)
        });
        if let Some(id) = id.filter(|_| entry.path().join("active-account").is_file()) {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    Ok(ids)
}
/// Creates `data_dir` and `<data_dir>/accounts` (0700) when missing; the parent of `data_dir` must exist.
fn ensure_dirs(data_dir: &Path) -> Result<(), MobileError> {
    use std::os::unix::fs::DirBuilderExt;
    for dir in [data_dir.to_owned(), data_dir.join("accounts")] {
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(MobileError::Storage);
            }
            _ => (),
        }
    }
    Ok(())
}
/// Smallest account ID whose directory does not exist yet.
fn free_id(data_dir: &Path) -> Result<u64, MobileError> {
    (1..=128)
        .find(|id| std::fs::symlink_metadata(profile_root(data_dir, *id)).is_err())
        .ok_or(MobileError::Storage)
}
impl ForeignSource {
    fn secret(&self, text: Result<String, MobileError>) -> Result<Secret, VaultError> {
        Secret::new(text.map_err(|_| VaultError::Unavailable)?).map_err(|_| VaultError::InvalidItem)
    }
}

/// Connection state, as the engine reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncState {
    Connecting,
    Ready,
    AuthenticationFailed,
    SyncFailed,
}
impl From<RoomSyncStatus> for SyncState {
    fn from(status: RoomSyncStatus) -> Self {
        match status {
            RoomSyncStatus::Connecting => Self::Connecting,
            RoomSyncStatus::Ready => Self::Ready,
            RoomSyncStatus::AuthenticationFailed => Self::AuthenticationFailed,
            RoomSyncStatus::SyncFailed => Self::SyncFailed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RoomItem {
    pub room_id: String,
    pub name: String,
    pub unread: u64,
    pub mentions: u64,
    pub preview: Option<String>,
    pub is_space: bool,
    pub invited: bool,
    pub muted: bool,
}

/// Updates, all delivered from one dedicated thread: keep the handlers short.
#[uniffi::export(with_foreign)]
pub trait SyncListener: Send + Sync {
    fn on_status(&self, state: SyncState);
    /// The whole list, most recent first. Sent once per burst of engine changes.
    fn on_rooms(&self, rooms: Vec<RoomItem>);
    /// Outcome of a `send_text`; `error` is None when the server has the message.
    fn on_send_result(&self, room_id: String, transaction_id: String, error: Option<MobileError>);
}

/// Room-list projection of the engine events; what `SyncListener` is fed from.
#[derive(Default)]
struct RoomsView {
    rooms: HashMap<String, RoomItem>,
    order: Vec<String>,
    status: Option<RoomSyncStatus>,
    announced: Option<SyncState>,
    rooms_dirty: bool,
}
impl RoomsView {
    fn apply(&mut self, event: RoomListEvent) {
        match event {
            RoomListEvent::Diff(RoomListDiff::Upsert(room)) => {
                let id = room.scope.room_id.as_str().to_owned();
                let muted = room.info.muted();
                let item = RoomItem {
                    room_id: id.clone(),
                    name: room.name,
                    unread: room.unread,
                    mentions: room.info.mentions,
                    preview: room.info.preview,
                    is_space: room.info.is_space,
                    invited: room.info.invited,
                    muted,
                };
                if self.rooms.insert(id, item.clone()) != Some(item) {
                    self.rooms_dirty = true;
                }
            }
            RoomListEvent::Diff(RoomListDiff::Remove(scope)) => {
                self.rooms_dirty |= self.rooms.remove(scope.room_id.as_str()).is_some();
            }
            RoomListEvent::Order(order) => {
                self.order = order
                    .iter()
                    .map(|s| s.room_id.as_str().to_owned())
                    .collect();
                self.rooms_dirty = true;
            }
            RoomListEvent::Status(status) => self.status = Some(status),
            RoomListEvent::TimelineActivity(_) | RoomListEvent::Alert(..) => (),
        }
    }

    /// The engine stopped talking: a sync that was still starting or running has failed.
    fn closed(&mut self) {
        if matches!(
            self.status,
            None | Some(RoomSyncStatus::Connecting | RoomSyncStatus::Ready)
        ) {
            self.status = Some(RoomSyncStatus::SyncFailed);
        }
    }

    fn snapshot(&self) -> Vec<RoomItem> {
        let mut rest: Vec<&String> = self
            .rooms
            .keys()
            .filter(|id| !self.order.contains(id))
            .collect();
        rest.sort();
        self.order
            .iter()
            .chain(rest)
            .filter_map(|id| self.rooms.get(id).cloned())
            .collect()
    }

    fn flush(&mut self, listener: &dyn SyncListener) {
        let state = self.status.map(SyncState::from);
        if let Some(announce) = state.filter(|_| state != self.announced) {
            self.announced = state;
            listener.on_status(announce);
        }
        if std::mem::take(&mut self.rooms_dirty) {
            listener.on_rooms(self.snapshot());
        }
    }
}

fn run_events(
    mut rooms: Receiver<RoomListEvent>,
    mut sends: Receiver<SendEvent>,
    listener: Arc<dyn SyncListener>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        listener.on_status(SyncState::SyncFailed);
        return;
    };
    runtime.block_on(async {
        let mut view = RoomsView::default();
        let (mut rooms_open, mut sends_open) = (true, true);
        while rooms_open || sends_open {
            tokio::select! {
                first = rooms.recv(), if rooms_open => {
                    match first {
                        Some(event) => {
                            view.apply(event);
                            // One callback per burst: the first sync delivers every room as its own diff.
                            while let Ok(event) = rooms.try_recv() {
                                view.apply(event);
                            }
                        }
                        None => {
                            rooms_open = false;
                            view.closed();
                        }
                    }
                    view.flush(listener.as_ref());
                }
                event = sends.recv(), if sends_open => match event {
                    Some(SendEvent::Result(result)) => listener.on_send_result(
                        result.scope.room_id.as_str().to_owned(),
                        result.transaction_id.as_str().to_owned(),
                        result.result.err().map(MobileError::from),
                    ),
                    Some(SendEvent::Notice { .. }) => (),
                    None => sends_open = false,
                },
            }
        }
    });
}

/// A signed-in account syncing in the background.
#[derive(uniffi::Object)]
pub struct MobileSession {
    account: AccountId,
    root: PathBuf,
    vault: Vault,
    control: WorkerControl,
    sender: Mutex<Sender<SendRequest>>,
    worker: Mutex<Option<RoomListWorker>>,
    events: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[uniffi::export]
impl MobileSession {
    /// The account ID of this crate (1..=128): what `restore_session` and `list_saved_accounts` use.
    pub fn account_id(&self) -> u64 {
        self.account.as_u64()
    }

    /// Queues a plain-text message; the outcome arrives in `on_send_result` with the returned
    /// transaction id. The engine keeps the message across restarts until the server has it.
    pub fn send_text(&self, room_id: String, body: String) -> Result<String, MobileError> {
        if room_id.is_empty() || body.trim().is_empty() {
            return Err(MobileError::InvalidInput);
        }
        let transaction_id = fresh_transaction_id();
        let request = SendRequest {
            scope: RoomScope {
                account_id: self.account,
                room_id: RoomId::new(room_id),
            },
            transaction_id: transaction_id.clone(),
            body,
            attachment: None, poll: None,
            mentions: Vec::new(),
        };
        use matrix_engine::TrySendError::{Busy, Closed, Full};
        match self
            .sender
            .lock()
            .map_err(|_| MobileError::Closed)?
            .try_send(request)
        {
            Ok(()) => Ok(transaction_id.as_str().to_owned()),
            Err(Busy(_) | Full(_)) => Err(MobileError::Busy),
            Err(Closed(_)) => Err(MobileError::Closed),
        }
    }

    /// Stops syncing and waits for the worker to end: call it off the main thread.
    pub fn stop(&self) -> Result<(), MobileError> {
        let worker = self.worker.lock().map_err(|_| MobileError::Closed)?.take();
        let result = worker.map_or(Ok(()), |w| w.shutdown().map_err(|_| MobileError::Closed));
        if let Some(events) = self.events.lock().map_err(|_| MobileError::Closed)?.take() {
            let _ = events.join();
        }
        result
    }

    /// Signs this account out: stops syncing, revokes the session on the server, wipes its store
    /// and removes its vault item. Other accounts are untouched. Blocking (up to ~15 s of network):
    /// call it off the main thread. If the server could not be reached nothing is deleted and
    /// `Network` is returned: the session is stopped, so retry with `restore_session` then
    /// `logout`, or give up with `remove_local_account`.
    pub fn logout(&self) -> Result<(), MobileError> {
        self.control.logout_and_stop();
        self.stop()?;
        // The worker forgets the store (sealed credential first) only once the server has revoked.
        let sealed = self.root.join(format!("account-{}", self.account.as_u64()));
        if sealed.join("session.sealed").exists() {
            return Err(MobileError::Network);
        }
        self.vault
            .delete_profile(self.account, &self.root.join(format!("account-{}", self.account.as_u64())))
            .map_err(|_| MobileError::SecretStore)?;
        std::fs::remove_dir_all(&self.root).map_err(|_| MobileError::Storage)
    }
}

enum Job {
    SignIn {
        homeserver: String,
        user: String,
        password: Secret,
    },
    Restore,
}

/// Runs inside the worker runtime, where the SDK client is born and must live.
async fn open_session(
    root: PathBuf,
    account: AccountId,
    source: Vault,
    job: Job,
    trust: Option<Vec<u8>>,
) -> Result<AccountSession, SignInError> {
    let mut profile = SignInProfile::open_as(root, account)?;
    match job {
        Job::SignIn {
            homeserver,
            user,
            password,
        } => {
            profile
                .password_login_vault(source.as_ref(), homeserver, user, password, trust)
                .await
        }
        Job::Restore if profile.active_account().is_none() => Err(SignInError::NoSession),
        Job::Restore => {
            profile
                .unlock_vault_timed(source, trust, Duration::from_secs(10))
                .await
        }
    }
}

async fn start(
    root: PathBuf,
    account: AccountId,
    source: Vault,
    listener: Arc<dyn SyncListener>,
    job: Job,
    trust: Option<Vec<u8>>,
) -> Result<Arc<MobileSession>, MobileError> {
    let (done, outcome) = tokio::sync::oneshot::channel::<Result<AccountId, MobileError>>();
    let (session_root, session_source) = (root.clone(), source.clone());
    let (mut worker, rooms) = RoomListWorker::spawn(move || async move {
        match open_session(session_root, account, session_source, job, trust).await {
            Ok(session) => {
                let _ = done.send(Ok(session.account_id()));
                Ok(session)
            }
            Err(error) => {
                let _ = done.send(Err(error.into()));
                Err(AuthError::LoginFailed)
            }
        }
    })
    .map_err(|_| MobileError::Storage)?;
    // On failure `worker` drops here, which stops it.
    let signed_in = outcome.await.map_err(|_| MobileError::Closed)?;
    if let Err(error) = signed_in {
        drop(worker);
        // A refused first sign-in leaves an empty profile directory behind: no selector, no store.
        // Busy: another sign-in owns this directory right now, so it is not ours to delete.
        if error != MobileError::Busy && !root.join("active-account").exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        return Err(error);
    }
    let control = worker.control();
    let (sender, sends) = worker.take_send_route().ok_or(MobileError::Closed)?;
    let events = std::thread::Builder::new()
        .name("matrix-mobile-events".into())
        .spawn(move || run_events(rooms, sends, listener))
        .map_err(|_| MobileError::Storage)?;
    Ok(Arc::new(MobileSession {
        account,
        root,
        vault: source,
        control,
        sender: Mutex::new(sender),
        worker: Mutex::new(Some(worker)),
        events: Mutex::new(Some(events)),
    }))
}

/// Accounts signed in earlier in `data_dir`, ascending; empty on a first launch. Reads the
/// directory only: no vault access, no network. Feed each ID to `restore_session`.
#[uniffi::export]
pub fn list_saved_accounts(data_dir: String) -> Result<Vec<u64>, MobileError> {
    saved_ids(Path::new(&data_dir))
}

/// Adds an account (several can be signed in at once, each with its own session). The new
/// account gets the smallest free ID, readable from `MobileSession::account_id`.
/// `data_dir` is an absolute app-private directory (created if missing, parent must exist).
/// `secrets`: the keystore callback, or `None` to use the platform vault (iOS Keychain).
/// `trusted_root_pem`: extra CA for a private homeserver (test servers); None for public ones.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sign_in(
    data_dir: String,
    homeserver: String,
    user_id: String,
    password: String,
    secrets: Option<Arc<dyn SecretStore>>,
    listener: Arc<dyn SyncListener>,
    trusted_root_pem: Option<String>,
) -> Result<Arc<MobileSession>, MobileError> {
    let password = Secret::new(password)?;
    let source = vault(secrets)?;
    let data_dir = PathBuf::from(data_dir);
    ensure_dirs(&data_dir)?;
    let account = free_id(&data_dir)?;
    let job = Job::SignIn {
        homeserver,
        user: user_id,
        password,
    };
    start(
        profile_root(&data_dir, account),
        AccountId::new(account),
        source,
        listener,
        job,
        trusted_root_pem.map(String::into_bytes),
    )
    .await
}

/// Reopens one account listed by `list_saved_accounts`, without the password; `NoSession` when
/// `account` is not saved, `Busy` when it is already open. A server that no longer accepts the
/// session gives `Rejected`, a lost vault item `SecretStore`: sign in again or `remove_local_account`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn restore_session(
    data_dir: String,
    account: u64,
    secrets: Option<Arc<dyn SecretStore>>,
    listener: Arc<dyn SyncListener>,
    trusted_root_pem: Option<String>,
) -> Result<Arc<MobileSession>, MobileError> {
    let source = vault(secrets)?;
    let data_dir = PathBuf::from(data_dir);
    if !saved_ids(&data_dir)?.contains(&account) {
        return Err(MobileError::NoSession);
    }
    start(
        profile_root(&data_dir, account),
        AccountId::new(account),
        source,
        listener,
        Job::Restore,
        trusted_root_pem.map(String::into_bytes),
    )
    .await
}

/// Forgets an account on this device only: deletes its vault item and its directory, without
/// telling the server (the device stays in the server's session list). For an account that cannot
/// be restored or logged out. `Busy` while the account is open; `NoSession` when not saved.
/// Other accounts are untouched. Blocking: call it off the main thread.
#[uniffi::export]
pub fn remove_local_account(
    data_dir: String,
    account: u64,
    secrets: Option<Arc<dyn SecretStore>>,
) -> Result<(), MobileError> {
    let source = vault(secrets)?;
    let data_dir = PathBuf::from(data_dir);
    if !saved_ids(&data_dir)?.contains(&account) {
        return Err(MobileError::NoSession);
    }
    let id = AccountId::new(account);
    let root = profile_root(&data_dir, account);
    // The profile lock only serialises sign-ins; an open session holds the store lock.
    let _lock = SignInProfile::open_as(root.clone(), id).map_err(MobileError::from)?;
    // Any other open error (broken store) is exactly what this call exists to clean up.
    let _store = match AccountStore::open(&root, id) {
        Err(StoreError::Busy) => return Err(MobileError::Busy),
        other => other.ok(),
    };
    source
        .delete_profile(id, &root.join(format!("account-{}", id.as_u64())))
        .map_err(|_| MobileError::SecretStore)?;
    std::fs::remove_dir_all(root).map_err(|_| MobileError::Storage)
}

#[cfg(test)]
mod tests;
