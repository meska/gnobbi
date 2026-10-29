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
        crate::Receiver<crate::sending::SendEvent>,
    )>,
    avatars: crate::avatar::AvatarSlot,
    send_control: crate::sending::ControlSlot,
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
struct EmptyControlOnDrop(crate::sending::ControlSlot);
impl Drop for EmptyControlOnDrop {
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
        let send_control = crate::sending::ControlSlot::default();
        let worker_control = send_control.clone();
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
                let _control = EmptyControlOnDrop(worker_control.clone());
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
                    if let Some(fetcher) = crate::avatar_cache::AvatarFetcher::new(
                        &sync.session,
                        tokio::runtime::Handle::current(),
                    ) && let Ok(mut slot) = worker_avatars.lock()
                    {
                        *slot = Some(Box::new(fetcher));
                    }
                    let video_dir = sync.session.store.as_ref().and_then(|s| {
                        let dir = s.video_dir();
                        match crate::fsx::create_private_dir(&dir) {
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
                    let timelines = Timelines::default();
                    let operation_timelines = timelines.clone();
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
                                            &operation_timelines,
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
                                    let upload_limit = tokio::time::timeout(
                                        std::time::Duration::from_secs(10),
                                        operation_client.load_or_fetch_max_upload_size(),
                                    )
                                    .await
                                    .ok()
                                    .and_then(Result::ok)
                                    .map(u64::from);
                                    OperationResult::Members { scope, members, upload_limit }
                                }
                                OperationCommand::MarkRoom {
                                    scope,
                                    read,
                                    public,
                                } => {
                                    let result = crate::operations::mark_room(
                                        &operation_client,
                                        &operation_timelines,
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
                                OperationCommand::CreateRoom { name, topic, public } => {
                                    let result = crate::operations::create_room(
                                        &operation_client,
                                        account,
                                        &name,
                                        topic.as_deref(),
                                        public,
                                    )
                                    .await;
                                    OperationResult::RoomStarted { result }
                                }
                                OperationCommand::SetMuted { scope, muted } => {
                                    let result = crate::notify::set_muted(&operation_client, account, &scope, muted).await;
                                    OperationResult::RoomMarked { scope, result }
                                }
                                OperationCommand::EditTags(edit) => OperationResult::TagsEdited {
                                    result: crate::room_tags::edit(&operation_client, &edit).await,
                                },
                                OperationCommand::Apply(request) => {
                                    let result = crate::operations::perform(
                                        &operation_client,
                                        &operation_timelines,
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
                    let queue_client = client.clone();
                    let outbox = crate::outbox::Outbox::new(
                        client.clone(),
                        account,
                        tokio::runtime::Handle::current(),
                    );
                    if let Ok(mut slot) = worker_control.lock() {
                        *slot = Some(std::sync::Arc::new(outbox.clone()));
                    }
                    let send_timelines = timelines.clone();
                    let send_stop = cancelled.clone();
                    let mut sending_task = tokio::spawn(async move {
                        crate::outbox::run(
                            &outbox,
                            &send_timelines,
                            &mut sends,
                            &mut send_results,
                            &send_stop,
                        )
                        .await
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
                            if !serve_history(
                                &client,
                                &timelines,
                                account,
                                request,
                                &mut history_sender,
                            )
                            .await
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
                    // The queue of an account that leaves must not keep sending in the background;
                    // what is still queued stays in the store and restarts with the account.
                    queue_client.send_queue().set_enabled(false).await;
                    if worker_logout.load(std::sync::atomic::Ordering::Acquire) {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(15), sync.session.logout())
                                .await;
                    }
                    // The outbox holds a client too: drop it here, inside the runtime (see below).
                    if let Ok(mut slot) = worker_control.lock() {
                        drop(slot.take());
                    }
                    drop(sync);
                    // deadpool-sqlite closes its pooled connections via spawn_blocking:
                    // the avatar fetcher holds the last client clone, and dropping it
                    // after the runtime panics with "no reactor running" and aborts.
                    if let Ok(mut slot) = worker_avatars.lock() {
                        drop(slot.take());
                    }
                    drained
                })
            })?;
        Ok((
            Self {
                media_route: Some((media_commands, media_events)),
                verification_route: Some((verification_commands, verification_events)),
                operation_route: Some((operation_commands, operation_events)),
                send_route: Some((send_commands, send_events)),
                send_control,
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

    /// Take the single UI owner route; bounded8 requests/events, no SDK types.
    pub fn take_send_route(
        &mut self,
    ) -> Option<(
        crate::Sender<crate::sending::SendRequest>,
        crate::Receiver<crate::sending::SendEvent>,
    )> {
        self.send_route.take()
    }

    /// Retry, cancel and edit of queued texts; filled once the session is up, empty again when
    /// the worker ends.
    pub fn send_control(&self) -> crate::sending::ControlSlot {
        self.send_control.clone()
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
    /// SDK space graph, created at the first projection.
    spaces: Option<matrix_sdk_ui::spaces::SpaceService>,
    /// Sliding sync sends no server counts: unread comes from the SDK's client-side counts.
    pub(crate) sliding_counts: bool,
    timelines: Timelines,
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
        crate::operations::details(
            client,
            &self.timelines,
            self.session.account_id(),
            scope,
            target,
        )
        .await
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
            &self.timelines,
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
        let account = self.session.account_id();
        let room = crate::sending::room_for(client, account, request)?;
        if request.attachment.is_some() {
            return crate::sending::send_text(client, account, request).await;
        }
        let timeline = self
            .timelines
            .get(&room)
            .await
            .map_err(|_| crate::sending::SendError::Unavailable)?;
        crate::sending::send_text_waiting(client, &room, &timeline, request).await
    }

    pub fn new(session: AccountSession) -> Self {
        Self {
            session,
            previous: BTreeMap::new(),
            timeline_heads: BTreeMap::new(),
            latest_watched: Default::default(),
            spaces: None,
            sliding_counts: false,
            timelines: Default::default(),
            operation_state: std::sync::Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
    }

    /// Projection over an already logged-in client (the sliding-sync adapter and its tests).
    pub fn from_client(account_id: matrix_domain::AccountId, client: matrix_sdk::Client) -> Self {
        Self::new(AccountSession {
            account_id,
            client: Some(client),
            store: None,
        })
    }

    pub(crate) fn account_id(&self) -> matrix_domain::AccountId {
        self.session.account_id()
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
        crate::operations::mark_room(client, &self.timelines, self.session.account_id(), scope, read, true).await
    }

    /// A thread: root from the room's own timeline (the thread's `hit_start` is not reliable,
    /// see THREADS.md) and the replies from a `TimelineFocus::Thread` timeline.
    pub async fn open_thread(
        &self,
        scope: &RoomScope,
        root: &matrix_domain::EventId,
        want: usize,
    ) -> Result<crate::threads::ThreadPage, crate::history::HistoryError> {
        let room = self.thread_room(scope)?;
        crate::threads::open(&self.timelines, &room, root, want).await
    }

    /// The room's threads, newest activity first (one `ThreadListService` page).
    pub async fn thread_list(
        &self,
        scope: &RoomScope,
    ) -> Result<Vec<crate::threads::ThreadSummary>, crate::history::HistoryError> {
        crate::threads::list(self.thread_room(scope)?).await
    }

    fn thread_room(&self, scope: &RoomScope) -> Result<matrix_sdk::Room, crate::history::HistoryError> {
        use crate::history::HistoryError;
        let client = self.session.client.as_ref().ok_or(HistoryError::Unavailable)?;
        if scope.account_id != self.session.account_id() {
            return Err(HistoryError::WrongScope);
        }
        let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| HistoryError::WrongScope)?;
        client.get_room(&id).filter(|r| r.state() == RoomState::Joined).ok_or(HistoryError::Unavailable)
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
        fetch_history(
            client,
            &self.timelines,
            self.session.account_id(),
            tag,
            before,
        )
        .await
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
        let opened = open_page(client, &self.timelines, tag).await?;
        if visibility.sees_last_message() && !opened.0.events.is_empty() {
            self.mark_read(client, &scope.room_id, visibility.public_receipts)
                .await;
        }
        Ok(opened)
    }

    /// Read receipt (public or private) and fully_read for the newest event through the SDK
    /// Timeline, which skips markers that would not move forward; best effort, retried on the next open.
    async fn mark_read(&self, client: &matrix_sdk::Client, room_id: &RoomId, public: bool) {
        use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
        let Ok(id) = matrix_sdk::ruma::RoomId::parse(room_id.as_str()) else {
            return;
        };
        let Some(room) = client.get_room(&id) else {
            return;
        };
        let Ok(timeline) = self.timelines.get(&room).await else {
            return;
        };
        let read = if public { ReceiptType::Read } else { ReceiptType::ReadPrivate };
        let _ = timeline.mark_as_read(read).await;
        let _ = timeline.mark_as_read(ReceiptType::FullyRead).await;
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
        let mut children_of = self.space_children(&client, &rooms).await;
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
            // Peer markers are drawn in 1:1 rooms only; the Timeline reads them from the SDK store and receipt events.
            let peer_receipts = if room.state() == RoomState::Joined && room.joined_members_count() <= 2 {
                match self.timelines.get(&room).await {
                    Ok(timeline) => {
                        let own = client.user_id().map(ToString::to_string).unwrap_or_default();
                        crate::receipts::peer_receipts(&timeline, &own).await
                    }
                    Err(_) => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let children = children_of.remove(&id).unwrap_or_default();
            let tags = match room.tags().await {
                Ok(Some(tags)) => crate::room_tags::group_names(&tags),
                _ => Vec::new(),
            };
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
                        public_read_receipts: peer_receipts,
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
                        tags,
                        muted: room.cached_user_defined_notification_mode()
                            == Some(matrix_sdk::notification_settings::RoomNotificationMode::Mute),
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
        self.previous = current;
        Ok(changes)
    }

    /// Direct children of each joined space, from the SDK space graph (parents are joined spaces
    /// only, cycles removed); the graph is rebuilt by `top_level_joined_spaces`.
    async fn space_children(
        &mut self,
        client: &matrix_sdk::Client,
        rooms: &[matrix_sdk::Room],
    ) -> BTreeMap<String, Vec<RoomId>> {
        let service = match &self.spaces {
            Some(service) => service,
            None => self.spaces.insert(matrix_sdk_ui::spaces::SpaceService::new(client.clone()).await),
        };
        service.top_level_joined_spaces().await;
        let mut children: BTreeMap<String, Vec<RoomId>> = BTreeMap::new();
        for room in rooms {
            for parent in service.joined_parent_ids_of_child(room.room_id()).await {
                children.entry(parent.to_string()).or_default().push(RoomId::new(room.room_id().as_str()));
            }
        }
        children.values_mut().for_each(|v| v.sort_by(|a, b| a.as_str().cmp(b.as_str())));
        children
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

pub(crate) async fn until_stopped<F: std::future::Future>(
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

/// Rows per history page.
pub(crate) const PAGE: usize = 25;

/// One SDK `Timeline` per room the user opened: the SDK owns edits, reactions, redactions,
/// replies and decryption; the engine only translates its items to `HistoryEvent`.
// shortcut: timelines are never evicted before the account closes, upgrade when memory shows it.
#[derive(Clone, Default)]
pub(crate) struct Timelines(
    std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<TimelineKey, std::sync::Arc<matrix_sdk_ui::timeline::Timeline>>>>,
);

/// A room's own timeline (no root) or one of its threads.
type TimelineKey = (matrix_sdk::ruma::OwnedRoomId, Option<matrix_sdk::ruma::OwnedEventId>);

impl Timelines {
    pub(crate) async fn get(
        &self,
        room: &matrix_sdk::Room,
    ) -> Result<std::sync::Arc<matrix_sdk_ui::timeline::Timeline>, crate::history::HistoryError>
    {
        self.focused(room, None).await
    }

    /// The timeline of one thread (same cache and same SDK `Timeline` type as the room's).
    pub(crate) async fn thread(
        &self,
        room: &matrix_sdk::Room,
        root: &matrix_sdk::ruma::EventId,
    ) -> Result<std::sync::Arc<matrix_sdk_ui::timeline::Timeline>, crate::history::HistoryError>
    {
        self.focused(room, Some(root.to_owned())).await
    }

    async fn focused(
        &self,
        room: &matrix_sdk::Room,
        root: Option<matrix_sdk::ruma::OwnedEventId>,
    ) -> Result<std::sync::Arc<matrix_sdk_ui::timeline::Timeline>, crate::history::HistoryError>
    {
        use matrix_sdk_ui::timeline::{RoomExt, TimelineFocus};
        let mut all = self.0.lock().await;
        let key = (room.room_id().to_owned(), root.clone());
        if let Some(timeline) = all.get(&key) {
            return Ok(timeline.clone());
        }
        let builder = room.timeline_builder();
        let builder = match root {
            Some(root_event_id) => builder.with_focus(TimelineFocus::Thread { root_event_id }),
            None => builder,
        };
        let timeline = std::sync::Arc::new(
            builder
                .build()
                .await
                .map_err(|_| crate::history::HistoryError::Unavailable)?,
        );
        all.insert(key, timeline.clone());
        Ok(timeline)
    }
}

/// What a `Timeline` holds now, oldest first: rows, live reaction counts, and whether the
/// start of the room was reached (the SDK adds a virtual start item then).
pub(crate) struct Loaded {
    pub(crate) rows: Vec<crate::history::HistoryEvent>,
    pub(crate) reactions: Vec<crate::history::ReactionCount>,
    pub(crate) at_start: bool,
}

/// Translate the items. Never downloads: media rows keep only the sources of the event.
pub(crate) async fn loaded(room: &matrix_sdk::Room, timeline: &matrix_sdk_ui::timeline::Timeline) -> Loaded {
    use crate::history::{HistoryEvent, ReactionCount};
    use matrix_sdk_ui::timeline::{MsgLikeKind, VirtualTimelineItem};
    let mut out = Loaded {
        rows: Vec::new(),
        reactions: Vec::new(),
        at_start: false,
    };
    for item in timeline.items().await.iter() {
        if let Some(VirtualTimelineItem::TimelineStart) = item.as_virtual() {
            out.at_start = true;
        }
        // Local echoes have no event ID yet (skipped below): the send path owns them.
        let Some(event) = item.as_event() else {
            continue;
        };
        let Some(id) = event.event_id() else {
            continue;
        };
        let Some(content) = event.content().as_msglike() else {
            continue;
        };
        let (mut video, mut styles) = (None, Vec::new());
        let body = match &content.kind {
            MsgLikeKind::Message(message) => match message.msgtype() {
                MessageType::Image(_) => crate::history::IMAGE_BODY.into(),
                msgtype => {
                    video = match msgtype {
                        MessageType::Video(v) => Some(video_info(v)),
                        MessageType::Audio(a) => Some(audio_info(a)),
                        _ => None,
                    };
                    let (body, spans) = message_rich(msgtype);
                    styles = spans;
                    body
                }
            },
            MsgLikeKind::Sticker(_) => "Sticker".into(),
            MsgLikeKind::Poll(_) => "Sondaggio".into(),
            MsgLikeKind::Redacted => "[Messaggio eliminato]".into(),
            // A UTD stays a real row with server identity, never ciphertext or silence.
            MsgLikeKind::UnableToDecrypt(_) => {
                "[Messaggio cifrato · chiavi non disponibili o decifratura fallita]".into()
            }
            _ => continue,
        };
        out.reactions.extend(
            content
                .reactions
                .iter()
                .map(|(key, senders)| ReactionCount {
                    target: matrix_domain::EventId::new(id.as_str()),
                    key: key.clone(),
                    count: senders.len(),
                }),
        );
        out.rows.push(HistoryEvent {
            event_id: matrix_domain::EventId::new(id.as_str()),
            body,
            outgoing: event.is_own(),
            timestamp_ms: event.timestamp().0.into(),
            sender: history_sender(room, event.sender()).await,
            video,
            styles,
        });
    }
    out
}

/// Grow the timeline backwards until at least `want` rows are loaded or the room starts.
/// Saved chunks come first, then the server; a failed server step still returns what the
/// saved chunks added.
pub(crate) async fn fill(
    room: &matrix_sdk::Room,
    timeline: &matrix_sdk_ui::timeline::Timeline,
    want: usize,
) -> (Loaded, Result<(), crate::history::HistoryError>) {
    // shortcut: at most 10 steps of one page for a long absence, then the newest part is shown.
    let mut now = loaded(room, timeline).await;
    for _ in 0..10 {
        if now.rows.len() >= want || now.at_start {
            return (now, Ok(()));
        }
        let before = now.rows.len();
        let Ok(reached_start) = timeline.paginate_backwards(PAGE as u16).await else {
            return (
                loaded(room, timeline).await,
                Err(crate::history::HistoryError::Transport),
            );
        };
        // The timeline applies the cache updates in its own task: wait for the rows to show up.
        // Reaching the start adds its marker at once, so only a row count change counts.
        // shortcut: polling with a deadline, upgrade to the timeline's diff stream if it flickers.
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(if reached_start { 400 } else { 3000 });
        loop {
            now = loaded(room, timeline).await;
            if now.rows.len() != before || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    (now, Ok(()))
}

/// The cursor is the event ID of the oldest row sent: a stable anchor for the scroll position,
/// still opaque to the UI. None once nothing older exists.
fn cursor(rows: &[crate::history::HistoryEvent], at_start: bool) -> Option<String> {
    rows.first()
        .filter(|_| !at_start)
        .map(|e| e.event_id.as_str().to_owned())
}

fn joined(
    client: &matrix_sdk::Client,
    tag: &crate::RequestTag,
    account_id: matrix_domain::AccountId,
) -> Result<matrix_sdk::Room, crate::history::HistoryError> {
    use crate::history::HistoryError;
    if tag.scope().account_id != account_id {
        return Err(HistoryError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
        .map_err(|_| HistoryError::WrongScope)?;
    client
        .get_room(&id)
        .filter(|r| r.state() == RoomState::Joined)
        .ok_or(HistoryError::Unavailable)
}

/// Saved messages of one room, as the room's Timeline holds them. Never touches the network.
async fn saved_page(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    let room = joined(client, &tag, account_id)?;
    let timeline = timelines.get(&room).await?;
    let now = loaded(&room, &timeline).await;
    let next = cursor(&now.rows, now.at_start);
    Ok(crate::history::HistoryPage {
        tag,
        events: now.rows,
        next,
    })
}

/// Everything the Timeline holds, after the SDK filled what sync left missing (a first
/// page when it holds less than one).
async fn open_page(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    tag: crate::RequestTag,
) -> Result<
    (
        crate::history::HistoryPage,
        Vec<crate::history::ReactionCount>,
    ),
    crate::history::HistoryError,
> {
    let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
        .map_err(|_| crate::history::HistoryError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == RoomState::Joined)
        .ok_or(crate::history::HistoryError::Unavailable)?;
    let timeline = timelines.get(&room).await?;
    // Best effort: a failed gap keeps what is saved.
    let (now, _) = fill(&room, &timeline, PAGE).await;
    let next = cursor(&now.rows, now.at_start);
    Ok((
        crate::history::HistoryPage {
            tag,
            events: now.rows,
            next,
        },
        now.reactions,
    ))
}

/// Answer one history request. An opened room that has saved messages emits them first,
/// then the full list with what was missed; a failed gap keeps the saved list. A room
/// never seen (nothing saved) takes the plain server page.
pub(crate) async fn serve_history(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
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
        && let Ok(saved) = saved_page(client, timelines, account, tag.clone()).await
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
        return match open_page(client, timelines, tag.clone()).await {
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
    let page = fetch_history(client, timelines, account, tag.clone(), request.before).await;
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
    timelines: &Timelines,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
    before: Option<String>,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    use crate::history::HistoryError;
    use std::future::Future;
    let work = tokio::time::timeout(
        Duration::from_secs(15),
        load_history(client, timelines, account_id, tag.clone(), before),
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
                if a.voice.is_some() {
                    "Nota vocale"
                } else {
                    "Audio"
                },
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

/// One page: the newest, or the one just older than the row `before` (the cursor of the
/// previous page). Oldest first. The Timeline pages the server only when it holds too little.
async fn load_history(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
    before: Option<String>,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    use crate::history::{HistoryError, HistoryPage};
    if tag.is_cancelled() {
        return Err(HistoryError::Cancelled);
    }
    let room = joined(client, &tag, account_id)?;
    let timeline = timelines.get(&room).await?;
    let (mut now, step) = fill(&room, &timeline, PAGE).await;
    step?;
    if let Some(anchor) = before {
        let at = |rows: &[crate::history::HistoryEvent]| {
            rows.iter().position(|e| e.event_id.as_str() == anchor)
        };
        let mut shown = at(&now.rows).ok_or(HistoryError::Unavailable)?;
        if shown < PAGE && !now.at_start {
            let step;
            (now, step) = fill(&room, &timeline, now.rows.len() + PAGE).await;
            step?;
            shown = at(&now.rows).ok_or(HistoryError::Unavailable)?;
        }
        now.rows.truncate(shown);
    }
    if tag.is_cancelled() {
        return Err(HistoryError::Cancelled);
    }
    let from = now.rows.len().saturating_sub(PAGE);
    let events: Vec<_> = now.rows.drain(from..).collect();
    // Rows left over older than this page still count as "more" even at the start.
    let next = cursor(&events, now.at_start && from == 0);
    Ok(HistoryPage { tag, events, next })
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
