//! Backend-only Matrix /sync room projection. Owns one authenticated session.
use crate::{RoomListDiff, RoomSummary, auth::AccountSession};
use matrix_domain::{RoomId, RoomScope};
use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::{RoomState, config::SyncSettings, ruma::presence::PresenceState};
use std::{collections::BTreeMap, time::Duration};

pub use crate::{RoomListEvent, RoomSyncStatus};

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
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<(), ShutdownError>>>,
    history_route: Option<(
        crate::Sender<crate::history::HistoryRequest>,
        crate::Receiver<crate::history::HistoryResult>,
    )>,
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
        let (media_commands, mut media_requests) =
            crate::bounded::<crate::media::MediaCommand>(std::num::NonZeroUsize::new(2).unwrap());
        let (mut media_results, media_events) =
            crate::bounded(std::num::NonZeroUsize::new(2).unwrap());
        let thread = std::thread::Builder::new()
            .name("matrix-room-sync".into())
            .spawn(move || {
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
                                MediaCommand::Download { tag, event } => {
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
                                    let hit = [Thumbnail, Original]
                                        .into_iter()
                                        .find_map(|v| cache.get(&key(v)).map(|b| (v, b.to_vec())));
                                    let read = async {
                                        if let Some((variant, bytes)) = hit {
                                            let preview = crate::media_sdk::decode_png(
                                                bytes.clone(),
                                                crate::media_sdk::mime_of(&bytes).ok_or(
                                                    crate::media::MediaError::InvalidImage,
                                                )?,
                                                &tag,
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
                        let mut ready = false;
                        use std::hash::{Hash, Hasher};
                        let mut seed = std::collections::hash_map::DefaultHasher::new();
                        crate::sending::fresh_transaction_id()
                            .as_str()
                            .hash(&mut seed);
                        let mut backoff = crate::reconnect::Backoff::new(seed.finish());
                        if let Ok(saved) = sync.saved_batch().await {
                            for change in saved {
                                if sender.send(RoomListEvent::Diff(change)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        loop {
                            let (changes, activity) = match sync.next_batch_with_activity().await {
                                Ok(changes) => changes,
                                Err(SyncError::LoggedOut) => {
                                    let _ = sender
                                        .send(RoomListEvent::Status(
                                            RoomSyncStatus::AuthenticationFailed,
                                        ))
                                        .await;
                                    return;
                                }
                                Err(_) => {
                                    let _ = sender
                                        .send(RoomListEvent::Status(RoomSyncStatus::SyncFailed))
                                        .await;
                                    ready = false;
                                    // Sends keep their transaction identity; reconnect only /sync.
                                    tokio::time::sleep(backoff.next_delay()).await;
                                    continue;
                                }
                            };
                            backoff.reset();
                            for change in changes {
                                if sender.send(RoomListEvent::Diff(change)).await.is_err() {
                                    return;
                                }
                            }
                            for scope in activity {
                                if sender
                                    .send(RoomListEvent::TimelineActivity(scope))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            if !ready {
                                if sender
                                    .send(RoomListEvent::Status(RoomSyncStatus::Ready))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                ready = true;
                                // Runtime drop ends it with the account; one pass after the first sync.
                                let (client, opened, stop) =
                                    (warm_client.clone(), warm_opened.clone(), warm_stop.clone());
                                tokio::spawn(async move {
                                    crate::warm::warm(&client, &stop, &opened).await;
                                });
                            }
                        }
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
                stop,
                thread: Some(thread),
                history_route: Some((history_commands, history_events)),
            },
            receiver,
        ))
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
    /// Last real message per room as (event id, text); fed by sync only, so empty until a message arrives.
    previews: BTreeMap<String, (String, String, Option<u64>)>,
    receipts: crate::receipts::PeerReceipts,
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
            previews: BTreeMap::new(),
            receipts,
            sent_markers: Default::default(),
            operation_state: std::sync::Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
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
            self.mark_read(client, &scope.room_id, &last.event_id, visibility.public_receipts)
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

    /// Await only on the backend runtime. SDK persists/reuses its sync cursor;
    /// unchanged summaries are not emitted again. Payload-free errors only.
    pub async fn next_batch(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        self.next_batch_with_activity()
            .await
            .map(|(diffs, _)| diffs)
    }

    async fn next_batch_with_activity(
        &mut self,
    ) -> Result<(Vec<RoomListDiff>, Vec<RoomScope>), SyncError> {
        let client = self.session.client.as_ref().ok_or(SyncError::LoggedOut)?;
        use matrix_sdk::ruma::api::client::{
            filter::{FilterDefinition, LazyLoadOptions},
            sync::sync_events::v3::Filter,
        };
        let mut filter = FilterDefinition::default();
        filter.room.state.lazy_load_options = LazyLoadOptions::Enabled {
            include_redundant_members: false,
        };
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            client.sync_once(
                SyncSettings::default()
                    .filter(Filter::FilterDefinition(filter))
                    .timeout(Duration::from_millis(200))
                    .set_presence(PresenceState::Offline),
            ),
        )
        .await
        .map_err(|_| SyncError::Transport)?
        .map_err(|error| {
            if matches!(
                error.client_api_error_kind(),
                Some(matrix_sdk::ruma::api::error::ErrorKind::UnknownToken { .. })
            ) {
                SyncError::LoggedOut
            } else {
                SyncError::Transport
            }
        })?;
        let mut heads = Vec::new();
        for (room, update) in response.rooms.joined {
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
            for event in &update.timeline.events {
                apply_preview(
                    &mut self.previews,
                    room.as_str(),
                    preview_change(event.raw().json().get()),
                );
            }
            for raw in update.ephemeral {
                self.receipts.observe_event(&scope, &raw);
            }
        }
        let activity_heads = heads;
        let changes = self.summarize().await?;
        let activity = self.timeline_activity(activity_heads);
        Ok((changes, activity))
    }

    /// Joined rooms already in the SDK store, previews from the saved messages. No network.
    pub async fn saved_batch(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        let client = self
            .session
            .client
            .as_ref()
            .ok_or(SyncError::LoggedOut)?
            .clone();
        for room in client
            .rooms()
            .into_iter()
            .filter(|r| r.state() == RoomState::Joined)
        {
            let Ok((saved, _, _)) = stored_events(&client, room.room_id()).await else {
                continue;
            };
            for event in &saved {
                apply_preview(
                    &mut self.previews,
                    room.room_id().as_str(),
                    preview_change(event.raw().json().get()),
                );
            }
        }
        self.summarize().await
    }

    async fn summarize(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        let client = self.session.client.as_ref().ok_or(SyncError::LoggedOut)?;
        let mut current = BTreeMap::new();
        for room in client
            .rooms()
            .into_iter()
            .filter(|room| room.state() == RoomState::Joined)
        {
            let id = room.room_id().as_str().to_owned();
            let mut children = Vec::new();
            if room.is_space() {
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
                    unread: room.unread_notification_counts().notification_count,
                    info: crate::RoomInfo {
                        public_read_receipts: self.receipts.snapshot(&RoomScope {
                            account_id: self.session.account_id(),
                            room_id: RoomId::new(id.clone()),
                        }),
                        preview: self.previews.get(&id).map(|(_, text, _)| text.clone()),
                        last_activity: last_activity(&self.previews, &id),
                        topic: room.topic(),
                        joined_members: room.joined_members_count(),
                        mentions: room.unread_notification_counts().highlight_count,
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

    fn timeline_activity(
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

/// What one timeline event means for the room-list preview.
#[derive(Debug, PartialEq, Eq)]
enum PreviewChange {
    Set {
        event_id: String,
        text: String,
        ts: Option<u64>,
    },
    Remove {
        target: String,
    },
    Ignore,
}

fn preview_change(raw: &str) -> PreviewChange {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(raw) else {
        return PreviewChange::Ignore;
    };
    let text = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).map(str::to_owned);
    let content = &event["content"];
    match event["type"].as_str() {
        Some("m.room.redaction") => text(event.get("redacts").or(content.get("redacts")))
            .map_or(PreviewChange::Ignore, |target| PreviewChange::Remove {
                target,
            }),
        Some("m.room.message")
            if event["unsigned"].get("redacted_because").is_none()
                && content["m.relates_to"]["rel_type"] != "m.replace" =>
        {
            let (Some(event_id), Some(body)) =
                (text(event.get("event_id")), text(content.get("body")))
            else {
                return PreviewChange::Ignore;
            };
            let text = if content["msgtype"] == "m.image" {
                "Immagine".to_owned()
            } else {
                body
            };
            PreviewChange::Set {
                event_id,
                text,
                ts: event["origin_server_ts"].as_u64(),
            }
        }
        _ => PreviewChange::Ignore,
    }
}

type Previews = BTreeMap<String, (String, String, Option<u64>)>;

fn apply_preview(previews: &mut Previews, room: &str, change: PreviewChange) {
    match change {
        PreviewChange::Set { event_id, text, ts } => {
            previews.insert(room.to_owned(), (event_id, text, ts));
        }
        PreviewChange::Remove { target } => {
            if previews.get(room).is_some_and(|(id, ..)| *id == target) {
                previews.remove(room);
            }
        }
        PreviewChange::Ignore => {}
    }
}

/// Server time (ms) of the last real message seen. The SDK recency stamp is a bump counter, not a
/// time, and is absent without sliding sync; this one is the same on Synapse and Tuwunel.
fn last_activity(previews: &Previews, room: &str) -> Option<u64> {
    previews.get(room).and_then(|(.., ts)| *ts)
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
    // Subscribed before the sync so the cache applies the missed messages as they arrive.
    let (events, _drop) = room
        .event_cache()
        .await
        .map_err(|_| HistoryError::Unavailable)?;
    client
        .sync_once(matrix_sdk::config::SyncSettings::default())
        .await
        .map_err(|_| HistoryError::Transport)?;
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
                _ => None,
            };
            let body = match message.content.msgtype {
                MessageType::Image(_) => crate::history::IMAGE_BODY.into(),
                other => message_label(&other),
            };
            events.push(HistoryEvent {
                event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                body,
                outgoing: client.user_id() == Some(message.sender.as_ref()),
                timestamp_ms: message.origin_server_ts.0.into(),
                sender: history_sender(room, &message.sender).await,
                video,
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

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
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
            let duration = info.and_then(|i| i.duration).map(|d| {
                let s = d.as_secs();
                format!("{}:{:02}", s / 60, s % 60)
            });
            let size = info.and_then(|i| i.size).map(|s| human_size(s.into()));
            join("Video", duration.into_iter().chain(size).collect())
        }
        MessageType::Audio(a) if a.voice.is_some() => "Nota vocale".into(),
        MessageType::Audio(_) => "Audio".into(),
        MessageType::File(f) => {
            let size = f
                .info
                .as_deref()
                .and_then(|i| i.size)
                .map(|s| human_size(s.into()));
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
