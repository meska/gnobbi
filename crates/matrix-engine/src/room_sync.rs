//! Backend-only Matrix /sync room projection. Owns one authenticated session.
use crate::{RoomListDiff, RoomSummary, auth::AccountSession};
use matrix_domain::{RoomId, RoomScope};
use matrix_sdk::RoomState;
use matrix_sdk::ruma::events::room::message::MessageType;
use std::{collections::BTreeMap, time::Duration};

pub use crate::{RoomListEvent, RoomSyncStatus, WorkerControl};

#[cfg(test)]
#[path = "room_sync_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownError {
    Runtime,
    WorkerPanicked,
    WriteUnconfirmed,
    DrainDeadline,
}

/// Own outside the UI entity: request cancellation there, join after GPUI exits.
/// Drop requests stop; only shutdown proves the worker/runtime have terminated.
pub struct RoomListWorker {
    media_route: Option<(
        crate::Sender<crate::media::MediaCommand>,
        crate::Receiver<crate::media::MediaEvent>,
    )>,
    verification_route: Option<(
        crate::Sender<crate::verification::VerificationCommand>,
        crate::Receiver<crate::verification::VerificationEvent>,
    )>,
    operation_route: Option<(
        crate::Sender<crate::operations::OperationCommand>,
        crate::Receiver<crate::operations::OperationResult>,
    )>,
    send_route: Option<(
        crate::Sender<crate::sending::SendRequest>,
        crate::Receiver<crate::sending::SendResult>,
    )>,
    avatars: crate::avatar::AvatarSlot,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    logout: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<(), ShutdownError>>>,
    history_route: Option<(
        crate::Sender<crate::history::HistoryRequest>,
        crate::Receiver<crate::history::HistoryResult>,
    )>,
}
struct EmptyOnDrop(crate::avatar::AvatarSlot);
impl Drop for EmptyOnDrop {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}
struct DoneOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for DoneOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for RoomListWorker {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }
}
impl RoomListWorker {
    pub fn spawn<F, Fut>(login: F) -> std::io::Result<(Self, crate::Receiver<RoomListEvent>)>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<AccountSession, crate::auth::AuthError>> + 'static,
    {
        let (mut sender, receiver) = crate::bounded(std::num::NonZeroUsize::new(64).unwrap());
        let (history_commands, mut requests) = crate::bounded::<crate::history::HistoryRequest>(
            std::num::NonZeroUsize::new(8).unwrap(),
        );
        let (mut history_sender, history_events) =
            crate::bounded(std::num::NonZeroUsize::new(8).unwrap());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (send_commands, mut sends) =
            crate::bounded::<crate::sending::SendRequest>(std::num::NonZeroUsize::new(8).unwrap());
        let (mut send_results, send_events) =
            crate::bounded(std::num::NonZeroUsize::new(8).unwrap());
        let (operation_commands, mut operations) =
            crate::bounded::<crate::operations::OperationCommand>(
                std::num::NonZeroUsize::new(8).unwrap(),
            );
        let (mut operation_results, operation_events) =
            crate::bounded(std::num::NonZeroUsize::new(8).unwrap());
        let (verification_commands, verification_requests) =
            crate::bounded(std::num::NonZeroUsize::new(8).unwrap());
        let (verification_results, verification_events) =
            crate::bounded(std::num::NonZeroUsize::new(8).unwrap());
        let cancelled = stop.clone();
        let logout = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (worker_logout, worker_done) = (logout.clone(), done.clone());
        let avatars = crate::avatar::AvatarSlot::default();
        let worker_avatars = avatars.clone();
        let (media_commands, mut media_requests) =
            crate::bounded::<crate::media::MediaCommand>(std::num::NonZeroUsize::new(2).unwrap());
        let (mut media_results, media_events) =
            crate::bounded(std::num::NonZeroUsize::new(2).unwrap());
        let thread = std::thread::Builder::new()
            .name("matrix-room-sync".into())
            .spawn(move || {
                // Declared first, so it drops after the runtime and the session.
                let _done = DoneOnDrop(worker_done);
                // Dropped before `_done`, after the runtime: the window must not keep the client.
                let _avatars = EmptyOnDrop(worker_avatars.clone());
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return Err(ShutdownError::Runtime);
                };
                runtime.block_on(async move {
                    if sender
                        .send(RoomListEvent::Status(RoomSyncStatus::Connecting))
                        .await
                        .is_err()
                    {
                        return unstarted_shutdown(
                            &mut sends,
                            &mut operations,
                            &mut media_requests,
                        )
                        .await;
                    }
                    let session = match until_stopped(&cancelled, login()).await {
                        None => {
                            return unstarted_shutdown(
                                &mut sends,
                                &mut operations,
                                &mut media_requests,
                            )
                            .await;
                        }
                        Some(Ok(session)) => session,
                        Some(Err(_)) => {
                            let _ = sender
                                .send(RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed))
                                .await;
                            return unstarted_shutdown(
                                &mut sends,
                                &mut operations,
                                &mut media_requests,
                            )
                            .await;
                        }
                    };
                    let mut sync = RoomSync::new(session);
                    let client = sync.session.client.as_ref().unwrap().clone();
                    let account = sync.session.account_id();
                    let media_client = client.clone();
                    #[cfg(unix)]
                    if let Some(fetcher) = crate::avatar_cache::AvatarFetcher::new(
                        &sync.session,
                        tokio::runtime::Handle::current(),
                    ) && let Ok(mut slot) = worker_avatars.lock()
                    {
                        *slot = Some(Box::new(fetcher));
                    }
                    #[cfg(unix)]
                    let video_dir = sync.session.store.as_ref().and_then(|s| {
                        let dir = s.video_dir();
                        use std::os::unix::fs::DirBuilderExt;
                        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => None,
                            _ => Some(dir),
                        }
                    });
                    let media_stop = cancelled.clone();
                    let mut media_task = tokio::spawn(async move {
                        let mut clean = true;
                        let mut cache = crate::media::MediaCache::new(8 * 1024 * 1024);
                        while let Some(command) =
                            shutdown_receive(&media_stop, &mut media_requests).await
                        {
                            use crate::media::{MediaCommand, MediaEvent};
                            let result = match command {
                                MediaCommand::Send {
                                    scope,
                                    transaction,
                                    bytes,
                                } => {
                                    let result = crate::media_sdk::send_png(
                                        &media_client,
                                        account,
                                        &scope,
                                        &transaction,
                                        bytes,
                                    )
                                    .await;
                                    MediaEvent::Sent {
                                        scope,
                                        transaction,
                                        result,
                                    }
                                }
                                MediaCommand::Download {
                                    tag,
                                    event,
                                    original,
                                } => {
                                    if tag.is_cancelled()
                                        || media_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    let key = |variant| crate::media::MediaKey {
                                        variant,
                                        scope: tag.scope().clone(),
                                        event_id: event.clone(),
                                    };
                                    use crate::media::MediaVariant::{Original, Thumbnail};
                                    let variants: &[_] = if original {
                                        &[Original]
                                    } else {
                                        &[Thumbnail, Original]
                                    };
                                    let hit = variants
                                        .iter()
                                        .copied()
                                        .find_map(|v| cache.get(&key(v)).map(|b| (v, b.to_vec())));
                                    let read = async {
                                        if let Some((variant, bytes)) = hit {
                                            let limit = if original {
                                                (
                                                    crate::media_sdk::LARGE_VIEW_SIDE,
                                                    crate::media_sdk::LARGE_VIEW_SIDE,
                                                )
                                            } else {
                                                (320, 240)
                                            };
                                            let preview = crate::media_sdk::decode_within(
                                                bytes.clone(),
                                                crate::media_sdk::mime_of(&bytes).ok_or(
                                                    crate::media::MediaError::InvalidImage,
                                                )?,
                                                &tag,
                                                limit,
                                            )
                                            .await?;
                                            Ok(crate::media::DownloadedPng {
                                                variant,
                                                bytes,
                                                preview,
                                            })
                                        } else {
                                            let downloaded = crate::media_sdk::download_png(
                                                &media_client,
                                                account,
                                                &tag,
                                                &event,
                                                original,
                                            )
                                            .await?;
                                            cache.insert(
                                                key(downloaded.variant),
                                                downloaded.bytes.clone(),
                                            );
                                            Ok(downloaded)
                                        }
                                    };
                                    let Some(result) = until_stopped(&media_stop, read).await
                                    else {
                                        continue;
                                    };
                                    MediaEvent::Downloaded { tag, event, result }
                                }
                                MediaCommand::OpenVideo { tag, event } => {
                                    if tag.is_cancelled()
                                        || media_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    #[cfg(unix)]
                                    let read = async {
                                        let profile = video_dir
                                            .as_deref()
                                            .ok_or(crate::media::MediaError::Failed)?;
                                        crate::video_file::download_video(
                                            &media_client,
                                            account,
                                            &tag,
                                            &event,
                                            profile,
                                            crate::video_file::MAX_VIDEO_BYTES,
                                            |_| (),
                                        )
                                        .await
                                    };
                                    #[cfg(not(unix))]
                                    let read = async { Err(crate::media::MediaError::Failed) };
                                    let Some(result) = until_stopped(&media_stop, read).await
                                    else {
                                        continue;
                                    };
                                    MediaEvent::VideoDownloaded { tag, event, result }
                                }
                                MediaCommand::VideoThumbnail { tag, event } => {
                                    if tag.is_cancelled()
                                        || media_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    let read = async {
                                        // No thumbnail is not an error for the video, but there is no picture to draw.
                                        crate::media_sdk::download_video_thumbnail(
                                            &media_client,
                                            account,
                                            &tag,
                                            &event,
                                        )
                                        .await?
                                        .ok_or(crate::media::MediaError::UnsupportedType)
                                    };
                                    let Some(result) = until_stopped(&media_stop, read).await
                                    else {
                                        continue;
                                    };
                                    MediaEvent::Downloaded { tag, event, result }
                                }
                            };
                            let failed = matches!(
                                &result,
                                crate::media::MediaEvent::Sent { result: Err(_), .. }
                            );
                            if !matches!(
                                until_stopped(&media_stop, media_results.send(result)).await,
                                Some(Ok(()))
                            ) && failed
                            {
                                clean = false;
                            }
                        }
                        clean
                    });
                    let verification_task = tokio::spawn(crate::verification::run_service(
                        client.clone(),
                        account,
                        verification_requests,
                        verification_results,
                    ));
                    let operation_client = client.clone();
                    let operation_state = sync.operation_state.clone();
                    let operation_stop = cancelled.clone();
                    let mut operations_task = tokio::spawn(async move {
                        let mut clean = true;
                        use crate::operations::{OperationCommand, OperationResult};
                        while let Some(command) =
                            shutdown_receive(&operation_stop, &mut operations).await
                        {
                            let result = match command {
                                OperationCommand::Inspect { tag, target } => {
                                    if tag.is_cancelled()
                                        || operation_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    let details = until_stopped(
                                        &operation_stop,
                                        crate::operations::details(
                                            &operation_client,
                                            account,
                                            tag.scope(),
                                            &target,
                                        ),
                                    )
                                    .await;
                                    let Some(details) = details else {
                                        continue;
                                    };
                                    if tag.is_cancelled() {
                                        continue;
                                    }
                                    OperationResult::Inspected {
                                        tag,
                                        target,
                                        details,
                                    }
                                }
                                OperationCommand::Members { scope } => {
                                    let members = crate::operations::members(
                                        &operation_client,
                                        account,
                                        &scope,
                                    )
                                    .await;
                                    OperationResult::Members { scope, members }
                                }
                                OperationCommand::MarkRoom {
                                    scope,
                                    read,
                                    public,
                                } => {
                                    let result = crate::operations::mark_room(
                                        &operation_client,
                                        account,
                                        &scope,
                                        read,
                                        public,
                                    )
                                    .await;
                                    OperationResult::RoomMarked { scope, result }
                                }
                                OperationCommand::StartDm { user_id } => {
                                    let result = crate::operations::start_dm(
                                        &operation_client,
                                        account,
                                        &user_id,
                                    )
                                    .await;
                                    OperationResult::RoomStarted { result }
                                }
                                OperationCommand::CreateRoom { name, public } => {
                                    let result = crate::operations::create_room(
                                        &operation_client,
                                        account,
                                        &name,
                                        public,
                                    )
                                    .await;
                                    OperationResult::RoomStarted { result }
                                }
                                OperationCommand::Apply(request) => {
                                    let result = crate::operations::perform(
                                        &operation_client,
                                        account,
                                        &operation_state,
                                        &request,
                                    )
                                    .await;
                                    OperationResult::Applied { request, result }
                                }
                            };
                            let failed =
                                matches!(&result, OperationResult::Applied { result: Err(_), .. });
                            if !matches!(
                                until_stopped(&operation_stop, operation_results.send(result))
                                    .await,
                                Some(Ok(()))
                            ) && failed
                            {
                                clean = false;
                            }
                        }
                        clean
                    });
                    let send_client = client.clone();
                    let send_stop = cancelled.clone();
                    let mut sending_task = tokio::spawn(async move {
                        let mut clean = true;
                        while let Some(request) = shutdown_receive(&send_stop, &mut sends).await {
                            let result =
                                crate::sending::send_text(&send_client, account, &request).await;
                            let failed = result.is_err();
                            if !matches!(
                                until_stopped(
                                    &send_stop,
                                    send_results.send(crate::sending::SendResult {
                                        scope: request.scope,
                                        transaction_id: request.transaction_id,
                                        result,
                                    }),
                                )
                                .await,
                                Some(Ok(()))
                            ) && failed
                            {
                                clean = false;
                            }
                        }
                        clean
                    });
                    // Set once the user opens a room: the background warm-up then yields.
                    let opened = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let warm_client = client.clone();
                    let warm_opened = opened.clone();
                    let warm_stop = cancelled.clone();
                    // Runtime owns this read-only task; shutdown drops/cancels it.
                    let history_task = tokio::spawn(async move {
                        while let Some(request) = requests.recv().await {
                            if request.tag.is_cancelled() {
                                continue;
                            }
                            if request.open {
                                opened.store(true, std::sync::atomic::Ordering::Release);
                            }
                            if !serve_history(&client, account, request, &mut history_sender).await
                            {
                                break;
                            }
                        }
                    });
                    let work = async {
                        if let Ok(saved) = sync.saved_batch().await {
                            for change in saved {
                                if sender.send(RoomListEvent::Diff(change)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        // Runtime drop ends the warm-up with the account; one pass after the first sync.
                        let start_warm = || {
                            let (client, opened, stop) =
                                (warm_client.clone(), warm_opened.clone(), warm_stop.clone());
                            tokio::spawn(async move {
                                crate::warm::warm(&client, &stop, &opened).await;
                            });
                        };
                        crate::sliding::run(&mut sync, &mut sender, start_warm).await;
                    };
                    let _ = until_stopped(&cancelled, work).await;
                    cancelled.store(true, std::sync::atomic::Ordering::Release);
                    verification_task.abort();
                    let _ = verification_task.await;
                    history_task.abort();
                    let _ = history_task.await;
                    // Keep session/account lock alive while accepted writes finish.
                    let mut send_joined = false;
                    let mut operations_joined = false;
                    let mut media_joined = false;
                    let drained = tokio::time::timeout(Duration::from_secs(6), async {
                        let sends = (&mut sending_task).await;
                        send_joined = true;
                        let operations = (&mut operations_task).await;
                        operations_joined = true;
                        let media = (&mut media_task).await;
                        media_joined = true;
                        match (sends, operations, media) {
                            (Ok(true), Ok(true), Ok(true)) => Ok(()),
                            (Ok(_), Ok(_), Ok(_)) => Err(ShutdownError::WriteUnconfirmed),
                            _ => Err(ShutdownError::WorkerPanicked),
                        }
                    })
                    .await
                    .unwrap_or(Err(ShutdownError::DrainDeadline));
                    if !send_joined {
                        sending_task.abort();
                        let _ = sending_task.await;
                    }
                    if !operations_joined {
                        operations_task.abort();
                        let _ = operations_task.await;
                    }
                    if !media_joined {
                        media_task.abort();
                        let _ = media_task.await;
                    }
                    if worker_logout.load(std::sync::atomic::Ordering::Acquire) {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(15), sync.session.logout())
                                .await;
                    }
                    drop(sync);
                    drained
                })
            })?;
        Ok((
            Self {
                media_route: Some((media_commands, media_events)),
                verification_route: Some((verification_commands, verification_events)),
                operation_route: Some((operation_commands, operation_events)),
                send_route: Some((send_commands, send_events)),
                avatars,
                stop,
                logout,
                done,
                thread: Some(thread),
                history_route: Some((history_commands, history_events)),
            },
            receiver,
        ))
    }

    /// Filled once the session is up, empty again when the worker ends.
    pub fn avatar_slot(&self) -> crate::avatar::AvatarSlot {
        self.avatars.clone()
    }

    pub fn control(&self) -> WorkerControl {
        WorkerControl::new(self.stop.clone(), self.logout.clone(), self.done.clone())
    }

    /// Single-owner, bounded two-command media route.
    pub fn take_media_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::media::MediaCommand>,
        crate::Receiver<crate::media::MediaEvent>,
    )> {
        self.media_route.take()
    }

    pub fn take_verification_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::verification::VerificationCommand>,
        crate::Receiver<crate::verification::VerificationEvent>,
    )> {
        self.verification_route.take()
    }

    pub fn take_operation_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::operations::OperationCommand>,
        crate::Receiver<crate::operations::OperationResult>,
    )> {
        self.operation_route.take()
    }

    /// Take the single UI owner route; bounded8 requests/results, no SDK types.
    pub fn take_send_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::sending::SendRequest>,
        crate::Receiver<crate::sending::SendResult>,
    )> {
        self.send_route.take()
    }

    /// Take the single UI owner history route.
    pub fn take_history_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::history::HistoryRequest>,
        crate::Receiver<crate::history::HistoryResult>,
    )> {
        self.history_route.take()
    }

    /// Call after the UI loop, never within a render/input callback.
    pub fn shutdown(mut self) -> Result<(), ShutdownError> {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| ShutdownError::WorkerPanicked)?
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    LoggedOut,
    Transport,
}

