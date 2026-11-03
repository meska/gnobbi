// Same reason as matrix-engine: the worker future is generic over the login future and nests deeply.
#![recursion_limit = "256"]
//! UniFFI bridge for the mobile apps (Swift/Kotlin). A thin layer over `matrix-engine`: sign-in,
//! session restore, the room-list projection with its sync status, and plain-text sends.
//!
//! The engine worker owns the SDK client and its own runtime thread; this crate never touches the SDK.
//! The protection secret of the account store lives in the platform keychain/keystore, reached
//! only through the [`SecretStore`] callback the app implements (no desktop vault is compiled in).
//! Updates are pushed through [`SyncListener`], always from one dedicated thread.
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    Receiver, RoomListDiff, RoomListEvent, RoomSyncStatus, Sender,
    auth::{AccountSession, AuthError},
    persistence::AccountStore,
    room_sync::RoomListWorker,
    sending::{SendError, SendEvent, SendRequest, fresh_transaction_id},
    signin::{ProtectionSource, Secret, SignInError, SignInProfile, VaultError},
};
use std::{
    collections::HashMap,
    path::PathBuf,
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

/// Where the protection secret of an account store is kept (iOS Keychain, Android Keystore).
/// The app generates the secret (at least 16 random characters) on `create`. Calls are blocking
/// and come from a background thread, never the main one.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn create(&self, account: u64) -> Result<String, MobileError>;
    fn read(&self, account: u64) -> Result<String, MobileError>;
}
struct ForeignSource(Arc<dyn SecretStore>);
impl ProtectionSource for ForeignSource {
    fn create(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        self.secret(self.0.create(store.account_id().as_u64()))
    }
    fn read(&self, store: &AccountStore) -> Result<Secret, VaultError> {
        self.secret(self.0.read(store.account_id().as_u64()))
    }
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
                let item = RoomItem {
                    room_id: id.clone(),
                    name: room.name,
                    unread: room.unread,
                    mentions: room.info.mentions,
                    preview: room.info.preview,
                    is_space: room.info.is_space,
                    invited: room.info.invited,
                    muted: room.info.muted,
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
            RoomListEvent::TimelineActivity(_) | RoomListEvent::Alert(_) => (),
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
    sender: Mutex<Sender<SendRequest>>,
    worker: Mutex<Option<RoomListWorker>>,
    events: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[uniffi::export]
impl MobileSession {
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
            attachment: None,
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
    secrets: Arc<dyn SecretStore>,
    job: Job,
    trust: Option<Vec<u8>>,
) -> Result<AccountSession, SignInError> {
    let mut profile = SignInProfile::open(root)?;
    let source = ForeignSource(secrets);
    match job {
        Job::SignIn {
            homeserver,
            user,
            password,
        } => {
            profile
                .password_login_vault(&source, homeserver, user, password, trust)
                .await
        }
        Job::Restore if profile.active_account().is_none() => Err(SignInError::NoSession),
        Job::Restore => {
            profile
                .unlock_vault_timed(Arc::new(source), trust, Duration::from_secs(10))
                .await
        }
    }
}

async fn start(
    data_dir: String,
    secrets: Arc<dyn SecretStore>,
    listener: Arc<dyn SyncListener>,
    job: Job,
    trust: Option<Vec<u8>>,
) -> Result<Arc<MobileSession>, MobileError> {
    let (done, outcome) = tokio::sync::oneshot::channel::<Result<AccountId, MobileError>>();
    let root = PathBuf::from(data_dir);
    let (mut worker, rooms) = RoomListWorker::spawn(move || async move {
        match open_session(root, secrets, job, trust).await {
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
    let account = outcome.await.map_err(|_| MobileError::Closed)??;
    let (sender, sends) = worker.take_send_route().ok_or(MobileError::Closed)?;
    let events = std::thread::Builder::new()
        .name("matrix-mobile-events".into())
        .spawn(move || run_events(rooms, sends, listener))
        .map_err(|_| MobileError::Storage)?;
    Ok(Arc::new(MobileSession {
        account,
        sender: Mutex::new(sender),
        worker: Mutex::new(Some(worker)),
        events: Mutex::new(Some(events)),
    }))
}

/// `trusted_root_pem`: extra CA for a private homeserver (test servers); None for public ones.
/// First sign-in: `data_dir` is an absolute app-private directory (it is created if missing).
#[uniffi::export(async_runtime = "tokio")]
pub async fn sign_in(
    data_dir: String,
    homeserver: String,
    user_id: String,
    password: String,
    secrets: Arc<dyn SecretStore>,
    listener: Arc<dyn SyncListener>,
    trusted_root_pem: Option<String>,
) -> Result<Arc<MobileSession>, MobileError> {
    let password = Secret::new(password)?;
    let job = Job::SignIn {
        homeserver,
        user: user_id,
        password,
    };
    start(
        data_dir,
        secrets,
        listener,
        job,
        trusted_root_pem.map(String::into_bytes),
    )
    .await
}

/// Reopens the account signed in earlier in `data_dir`; `NoSession` when there is none.
#[uniffi::export(async_runtime = "tokio")]
pub async fn restore_session(
    data_dir: String,
    secrets: Arc<dyn SecretStore>,
    listener: Arc<dyn SyncListener>,
    trusted_root_pem: Option<String>,
) -> Result<Arc<MobileSession>, MobileError> {
    start(
        data_dir,
        secrets,
        listener,
        Job::Restore,
        trusted_root_pem.map(String::into_bytes),
    )
    .await
}

#[cfg(test)]
mod tests;
