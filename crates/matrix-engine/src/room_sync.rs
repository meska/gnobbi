//! Backend-only Matrix /sync room projection. Owns one authenticated session.
use crate::{RoomListDiff, RoomSummary, auth::AccountSession};
use matrix_domain::{RoomId, RoomScope};
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
                                    let key = crate::media::MediaKey {
                                        scope: tag.scope().clone(),
                                        event_id: event.clone(),
                                    };
                                    let read = async {
                                        if let Some(bytes) = cache.get(&key).map(<[u8]>::to_vec) {
                                            let preview = crate::media_sdk::decode_png(
                                                bytes.clone(),
                                                crate::media_sdk::mime_of(&bytes)
                                                    .ok_or(crate::media::MediaError::InvalidImage)?,
                                                &tag,
                                            )
                                            .await?;
                                            Ok(crate::media::DownloadedPng { bytes, preview })
                                        } else {
                                            let downloaded = crate::media_sdk::download_png(
                                                &media_client,
                                                account,
                                                &tag,
                                                &event,
                                            )
                                            .await?;
                                            cache.insert(key, downloaded.bytes.clone());
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
                    // Runtime owns this read-only task; shutdown drops/cancels it.
                    let history_task = tokio::spawn(async move {
                        while let Some(request) = requests.recv().await {
                            let tag = request.tag;
                            if tag.is_cancelled() {
                                continue;
                            }
                            let page =
                                fetch_history(&client, account, tag.clone(), request.before).await;
                            if !tag.is_cancelled()
                                && history_sender
                                    .send(crate::history::HistoryResult { tag, page })
                                    .await
                                    .is_err()
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
    previews: BTreeMap<String, (String, String)>,
    receipts: crate::receipts::PeerReceipts,
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
        use crate::history::HistoryError;
        let client = self.session.client.as_ref().ok_or(HistoryError::Unavailable)?;
        if tag.scope().account_id != self.session.account_id() {
            return Err(HistoryError::WrongScope);
        }
        let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
            .map_err(|_| HistoryError::WrongScope)?;
        let room = client
            .get_room(&id)
            .filter(|r| r.state() == RoomState::Joined)
            .ok_or(HistoryError::Unavailable)?;
        let (cached, _) = stored_events(client, &id).await?;
        let events = history_rows(client, &room, &id, cached.into_iter(), false).await?;
        Ok(crate::history::HistoryPage { tag, events, next: None })
    }

    /// Open a room: the sync cursor brings the messages missed since the last run, and if
    /// that left a hole after the saved ones the SDK paginates just that hole.
    pub async fn open_history(
        &self,
        tag: crate::RequestTag,
    ) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
        use crate::history::HistoryError;
        let client = self.session.client.as_ref().ok_or(HistoryError::Unavailable)?;
        let id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
            .map_err(|_| HistoryError::WrongScope)?;
        let room = client.get_room(&id).filter(|r| r.state() == RoomState::Joined).ok_or(HistoryError::Unavailable)?;
        let (mut rows, _) = stored_events(client, &id).await?;
        let anchor = rows.last().and_then(|e| e.event_id());
        // Subscribed before the sync so the cache applies the missed messages as they arrive.
        let (events, _drop) = room.event_cache().await.map_err(|_| HistoryError::Unavailable)?;
        client
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
            .map_err(|_| HistoryError::Transport)?;
        // shortcut: at most 10 batches of 20 for the hole; a longer absence reopens to the newest part.
        let mut fresh = events.events().await.map_err(|_| HistoryError::Unavailable)?;
        for _ in 0..10 {
            if anchor.is_none() || fresh.iter().any(|e| e.event_id() == anchor) {
                break;
            }
            events.pagination().run_backwards_until(20).await.map_err(|_| HistoryError::Transport)?;
            fresh = events.events().await.map_err(|_| HistoryError::Unavailable)?;
        }
        // The saved list keeps its order; only what follows its last message is added.
        let start = fresh.iter().position(|e| e.event_id() == anchor).map_or(0, |at| at + 1);
        rows.extend(fresh.into_iter().skip(start));
        let events = history_rows(client, &room, &id, rows.into_iter(), false).await?;
        Ok(crate::history::HistoryPage { tag, events, next: None })
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
                match preview_change(event.raw().json().get()) {
                    PreviewChange::Set { event_id, text } => {
                        self.previews.insert(room.to_string(), (event_id, text));
                    }
                    PreviewChange::Remove { target } => {
                        if self.previews.get(room.as_str()).is_some_and(|(id, _)| *id == target) {
                            self.previews.remove(room.as_str());
                        }
                    }
                    PreviewChange::Ignore => {}
                }
            }
            for raw in update.ephemeral {
                self.receipts.observe_event(&scope, &raw);
            }
        }
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
                        preview: self.previews.get(&id).map(|(_, text)| text.clone()),
                        topic: room.topic(),
                        joined_members: room.joined_members_count(),
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
        let activity = self.timeline_activity(heads);
        Ok((changes, activity))
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
    Set { event_id: String, text: String },
    Remove { target: String },
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
            .map_or(PreviewChange::Ignore, |target| PreviewChange::Remove { target }),
        Some("m.room.message")
            if event["unsigned"].get("redacted_because").is_none()
                && content["m.relates_to"]["rel_type"] != "m.replace" =>
        {
            let (Some(event_id), Some(body)) = (text(event.get("event_id")), text(content.get("body")))
            else {
                return PreviewChange::Ignore;
            };
            let text = if content["msgtype"] == "m.image" { "Immagine".to_owned() } else { body };
            PreviewChange::Set { event_id, text }
        }
        _ => PreviewChange::Ignore,
    }
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
                    sender: history_sender(&room, &sender).await,
                });
            }
            continue;
        }
        if let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            message,
        ))) = event.raw().deserialize()
        {
            let mut message = match message {
                SyncMessageLikeEvent::Original(message) => message,
                SyncMessageLikeEvent::Redacted(message) => {
                    events.push(HistoryEvent {
                        event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                        body: "[Messaggio eliminato]".into(),
                        outgoing: client.user_id() == Some(message.sender.as_ref()),
                        timestamp_ms: message.origin_server_ts.0.into(),
                        sender: history_sender(&room, &message.sender).await,
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
            let body = match message.content.msgtype {
                MessageType::Text(text) => text.body,
                MessageType::Notice(text) => text.body,
                MessageType::Emote(text) => format!("* {}", text.body),
                MessageType::Image(_) => crate::history::IMAGE_BODY.into(),
                _ => "[Messaggio non testuale · supporto non disponibile]".into(),
            };
            events.push(HistoryEvent {
                event_id: matrix_domain::EventId::new(message.event_id.as_str()),
                body,
                outgoing: client.user_id() == Some(message.sender.as_ref()),
                timestamp_ms: message.origin_server_ts.0.into(),
                sender: history_sender(&room, &message.sender).await,
            });
        }
    }
    Ok(events)
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

/// Saved events of a room oldest first, and whether a hole sits after some saved messages.
async fn stored_events(
    client: &matrix_sdk::Client,
    id: &matrix_sdk::ruma::RoomId,
) -> Result<(Vec<matrix_sdk::deserialized_responses::TimelineEvent>, bool), crate::history::HistoryError> {
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
    let mut next = chunks.values().find(|c| c.previous.is_none()).map(|c| c.identifier);
    let (mut cached, mut hole) = (Vec::new(), false);
    while let Some(chunk) = next.and_then(|n| chunks.remove(&n)) {
        next = chunk.next;
        match chunk.content {
            ChunkContent::Items(items) => cached.extend(items),
            ChunkContent::Gap(_) => hole |= !cached.is_empty(),
        }
    }
    Ok((cached, hole))
}