pub struct RoomSync {
    session: AccountSession,
    previous: BTreeMap<String, RoomSummary>,
    timeline_heads: BTreeMap<String, matrix_domain::EventId>,
    /// Rooms whose SDK latest event is being tracked (a store written before sliding sync has none).
    latest_watched: std::collections::BTreeSet<String>,
    members_loaded: std::collections::BTreeSet<String>,
    receipts: crate::receipts::PeerReceipts,
    /// Sliding sync sends no server counts: unread comes from the SDK's client-side counts.
    pub(crate) sliding_counts: bool,
    sent_markers: std::sync::Mutex<crate::receipts::SentMarkers>,
    operation_state: std::sync::Arc<tokio::sync::Mutex<crate::operations::OperationJournal>>,
}

impl RoomSync {
    pub async fn message_details(
        &self,
        scope: &RoomScope,
        target: &matrix_domain::EventId,
    ) -> Result<crate::operations::MessageDetails, crate::operations::OperationError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::operations::OperationError::Unavailable)?;
        crate::operations::details(client, self.session.account_id(), scope, target).await
    }

    pub async fn operate(
        &self,
        request: &crate::operations::OperationRequest,
    ) -> Result<matrix_domain::EventId, crate::operations::OperationError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::operations::OperationError::Unavailable)?;
        crate::operations::perform(
            client,
            self.session.account_id(),
            &self.operation_state,
            request,
        )
        .await
    }

    pub async fn send_text(
        &self,
        request: &crate::sending::SendRequest,
    ) -> Result<matrix_domain::EventId, crate::sending::SendError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::sending::SendError::Unavailable)?;
        crate::sending::send_text(client, self.session.account_id(), request).await
    }

    pub fn new(session: AccountSession) -> Self {
        let receipts = crate::receipts::PeerReceipts::new(
            session.account_id(),
            session
                .client
                .as_ref()
                .and_then(|client| client.user_id())
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        Self {
            session,
            previous: BTreeMap::new(),
            timeline_heads: BTreeMap::new(),
            latest_watched: Default::default(),
            members_loaded: Default::default(),
            receipts,
            sliding_counts: false,
            sent_markers: Default::default(),
            operation_state: std::sync::Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
    }

    /// Projection over an already logged-in client (the sliding-sync adapter and its tests).
    pub fn from_client(account_id: matrix_domain::AccountId, client: matrix_sdk::Client) -> Self {
        Self::new(AccountSession {
            account_id,
            client: Some(client),
            #[cfg(unix)]
            store: None,
        })
    }

    pub(crate) fn client(&self) -> Option<&matrix_sdk::Client> {
        self.session.client.as_ref()
    }

    #[cfg(test)]
    pub(crate) async fn mark_room_for_test(
        &self,
        scope: &RoomScope,
        read: bool,
    ) -> Result<(), crate::operations::OperationError> {
        let client = self.session.client.as_ref().expect("store-backed session");
        crate::operations::mark_room(client, self.session.account_id(), scope, read, true).await
    }

    /// Read one server page on the backend, never on the UI thread.
    pub async fn history_page(
        &self,
        tag: crate::RequestTag,
        before: Option<String>,
    ) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::history::HistoryError::Unavailable)?;
        fetch_history(client, self.session.account_id(), tag, before).await
    }

    /// Messages already in the SDK event cache, oldest first. Never touches the network.
    pub async fn cached_history(
        &self,
        tag: crate::RequestTag,
    ) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::history::HistoryError::Unavailable)?;
        saved_page(client, self.session.account_id(), tag).await
    }

    /// Open a room: the sync cursor brings the messages missed since the last run, and if
    /// that left a hole after the saved ones the SDK paginates just that hole.
    pub async fn open_history(
        &self,
        tag: crate::RequestTag,
        visibility: crate::receipts::ReadVisibility,
    ) -> Result<
        (
            crate::history::HistoryPage,
            Vec<crate::history::ReactionCount>,
        ),
        crate::history::HistoryError,
    > {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(crate::history::HistoryError::Unavailable)?;
        let scope = tag.scope().clone();
        let opened = open_page(client, tag).await?;
        if let (true, Some(last)) = (visibility.sees_last_message(), opened.0.events.last()) {
            self.mark_read(
                client,
                &scope.room_id,
                &last.event_id,
                visibility.public_receipts,
            )
            .await;
        }
        Ok(opened)
    }

    /// Public receipt and fully_read for the newest event; best effort, retried on the next open.
    async fn mark_read(
        &self,
        client: &matrix_sdk::Client,
        room_id: &RoomId,
        event: &matrix_domain::EventId,
        public: bool,
    ) {
        if !self.sent_markers.lock().unwrap().is_new(room_id, event) {
            return;
        }
        let (Ok(id), Ok(event_id)) = (
            matrix_sdk::ruma::RoomId::parse(room_id.as_str()),
            matrix_sdk::ruma::EventId::parse(event.as_str()),
        ) else {
            return;
        };
        let Some(room) = client.get_room(&id) else {
            return;
        };
        let receipts = crate::receipts::marker_receipts(event_id, public);
        if room.send_multiple_receipts(receipts).await.is_ok() {
            self.sent_markers
                .lock()
                .unwrap()
                .record(room_id.clone(), event.clone());
        }
    }

    /// Fixture: runs the live `sliding::run` until `Ready` and a quiet moment; returns what the
    /// room list changed compared to the previous call (the run itself empties and refills it
    /// while it starts, so the net difference is what a caller can rely on).
    #[cfg(test)]
    pub async fn next_batch(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        self.next_batch_after(async {}).await
    }

    /// Like `next_batch`, but `action` runs once the run is `Ready`: what it changes on the
    /// server reaches the room list as a live delivery, as in the app (sliding sync only counts
    /// unread from deliveries it sees, so messages sent before the run show as one).
    #[cfg(test)]
    pub async fn next_batch_after(
        &mut self,
        action: impl std::future::Future<Output = ()>,
    ) -> Result<Vec<RoomListDiff>, SyncError> {
        use crate::{RoomListEvent, RoomSyncStatus};
        let before = self.previous.clone();
        let (mut tx, mut rx) = crate::bounded(std::num::NonZeroUsize::new(1024).unwrap());
        let collect = async {
            let (mut ready, mut action) = (false, Some(action));
            loop {
                let wait = Duration::from_secs(if ready { 3 } else { 30 });
                match tokio::time::timeout(wait, rx.recv()).await {
                    Ok(Some(RoomListEvent::Status(RoomSyncStatus::Ready))) => {
                        ready = true;
                        if let Some(action) = action.take() {
                            action.await;
                        }
                    }
                    Ok(Some(RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed))) => {
                        return Err(SyncError::LoggedOut);
                    }
                    Ok(Some(_)) => {}
                    _ if ready => return Ok(()),
                    _ => return Err(SyncError::Transport),
                }
            }
        };
        tokio::select! {
            () = crate::sliding::run(self, &mut tx, || {}) => Err(SyncError::Transport),
            done = collect => done,
        }?;
        let mut diffs: Vec<_> = self
            .previous
            .iter()
            .filter(|(id, room)| before.get(*id) != Some(*room))
            .map(|(_, room)| RoomListDiff::Upsert(room.clone()))
            .collect();
        diffs.extend(
            before
                .iter()
                .filter(|(id, _)| !self.previous.contains_key(*id))
                .map(|(_, room)| RoomListDiff::Remove(room.scope.clone())),
        );
        Ok(diffs)
    }

    /// Folds one sync delivery (classic /sync or sliding sync) into peer receipts;
    /// returns the newest event id per room that had timeline events.
    pub(crate) fn observe_joined(
        &mut self,
        rooms: &matrix_sdk::sync::RoomUpdates,
    ) -> Vec<(String, matrix_domain::EventId)> {
        let mut heads = Vec::new();
        for (room, update) in &rooms.joined {
            let scope = RoomScope {
                account_id: self.session.account_id(),
                room_id: RoomId::new(room.as_str()),
            };
            if let Some(event) = update
                .timeline
                .events
                .iter()
                .rev()
                .find_map(|event| event.event_id())
            {
                heads.push((
                    room.to_string(),
                    matrix_domain::EventId::new(event.as_str()),
                ));
            }
            for raw in &update.ephemeral {
                self.receipts.observe_event(&scope, raw);
            }
        }
        heads
    }

    /// Rooms already in the SDK store. No network.
    pub async fn saved_batch(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        self.summarize().await
    }

    /// Every stored room, in store order: classic fixtures and recovery from missed deliveries.
    /// The app projects through `summarize_rooms` with the `RoomListService` list.
    pub(crate) async fn summarize(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        let rooms = self
            .session
            .client
            .as_ref()
            .ok_or(SyncError::LoggedOut)?
            .rooms();
        self.summarize_rooms(rooms).await
    }

    /// Summaries of the joined and invited rooms among `rooms`; the diff against the last call.
    pub(crate) async fn summarize_rooms(
        &mut self,
        rooms: Vec<matrix_sdk::Room>,
    ) -> Result<Vec<RoomListDiff>, SyncError> {
        let client = self.session.client.clone().ok_or(SyncError::LoggedOut)?;
        let mut current = BTreeMap::new();
        for room in rooms
            .into_iter()
            .filter(|room| matches!(room.state(), RoomState::Joined | RoomState::Invited))
        {
            let id = room.room_id().as_str().to_owned();
            // Once per room: the SDK computes a missing latest event from the event cache.
            if room.state() == RoomState::Joined && self.latest_watched.insert(id.clone()) {
                let _ = client
                    .latest_events()
                    .await
                    .listen_to_room(room.room_id())
                    .await;
            }
            // Sliding sync does not replay receipts sent before the app started: read the SDK store (1:1 only, the only place they are drawn).
            if room.state() == RoomState::Joined && room.joined_members_count() <= 2 {
                use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
                let scope = RoomScope {
                    account_id: self.session.account_id(),
                    room_id: RoomId::new(id.clone()),
                };
                // Lazy-loaded rooms know only some members: fetch the two once per room.
                if self.members_loaded.insert(id.clone()) {
                    let _ = room.sync_members().await;
                }
                for member in room
                    .members_no_sync(matrix_sdk::RoomMemberships::JOIN)
                    .await
                    .unwrap_or_default()
                {
                    if let Ok(Some((event, _))) = room
                        .load_user_receipt(
                            ReceiptType::Read,
                            &ReceiptThread::Unthreaded,
                            member.user_id(),
                        )
                        .await
                    {
                        self.receipts.observe(
                            &scope,
                            member.user_id().as_str(),
                            &matrix_domain::EventId::new(event.as_str()),
                        );
                    }
                }
            }
            let mut children = Vec::new();
            if room.state() == RoomState::Joined && room.is_space() {
                use matrix_sdk::deserialized_responses::SyncOrStrippedState;
                use matrix_sdk::ruma::events::{
                    SyncStateEvent, space::child::SpaceChildEventContent,
                };
                for event in room
                    .get_state_events_static::<SpaceChildEventContent>()
                    .await
                    .map_err(|_| SyncError::Transport)?
                {
                    if let Ok(SyncOrStrippedState::Sync(SyncStateEvent::Original(event))) =
                        event.deserialize()
                        && !event.content.via.is_empty()
                    {
                        children.push(RoomId::new(event.state_key.as_str()));
                    }
                }
                children.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                children.dedup();
            }
            current.insert(
                id.clone(),
                RoomSummary {
                    scope: RoomScope {
                        account_id: self.session.account_id(),
                        room_id: RoomId::new(id.clone()),
                    },
                    name: room_label(room.display_name().await.ok(), &id),
                    unread: if self.sliding_counts {
                        room.num_unread_notifications()
                    } else {
                        room.unread_notification_counts().notification_count
                    },
                    info: crate::RoomInfo {
                        public_read_receipts: self.receipts.snapshot(&RoomScope {
                            account_id: self.session.account_id(),
                            room_id: RoomId::new(id.clone()),
                        }),
                        preview: latest_event_text(&room).await,
                        invited: room.state() == RoomState::Invited,
                        topic: room.topic(),
                        avatar: room.avatar_url().map(|u| u.to_string()),
                        marked_unread: room.is_marked_unread(),
                        joined_members: room.joined_members_count(),
                        mentions: if self.sliding_counts {
                            room.num_unread_mentions()
                        } else {
                            room.unread_notification_counts().highlight_count
                        },
                        is_space: room.is_space(),
                        children,
                        encrypted: (!room.encryption_state().is_unknown())
                            .then(|| room.encryption_state().is_encrypted()),
                    },
                },
            );
        }
        let mut changes: Vec<_> = current
            .iter()
            .filter(|(id, room)| self.previous.get(*id) != Some(*room))
            .map(|(_, room)| RoomListDiff::Upsert(room.clone()))
            .collect();
        changes.extend(
            self.previous
                .iter()
                .filter(|(id, _)| !current.contains_key(*id))
                .map(|(_, room)| RoomListDiff::Remove(room.scope.clone())),
        );
        self.receipts.retain_rooms(
            &current
                .values()
                .map(|room| room.scope.room_id.clone())
                .collect::<Vec<_>>(),
        );
        self.previous = current;
        Ok(changes)
    }

    /// Scope of a room projected by the last `summarize_rooms` (joined or invited).
    pub(crate) fn scope_of(&self, room: &str) -> Option<RoomScope> {
        self.previous.get(room).map(|summary| summary.scope.clone())
    }

    pub(crate) fn timeline_activity(
        &mut self,
        heads: Vec<(String, matrix_domain::EventId)>,
    ) -> Vec<RoomScope> {
        self.timeline_heads
            .retain(|room, _| self.previous.contains_key(room));
        heads
            .into_iter()
            .filter_map(|(room, head)| {
                let summary = self.previous.get(&room)?;
                if self.timeline_heads.get(&room) == Some(&head) {
                    return None;
                }
                self.timeline_heads.insert(room, head);
                Some(summary.scope.clone())
            })
            .collect()
    }
}

/// Preview text from the SDK latest event (`LatestEventValue`, kept by the SDK across edits,
/// redactions and unsent local messages); None when it is not a message.
async fn latest_event_text(room: &matrix_sdk::Room) -> Option<String> {
    use matrix_sdk_ui::timeline::{LatestEventValue, RoomExt};
    let (LatestEventValue::Remote { content, .. } | LatestEventValue::Local { content, .. }) =
        room.latest_event().await
    else {
        return None;
    };
    let message = content.as_message()?;
    Some(match message.msgtype() {
        MessageType::Image(_) => "Immagine".to_owned(),
        _ => message.body().to_owned(),
    })
}

/// Name shown in the room list: the SDK-computed display name, the raw id only as a last resort.
fn room_label(name: Option<matrix_sdk::RoomDisplayName>, id: &str) -> String {
    use matrix_sdk::RoomDisplayName as N;
    match name {
        Some(N::Named(n) | N::Aliased(n) | N::Calculated(n) | N::EmptyWas(n)) if !n.is_empty() => n,
        _ => id.to_owned(),
    }
}

async fn unstarted_shutdown(
    sends: &mut crate::Receiver<crate::sending::SendRequest>,
    operations: &mut crate::Receiver<crate::operations::OperationCommand>,
    media: &mut crate::Receiver<crate::media::MediaCommand>,
) -> Result<(), ShutdownError> {
    sends.close();
    operations.close();
    media.close();
    let mut unconfirmed = false;
    while sends.recv().await.is_some() {
        unconfirmed = true;
    }
    while let Some(command) = operations.recv().await {
        unconfirmed |= matches!(command, crate::operations::OperationCommand::Apply(_));
    }
    while let Some(command) = media.recv().await {
        unconfirmed |= matches!(command, crate::media::MediaCommand::Send { .. });
    }
    if unconfirmed {
        Err(ShutdownError::WriteUnconfirmed)
    } else {
        Ok(())
    }
}

async fn shutdown_receive<T>(
    stop: &std::sync::atomic::AtomicBool,
    receiver: &mut crate::Receiver<T>,
) -> Option<T> {
    match until_stopped(stop, receiver.recv()).await {
        Some(value) => value,
        None => {
            receiver.close();
            receiver.recv().await
        }
    }
}

async fn until_stopped<F: std::future::Future>(
    stop: &std::sync::atomic::AtomicBool,
    work: F,
) -> Option<F::Output> {
    use std::{future::Future, task::Poll};
    let cancellation = async {
        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let mut work = std::pin::pin!(work);
    let mut cancellation = std::pin::pin!(cancellation);
    std::future::poll_fn(|cx| {
        if cancellation.as_mut().poll(cx).is_ready() {
            Poll::Ready(None)
        } else {
            work.as_mut().poll(cx).map(Some)
        }
    })
    .await
}

/// Saved messages of one room from the SDK event cache. Never touches the network.
async fn saved_page(
    client: &matrix_sdk::Client,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    use crate::history::HistoryError;
    if tag.scope().account_id != account_id {
        return Err(HistoryError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
        .map_err(|_| HistoryError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == RoomState::Joined)
        .ok_or(HistoryError::Unavailable)?;
    let (cached, _, next) = stored_events(client, &id).await?;
    let events = history_rows(client, &room, &id, cached.into_iter(), false).await?;
    Ok(crate::history::HistoryPage { tag, events, next })
}

async fn open_page(
    client: &matrix_sdk::Client,
    tag: crate::RequestTag,
) -> Result<
    (
        crate::history::HistoryPage,
        Vec<crate::history::ReactionCount>,
    ),
    crate::history::HistoryError,
> {
    use crate::history::HistoryError;
    let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
        .map_err(|_| HistoryError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == RoomState::Joined)
        .ok_or(HistoryError::Unavailable)?;
    let (mut rows, _, next) = stored_events(client, &id).await?;
    let anchor = rows.last().and_then(|e| e.event_id());
    // Subscribed first: the running SyncService feeds the cache, so missed messages arrive live.
    let (events, _drop) = room
        .event_cache()
        .await
        .map_err(|_| HistoryError::Unavailable)?;
    // shortcut: at most 10 batches of 20 for the hole; a longer absence reopens to the newest part.
    let mut fresh = events
        .events()
        .await
        .map_err(|_| HistoryError::Unavailable)?;
    for _ in 0..10 {
        if anchor.is_none() || fresh.iter().any(|e| e.event_id() == anchor) {
            break;
        }
        events
            .pagination()
            .run_backwards_until(20)
            .await
            .map_err(|_| HistoryError::Transport)?;
        fresh = events
            .events()
            .await
            .map_err(|_| HistoryError::Unavailable)?;
    }
    // The saved list keeps its order; only what follows its last message is added.
    let start = fresh
        .iter()
        .position(|e| e.event_id() == anchor)
        .map_or(0, |at| at + 1);
    rows.extend(fresh.into_iter().skip(start));
    let reactions = reaction_counts(&rows);
    let events = history_rows(client, &room, &id, rows.into_iter(), false).await?;
    Ok((crate::history::HistoryPage { tag, events, next }, reactions))
}

/// Answer one history request. An opened room that has saved messages emits them first,
/// then the full list with what was missed; a failed gap keeps the saved list. A room
/// never seen (nothing saved) takes the plain server page.
pub(crate) async fn serve_history(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: crate::history::HistoryRequest,
    out: &mut crate::Sender<crate::history::HistoryResult>,
) -> bool {
    let tag = request.tag;
    // False once the receiver is gone.
    let mut emit = async |result: crate::history::HistoryResult| {
        result.tag.is_cancelled() || out.send(result).await.is_ok()
    };
    if request.open
        && let Ok(saved) = saved_page(client, account, tag.clone()).await
        && !saved.events.is_empty()
    {
        if !emit(crate::history::HistoryResult {
            tag: tag.clone(),
            page: Ok(saved),
        })
        .await
        {
            return false;
        }
        return match open_page(client, tag.clone()).await {
            Ok((page, _)) => {
                emit(crate::history::HistoryResult {
                    tag,
                    page: Ok(page),
                })
                .await
            }
            Err(_) => true,
        };
    }
    let page = fetch_history(client, account, tag.clone(), request.before).await;
    emit(crate::history::HistoryResult { tag, page }).await
}

async fn history_sender(
    room: &matrix_sdk::room::Room,
    id: &matrix_sdk::ruma::UserId,
) -> crate::history::HistorySender {
    let member = room.get_member_no_sync(id).await.ok().flatten();
    crate::history::HistorySender::new(id.as_str(), member.as_ref().and_then(|m| m.display_name()))
        .with_avatar(
            member
                .as_ref()
                .and_then(|m| m.avatar_url())
                .map(|u| u.to_string()),
        )
}

async fn fetch_history(
    client: &matrix_sdk::Client,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
    before: Option<String>,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    use crate::history::HistoryError;
    use std::future::Future;
    let work = tokio::time::timeout(
        Duration::from_secs(15),
        load_history(client, account_id, tag.clone(), before),
    );
    let cancellation = async {
        loop {
            if tag.is_cancelled() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let mut work = std::pin::pin!(work);
    let mut cancellation = std::pin::pin!(cancellation);
    std::future::poll_fn(|cx| {
        if cancellation.as_mut().poll(cx).is_ready() {
            return std::task::Poll::Ready(Err(HistoryError::Cancelled));
        }
        work.as_mut().poll(cx).map(|result| {
            result
                .map_err(|_| HistoryError::Transport)
                .and_then(|result| result)
        })
    })
    .await
}

/// Message rows for events given oldest first. With `fetch_edits` an unedited-looking message
/// asks the server for its edits; without it only edits already inside the event are applied.
async fn history_rows(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::room::Room,
    id: &matrix_sdk::ruma::RoomId,
    chronological: impl Iterator<Item = matrix_sdk::deserialized_responses::TimelineEvent>,
    fetch_edits: bool,
) -> Result<Vec<crate::history::HistoryEvent>, crate::history::HistoryError> {
    use crate::history::{HistoryError, HistoryEvent};
    use matrix_sdk::ruma::events::{
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        room::message::MessageType,
    };
    let mut events = Vec::new();
    let chronological: Vec<_> = chronological.collect();
    let saved_edits = saved_edits(&chronological);
    let redacted = redacted_ids(&chronological);
    for event in chronological {
        // SDK owns decryption. A UTD remains a real row with server identity,
        // never silently omitted or rendered as ciphertext/plaintext.
        if event
            .raw()
            .get_field::<String>("type")
            .ok()
            .flatten()
            .as_deref()
            == Some("m.room.encrypted")
        {
            if let (Some(id), Ok(Some(sender)), Ok(Some(timestamp))) = (
                event.event_id(),
                event
                    .raw()
                    .get_field::<matrix_sdk::ruma::OwnedUserId>("sender"),
                event.raw().get_field::<u64>("origin_server_ts"),
            ) {
                events.push(HistoryEvent {
                    event_id: matrix_domain::EventId::new(id.as_str()),
                    body: if matches!(
                        event.raw().deserialize(),
                        Ok(AnySyncTimelineEvent::MessageLike(
                            AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Redacted(
                                _
                            ))
                        ))
                    ) {
                        "[Messaggio eliminato]"
                    } else {
                        "[Messaggio cifrato · chiavi non disponibili o decifratura fallita]"
                    }
                    .into(),
                    outgoing: client.user_id() == Some(sender.as_ref()),
                    timestamp_ms: timestamp,
                    sender: history_sender(room, &sender).await,
                    video: None,
                    styles: Vec::new(),
                });
            }
            continue;
        }
        if let Some(label) = event
            .raw()
            .get_field::<String>("type")
            .ok()
            .flatten()
            .and_then(|t| event_type_label(&t))
            && let (Some(id), Some(sender), Some(timestamp)) = (
                event.event_id(),
                event
                    .raw()
                    .get_field::<matrix_sdk::ruma::OwnedUserId>("sender")
                    .ok()
                    .flatten(),
                event
                    .raw()
                    .get_field::<u64>("origin_server_ts")
                    .ok()
                    .flatten(),
            )
        {
            events.push(HistoryEvent {
                event_id: matrix_domain::EventId::new(id.as_str()),
                body: if redacted.contains(id) {
                    "[Messaggio eliminato]"
                } else {
                    label
                }
                .into(),
                outgoing: client.user_id() == Some(sender.as_ref()),
                timestamp_ms: timestamp,
                sender: history_sender(room, &sender).await,
                video: None,
                styles: Vec::new(),
            });
            continue;
        }
        if let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            message,
        ))) = event.raw().deserialize()
        {
            let mut message = match message {
                SyncMessageLikeEvent::Original(message)
                    if !redacted.contains(&message.event_id) =>
                {
                    message
                }
                SyncMessageLikeEvent::Original(message) => {
                    events.push(HistoryEvent {
                        event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                        body: "[Messaggio eliminato]".into(),
                        outgoing: client.user_id() == Some(message.sender.as_ref()),
                        timestamp_ms: message.origin_server_ts.0.into(),
                        sender: history_sender(room, &message.sender).await,
                        video: None,
                        styles: Vec::new(),
                    });
                    continue;
                }
                SyncMessageLikeEvent::Redacted(message) => {
                    events.push(HistoryEvent {
                        event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                        body: "[Messaggio eliminato]".into(),
                        outgoing: client.user_id() == Some(message.sender.as_ref()),
                        timestamp_ms: message.origin_server_ts.0.into(),
                        sender: history_sender(room, &message.sender).await,
                        video: None,
                        styles: Vec::new(),
                    });
                    continue;
                }
            };
            use matrix_sdk::ruma::events::room::message::Relation;
            if matches!(message.content.relates_to, Some(Relation::Replacement(_))) {
                continue;
            }
            if let Some(edit) = message.unsigned.relations.replace.as_ref()
                && edit.sender == message.sender
                && let Some(Relation::Replacement(replacement)) = &edit.content.relates_to
                && replacement.event_id == message.event_id
                && replacement.new_content.msgtype.msgtype() == message.content.msgtype.msgtype()
            {
                message
                    .content
                    .apply_replacement(replacement.new_content.clone());
            } else if !fetch_edits
                && let Some(edit) = saved_edits.get(&message.event_id)
                && edit.sender == message.sender
                && let Some(Relation::Replacement(replacement)) = &edit.content.relates_to
                && replacement.new_content.msgtype.msgtype() == message.content.msgtype.msgtype()
            {
                message
                    .content
                    .apply_replacement(replacement.new_content.clone());
            } else if fetch_edits {
                use matrix_sdk::{
                    room::{IncludeRelations, RelationsOptions},
                    ruma::events::{
                        relation::RelationType, room::message::OriginalSyncRoomMessageEvent,
                    },
                };
                let mut latest: Option<OriginalSyncRoomMessageEvent> = None;
                let mut from = None;
                let mut cursors = std::collections::BTreeSet::new();
                loop {
                    let related = room
                        .relations(
                            message.event_id.clone(),
                            RelationsOptions {
                                from,
                                limit: Some(25u32.into()),
                                include_relations: IncludeRelations::RelationsOfType(
                                    RelationType::Replacement,
                                ),
                                ..Default::default()
                            },
                        )
                        .await
                        .map_err(|_| HistoryError::Transport)?;
                    for edit in related.chunk {
                        // Decrypted SDK events may omit room_id; reject an explicit foreign scope.
                        if !edit
                            .raw()
                            .get_field::<matrix_sdk::ruma::OwnedRoomId>("room_id")
                            .is_ok_and(|room_id| room_id.is_none_or(|room_id| room_id == id))
                        {
                            continue;
                        }
                        if let Ok(AnySyncTimelineEvent::MessageLike(
                            AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(
                                edit,
                            )),
                        )) = edit.raw().deserialize()
                            && edit.sender == message.sender
                            && let Some(Relation::Replacement(replacement)) =
                                &edit.content.relates_to
                            && replacement.event_id == message.event_id
                            && replacement.new_content.msgtype.msgtype()
                                == message.content.msgtype.msgtype()
                            && latest.as_ref().is_none_or(|previous| {
                                (edit.origin_server_ts, &edit.event_id)
                                    > (previous.origin_server_ts, &previous.event_id)
                            })
                        {
                            latest = Some(edit);
                        }
                    }
                    let Some(next) = related.next_batch_token else {
                        break;
                    };
                    if !cursors.insert(next.clone()) {
                        return Err(HistoryError::Transport);
                    }
                    from = Some(next);
                }
                if let Some(edit) = latest
                    && let Some(Relation::Replacement(replacement)) = edit.content.relates_to
                {
                    message.content.apply_replacement(replacement.new_content);
                }
            }
            let video = match &message.content.msgtype {
                MessageType::Video(v) => Some(video_info(v)),
                MessageType::Audio(a) => Some(audio_info(a)),
                _ => None,
            };
            let (body, styles) = match message.content.msgtype {
                MessageType::Image(_) => (crate::history::IMAGE_BODY.into(), Vec::new()),
                other => message_rich(&other),
            };
            events.push(HistoryEvent {
                event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                body,
                outgoing: client.user_id() == Some(message.sender.as_ref()),
                timestamp_ms: message.origin_server_ts.0.into(),
                sender: history_sender(room, &message.sender).await,
                video,
                styles,
            });
        }
    }
    Ok(events)
}

fn video_info(
    v: &matrix_sdk::ruma::events::room::message::VideoMessageEventContent,
) -> crate::history::VideoInfo {
    let info = v.info.as_deref();
    crate::history::VideoInfo {
        duration_ms: info.and_then(|i| i.duration).map(|d| d.as_millis() as u64),
        size: info.and_then(|i| i.size).map(Into::into),
        thumbnail: info.and_then(|i| i.thumbnail_source.clone()),
    }
}

/// Audio and voice notes play through the video path: same facts, never a thumbnail.
fn audio_info(
    a: &matrix_sdk::ruma::events::room::message::AudioMessageEventContent,
) -> crate::history::VideoInfo {
    let info = a.info.as_deref();
    crate::history::VideoInfo {
        duration_ms: info.and_then(|i| i.duration).map(|d| d.as_millis() as u64),
        size: info.and_then(|i| i.size).map(Into::into),
        thumbnail: None,
    }
}

/// Row text and styles of a message: its sanitized HTML body when the sender gave one, else `message_label`.
fn message_rich(msgtype: &MessageType) -> (String, Vec<crate::rich_text::StyleSpan>) {
    use matrix_sdk::ruma::events::room::message::MessageFormat;
    let formatted = match msgtype {
        MessageType::Text(t) => t.formatted.as_ref(),
        MessageType::Notice(n) => n.formatted.as_ref(),
        _ => None,
    };
    if let Some(f) = formatted.filter(|f| f.format == MessageFormat::Html) {
        let (text, styles) = crate::rich_text::from_html(&f.body);
        if !text.is_empty() {
            return (text, styles);
        }
    }
    (message_label(msgtype), Vec::new())
}

/// Row text for a message type: its content when textual, otherwise a short real label.
fn message_label(msgtype: &MessageType) -> String {
    let join = |name: &str, parts: Vec<String>| {
        std::iter::once(name.to_owned())
            .chain(parts)
            .collect::<Vec<_>>()
            .join(" · ")
    };
    match msgtype {
        MessageType::Text(_) | MessageType::Notice(_) | MessageType::Emote(_)
            if msgtype.body().is_empty() =>
        {
            "Messaggio senza testo".into()
        }
        MessageType::Emote(_) => format!("* {}", msgtype.body()),
        MessageType::Text(_) | MessageType::Notice(_) => msgtype.body().to_owned(),
        MessageType::ServerNotice(n) => n.body.clone(),
        MessageType::Video(v) => {
            let info = v.info.as_deref();
            let duration = info
                .and_then(|i| i.duration)
                .map(|d| crate::history::duration_label(d.as_millis() as u64));
            let size = info
                .and_then(|i| i.size)
                .map(|s| crate::history::human_size(s.into()));
            join("Video", duration.into_iter().chain(size).collect())
        }
        MessageType::Audio(a) => {
            let duration = a
                .info
                .as_deref()
                .and_then(|i| i.duration)
                .map(|d| crate::history::duration_label(d.as_millis() as u64));
            join(
                if a.voice.is_some() { "Nota vocale" } else { "Audio" },
                duration.into_iter().collect(),
            )
        }
        MessageType::File(f) => {
            let size = f
                .info
                .as_deref()
                .and_then(|i| i.size)
                .map(|s| crate::history::human_size(s.into()));
            join(
                "File",
                std::iter::once(f.filename().to_owned())
                    .chain(size)
                    .collect(),
            )
        }
        MessageType::Location(_) => "Posizione".into(),
        MessageType::VerificationRequest(_) => "Richiesta di verifica".into(),
        MessageType::Image(_) => crate::history::IMAGE_BODY.into(),
        other => other.msgtype().to_owned(),
    }
}

/// Label for message-like events that are not `m.room.message` (sticker, poll).
fn event_type_label(event_type: &str) -> Option<&'static str> {
    match event_type {
        "m.sticker" => Some("Sticker"),
        "m.poll.start" | "org.matrix.msc3381.poll.start" => Some("Sondaggio"),
        _ => None,
    }
}

/// Targets of redaction events among saved events: the cache keeps the original and the redaction apart.
fn redacted_ids(
    events: &[matrix_sdk::deserialized_responses::TimelineEvent],
) -> std::collections::HashSet<matrix_sdk::ruma::OwnedEventId> {
    events
        .iter()
        .filter(|e| {
            e.raw()
                .get_field::<String>("type")
                .ok()
                .flatten()
                .as_deref()
                == Some("m.room.redaction")
        })
        .filter_map(|e| {
            // Older room versions carry the target at the top, newer ones in the content.
            e.raw().get_field("redacts").ok().flatten().or_else(|| {
                let content: serde_json::Value = e.raw().get_field("content").ok().flatten()?;
                content.get("redacts")?.as_str()?.parse().ok()
            })
        })
        .collect()
}

/// Latest edit of each message among saved events: the cache keeps edits as events of their own.
fn saved_edits(
    events: &[matrix_sdk::deserialized_responses::TimelineEvent],
) -> std::collections::HashMap<
    matrix_sdk::ruma::OwnedEventId,
    matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent,
> {
    use matrix_sdk::ruma::events::{
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        room::message::Relation,
    };
    let mut latest = std::collections::HashMap::new();
    for event in events {
        if let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(edit),
        ))) = event.raw().deserialize()
            && let Some(Relation::Replacement(replacement)) = &edit.content.relates_to
        {
            let target = replacement.event_id.clone();
            let newer = latest.get(&target).is_none_or(
                |p: &matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent| {
                    (edit.origin_server_ts, &edit.event_id) > (p.origin_server_ts, &p.event_id)
                },
            );
            if newer {
                latest.insert(target, edit);
            }
        }
    }
    latest
}

/// Live reactions among saved events (a redacted reaction no longer deserializes as one).
fn reaction_counts(
    events: &[matrix_sdk::deserialized_responses::TimelineEvent],
) -> Vec<crate::history::ReactionCount> {
    use matrix_sdk::ruma::events::{
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
    };
    let mut counts: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let redacted = redacted_ids(events);
    for event in events {
        if event
            .event_id()
            .is_some_and(|id| redacted.contains(id as &matrix_sdk::ruma::EventId))
        {
            continue;
        }
        if let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Reaction(
            SyncMessageLikeEvent::Original(reaction),
        ))) = event.raw().deserialize()
        {
            let note = reaction.content.relates_to;
            *counts
                .entry((note.event_id.to_string(), note.key))
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|((target, key), count)| crate::history::ReactionCount {
            target: matrix_domain::EventId::new(&target),
            key,
            count,
        })
        .collect()
}

async fn load_history(
    client: &matrix_sdk::Client,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
    before: Option<String>,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    use crate::history::{HistoryError, HistoryPage};
    use matrix_sdk::ruma::RoomId;
    if tag.is_cancelled() {
        return Err(HistoryError::Cancelled);
    }
    if tag.scope().account_id != account_id {
        return Err(HistoryError::WrongScope);
    }
    let id = RoomId::parse(tag.scope().room_id.as_str()).map_err(|_| HistoryError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == RoomState::Joined)
        .ok_or(HistoryError::Unavailable)?;
    if before.is_some() {
        let (older, next) = older_via_event_cache(client, &room, &id, before.as_deref()).await?;
        if tag.is_cancelled() {
            return Err(HistoryError::Cancelled);
        }
        let events = history_rows(client, &room, &id, older.into_iter(), true).await?;
        return Ok(HistoryPage { tag, events, next });
    }
    let mut options = matrix_sdk::room::MessagesOptions::backward();
    options.from = before.clone();
    options.limit = 25u32.into();
    let response = room
        .messages(options)
        .await
        .map_err(|_| HistoryError::Transport)?;
    if tag.is_cancelled() {
        return Err(HistoryError::Cancelled);
    }
    let events = history_rows(client, &room, &id, response.chunk.into_iter().rev(), true).await?;
    let next = response
        .end
        .filter(|end| Some(end) != before.as_ref() && end != &response.start);
    Ok(HistoryPage { tag, events, next })
}

/// One page older than the shown list: saved chunks first, then the server through the SDK
/// event cache, which also saves what it fetches. Oldest first, and the cursor to still older ones.
async fn older_via_event_cache(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::room::Room,
    id: &matrix_sdk::ruma::RoomId,
    before: Option<&str>,
) -> Result<
    (
        Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
        Option<String>,
    ),
    crate::history::HistoryError,
> {
    use crate::history::HistoryError;
    let saved = stored_events(client, id).await?.0;
    let shown: std::collections::HashSet<_> = saved.iter().filter_map(|e| e.event_id()).collect();
    let (events, _drop) = room
        .event_cache()
        .await
        .map_err(|_| HistoryError::Unavailable)?;
    // shortcut: at most 50 steps over saved chunks already shown, then the page comes from the server.
    let (mut older, mut reached_start) = (Vec::new(), false);
    for _ in 0..50 {
        let step = events
            .pagination()
            .run_backwards_once(25)
            .await
            .map_err(|_| HistoryError::Transport)?;
        reached_start = step.reached_start;
        older = step
            .events
            .into_iter()
            .filter(|e| e.event_id().is_none_or(|i| !shown.contains(&i)))
            .collect();
        if !older.is_empty() || reached_start {
            break;
        }
    }
    older.reverse();
    let next = if reached_start {
        None
    } else {
        stored_events(client, id).await?.2
    };
    Ok((older, next.filter(|n| Some(n.as_str()) != before)))
}

/// Saved events of a room oldest first, and whether a hole sits after some saved messages.
pub(crate) async fn stored_events(
    client: &matrix_sdk::Client,
    id: &matrix_sdk::ruma::RoomId,
) -> Result<
    (
        Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
        bool,
        Option<String>,
    ),
    crate::history::HistoryError,
> {
    use crate::history::HistoryError;
    use matrix_sdk::linked_chunk::{ChunkContent, LinkedChunkId};
    let lock = client
        .event_cache_store()
        .lock()
        .await
        .map_err(|_| HistoryError::Unavailable)?;
    let guard = match &lock {
        matrix_sdk::cross_process_lock::MappedCrossProcessLockState::Clean(g)
        | matrix_sdk::cross_process_lock::MappedCrossProcessLockState::Dirty(g) => g,
    };
    let mut chunks: std::collections::HashMap<_, _> = guard
        .load_all_chunks(LinkedChunkId::Room(id))
        .await
        .map_err(|_| HistoryError::Unavailable)?
        .into_iter()
        .map(|c| (c.identifier, c))
        .collect();
    let mut next = chunks
        .values()
        .find(|c| c.previous.is_none())
        .map(|c| c.identifier);
    let (mut cached, mut hole, mut older) = (Vec::new(), false, None);
    while let Some(chunk) = next.and_then(|n| chunks.remove(&n)) {
        next = chunk.next;
        match chunk.content {
            ChunkContent::Items(items) => cached.extend(items),
            ChunkContent::Gap(gap) if cached.is_empty() => older = Some(gap.token),
            ChunkContent::Gap(_) => hole = true,
        }
    }
    Ok((cached, hole, older))
}
