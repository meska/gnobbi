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
    revoked: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
        let revoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (worker_logout, worker_done, worker_revoked) = (logout.clone(), done.clone(), revoked.clone());
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
            // Closing the app drops the room-list controller, whose waiter chain (async-cell, SDK room list) is dropped by recursion:
            // with the 2 MiB default the thread overflowed its stack and aborted at quit on macOS. Memory is reserved, not committed.
            // shortcut: a bigger stack, not a fix of the chain; replace when the SDK drops it iteratively.
            .stack_size(256 << 20)
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
                    let push_task = tokio::spawn(crate::push::keep_registered(
                        client.clone(),
                        crate::auth::DEVICE_NAME,
                        sync.session.store.as_ref().and_then(|s| s.path.parent()).map(std::path::Path::to_path_buf),
                        account,
                    ));
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
                        // Saves run in their own tasks (a big file must not hold the thumbnails) and report here.
                        let (saved_tx, mut saved_rx) = tokio::sync::mpsc::unbounded_channel();
                        while let Some(command) = next_media_command(
                            &media_stop,
                            &mut media_requests,
                            &mut saved_rx,
                            &mut media_results,
                        )
                        .await
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
                                MediaCommand::OpenAudio { tag, event } => {
                                    if tag.is_cancelled()
                                        || media_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    let read = async {
                                        // Decoded in memory: the same cap images already use.
                                        crate::video_file::download_audio(
                                            &media_client,
                                            account,
                                            &tag,
                                            &event,
                                            crate::media::MAX_MEDIA_BYTES,
                                        )
                                        .await
                                    };
                                    let Some(result) = until_stopped(&media_stop, read).await
                                    else {
                                        continue;
                                    };
                                    MediaEvent::AudioOpened { tag, event, result }
                                }
                                MediaCommand::Save { tag, event, path } => {
                                    if !tag.is_cancelled() {
                                        spawn_save(&media_client, account, &media_stop, &saved_tx, tag, event, path);
                                    }
                                    continue;
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
                        sync.session.store.as_ref().map(|s| s.path.clone()),
                    ));
                    let operation_client = client.clone();
                    let timelines = Timelines::default();
                    let operation_timelines = timelines.clone();
                    let operation_state = sync.operation_state.clone();
                    let operation_stop = cancelled.clone();
                    let mut operations_task = tokio::spawn(async move {
                        let mut clean = true;
                        use crate::operations::{OperationCommand, OperationResult};
                        let mut search = crate::user_search::UserSearch::for_client(operation_client.clone());
                        let mut search_rx = search.as_ref().map(|s| s.subscribe());
                        // Message search of the open room: one instance, replaced when the room changes.
                        let mut messages: Option<(RoomScope, crate::message_search::MessageSearch)> = None;
                        let mut messages_rx: Option<tokio::sync::watch::Receiver<crate::operations::MessageSearchState>> = None;
                        let rooms_client = operation_client.clone();
                        let mut rooms = crate::search::Search::<(Option<String>, Option<crate::operations::DirectoryKind>), crate::operations::DirectoryRoom>::new(
                            (None, None),
                            move |term, (server, kind), since| {
                                let client = rooms_client.clone();
                                async move {
                                    crate::operations::public_rooms(&client, &term, server.as_deref(), since, crate::search::PAGE, kind).await
                                }
                            },
                        );
                        let mut rooms_rx = rooms.subscribe();
                        // One receipt in flight per account, polled beside the commands: a page never waits for it.
                        type InFlightReceipt = Option<(
                            crate::RequestTag,
                            std::pin::Pin<Box<dyn std::future::Future<Output = OperationResult> + Send>>,
                        )>;
                        let mut receipt: InFlightReceipt = None;
                        // Joins in flight (key = command target) and their `Slow` notices; dropped with the loop on stop.
                        let mut joins = tokio::task::JoinSet::<OperationResult>::new();
                        let mut joining = std::collections::HashSet::<String>::new();
                        let (join_progress_tx, mut join_progress) = tokio::sync::mpsc::unbounded_channel::<OperationResult>();
                        loop {
                            let command = tokio::select! {
                                command = shutdown_receive(&operation_stop, &mut operations) => {
                                    match command {
                                        Some(command) => command,
                                        None => break,
                                    }
                                }
                                Some(()) = async {
                                    match &mut search_rx {
                                        Some(rx) => rx.changed().await.ok(),
                                        None => std::future::pending().await,
                                    }
                                } => {
                                    let state = search_rx.as_mut().unwrap().borrow_and_update().clone();
                                    let all_available = search.as_ref().is_some_and(|s| s.all_available());
                                    let sent = until_stopped(
                                        &operation_stop,
                                        operation_results.send(OperationResult::UserSearch { state, all_available }),
                                    )
                                    .await;
                                    clean &= matches!(sent, Some(Ok(())) | None);
                                    continue;
                                }
                                Some(()) = async {
                                    match &mut messages_rx {
                                        Some(rx) => rx.changed().await.ok(),
                                        None => std::future::pending().await,
                                    }
                                } => {
                                    let state = messages_rx.as_mut().unwrap().borrow_and_update().clone();
                                    if let Some((scope, _)) = &messages {
                                        let sent = until_stopped(
                                            &operation_stop,
                                            operation_results.send(OperationResult::MessageSearch { scope: scope.clone(), state }),
                                        )
                                        .await;
                                        clean &= matches!(sent, Some(Ok(())) | None);
                                    }
                                    continue;
                                }
                                Ok(()) = rooms_rx.changed() => {
                                    let state = rooms_rx.borrow_and_update().clone();
                                    let sent = until_stopped(&operation_stop, operation_results.send(OperationResult::RoomSearch { state })).await;
                                    clean &= matches!(sent, Some(Ok(())) | None);
                                    continue;
                                }
                                Some(progress) = join_progress.recv() => {
                                    let _ = until_stopped(&operation_stop, operation_results.send(progress)).await;
                                    continue;
                                }
                                Some(Ok(done)) = joins.join_next() => {
                                    if let OperationResult::Join { target, .. } = &done {
                                        joining.remove(target);
                                    }
                                    let _ = until_stopped(&operation_stop, operation_results.send(done)).await;
                                    continue;
                                }
                                Some(done) = async {
                                    match &mut receipt {
                                        Some((_, future)) => Some(future.as_mut().await),
                                        None => std::future::pending().await,
                                    }
                                } => {
                                    // The slot is freed before the result goes out: a revoked receipt is not retried.
                                    receipt = None;
                                    let _ = until_stopped(&operation_stop, operation_results.send(done)).await;
                                    continue;
                                }
                            };
                            // A revoked slot is dropped at the next command: no retry, no orphan.
                            if receipt.as_ref().is_some_and(|(tag, _)| tag.is_cancelled()) {
                                receipt = None;
                            }
                            let result = match command {
                                OperationCommand::SearchMessages { scope, term } => {
                                    if messages.as_ref().is_none_or(|(s, _)| *s != scope) {
                                        // A scope of another account or an unparsable room closes the search.
                                        messages = (scope.account_id == account)
                                            .then(|| matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).ok())
                                            .flatten()
                                            .map(|id| (scope.clone(), crate::message_search::MessageSearch::for_room(operation_client.clone(), id)));
                                        messages_rx = messages.as_ref().map(|(_, m)| m.subscribe());
                                    }
                                    if let Some((_, m)) = &mut messages {
                                        m.set_query(&term);
                                    }
                                    continue;
                                }
                                OperationCommand::MoreMessages => {
                                    if let Some((_, m)) = &mut messages {
                                        m.more();
                                    }
                                    continue;
                                }
                                OperationCommand::SearchUsers { term, all } => {
                                    if let Some(search) = &mut search {
                                        search.set_filter(if all {
                                            crate::user_search::Filter::All
                                        } else {
                                            crate::user_search::Filter::SameInstance
                                        });
                                        search.set_term(&term);
                                    }
                                    continue;
                                }
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
                                OperationCommand::Profile { user_id } => {
                                    let result = crate::operations::user_profile(
                                        &operation_client,
                                        &user_id,
                                    )
                                    .await;
                                    OperationResult::Profile { user_id, result }
                                }
                                OperationCommand::Ignore { user_id, ignore } => {
                                    let result = crate::operations::set_ignored(&operation_client, &user_id, ignore).await;
                                    OperationResult::Ignored { user_id, ignore, result }
                                }
                                OperationCommand::MemberRights { scope, user_id } => {
                                    let result = crate::operations::load_member_rights(
                                        &operation_client,
                                        &operation_timelines,
                                        account,
                                        &scope,
                                        &user_id,
                                    )
                                    .await;
                                    OperationResult::MemberRights { scope, user_id, result }
                                }
                                OperationCommand::Moderate { scope, user_id, action } => {
                                    let result = crate::operations::moderate(
                                        &operation_client,
                                        account,
                                        &scope,
                                        &user_id,
                                        &action,
                                    )
                                    .await;
                                    OperationResult::Moderated { scope, user_id, action, result }
                                }
                                OperationCommand::BannedMembers { scope } => {
                                    let result = crate::operations::banned_members(&operation_client, account, &scope).await;
                                    OperationResult::BannedMembers { scope, result }
                                }
                                OperationCommand::MemberPermissions { scope } => {
                                    let result = crate::operations::member_permissions(&operation_client, account, &scope).await;
                                    OperationResult::MemberPermissions { scope, result }
                                }
                                OperationCommand::MarkRoom {
                                    scope,
                                    read,
                                    public,
                                    main_only,
                                } => {
                                    let result = crate::operations::mark_room(
                                        &operation_client,
                                        &operation_timelines,
                                        account,
                                        &scope,
                                        read,
                                        public,
                                        main_only,
                                    )
                                    .await;
                                    OperationResult::RoomMarked { scope, result }
                                }
                                OperationCommand::ReadMarkers { scope } => {
                                    let markers = crate::read_progress::markers(&operation_client, &operation_timelines, account, &scope).await;
                                    OperationResult::ReadMarkers { scope, markers }
                                }
                                OperationCommand::MarkReadTo { scope, target, public } => {
                                    let result = crate::read_progress::advance(&operation_client, &operation_timelines, account, &scope, &target, public).await;
                                    OperationResult::ReadAdvanced { scope, result }
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
                                OperationCommand::EditRoom { scope, name, topic, avatar } => {
                                    let result = crate::operations::edit_room(
                                        &operation_client,
                                        account,
                                        &scope,
                                        name.as_deref(),
                                        topic.as_deref(),
                                        avatar.as_deref(),
                                    )
                                    .await;
                                    OperationResult::RoomEdited { result }
                                }
                                OperationCommand::Aliases { scope, action } => {
                                    let (result, aliases) =
                                        crate::alias_ops::alias_action(&operation_client, account, &scope, &action).await;
                                    OperationResult::Aliases { scope, action, result, aliases }
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
                                OperationCommand::SpaceChoices { scope } => {
                                    let result = crate::operations::space_choices(&operation_client, account, &scope).await;
                                    OperationResult::SpaceChoices { scope, result }
                                }
                                OperationCommand::SpaceChildren { scope } => {
                                    let result = crate::operations::space_children(&operation_client, account, &scope).await;
                                    OperationResult::SpaceChildren { scope, result }
                                }
                                OperationCommand::AddToSpace { scope, space } => {
                                    let result = crate::operations::add_to_space(&operation_client, account, &scope, &space).await;
                                    OperationResult::SpaceChanged { scope, space, added: true, result }
                                }
                                OperationCommand::RemoveFromSpace { scope, space } => {
                                    let result = crate::operations::remove_from_space(&operation_client, account, &scope, &space).await;
                                    OperationResult::SpaceChanged { scope, space, added: false, result }
                                }
                                OperationCommand::PlaceInSpace { scope, from, to, before } => {
                                    let result = crate::operations::place_in_space(&operation_client, account, &scope, from.as_ref(), to.as_ref(), before.as_ref()).await;
                                    OperationResult::SpacePlaced { scope, result }
                                }
                                OperationCommand::CreateSpace { name, topic, public, avatar } => {
                                    let result = crate::operations::create_space(
                                        &operation_client,
                                        account,
                                        &name,
                                        topic.as_deref(),
                                        public,
                                        avatar.as_ref(),
                                    )
                                    .await;
                                    OperationResult::SpaceCreated { result }
                                }
                                OperationCommand::JoinRoom { target, via } => {
                                    // Not awaited here: a slow join must not stall search and the other commands.
                                    // A second request for a target already in flight is a no-op.
                                    if !joining.insert(target.clone()) {
                                        continue;
                                    }
                                    let client = operation_client.clone();
                                    let progress = join_progress_tx.clone();
                                    joins.spawn(async move {
                                        let slow = target.clone();
                                        let phase = crate::join::join_tracked(&client, account, &target, &via, |phase| {
                                            let _ = progress.send(OperationResult::Join { target: slow.clone(), phase });
                                        })
                                        .await;
                                        OperationResult::Join { target, phase }
                                    });
                                    continue;
                                }
                                OperationCommand::SearchRooms { term, server, kind } => {
                                    // Spaces and rooms list their first page for an empty term.
                                    let min = if kind.is_some() && term.trim().is_empty() { 0 } else { crate::search::MIN_CHARS };
                                    rooms.set_query_from(&term, (server, kind), min);
                                    continue;
                                }
                                OperationCommand::MoreRooms => {
                                    rooms.more();
                                    continue;
                                }
                                OperationCommand::ProbeServer { server } => {
                                    let result = tokio::time::timeout(
                                        crate::search::TIMEOUT,
                                        async {
                                            let result = crate::operations::public_rooms(&operation_client, "", Some(&server), None, 1, None).await.map(|_| ());
                                            if result == Err(crate::search::SearchError::Unreachable) {
                                                let discovery = crate::discovery::Discovery::with_http(operation_client.http_client().clone());
                                                return match discovery.probe_federation(&server).await {
                                                    Ok(()) => Err(crate::search::SearchError::OurServerBlocked),
                                                    Err(e @ (crate::search::SearchError::NotMatrix | crate::search::SearchError::Tls)) => Err(e),
                                                    Err(_) => result,
                                                };
                                            }
                                            result
                                        },
                                    )
                                    .await
                                    .unwrap_or(Err(crate::search::SearchError::Timeout))
                                    .map(|_| ());
                                    OperationResult::ServerProbed { server, result }
                                }
                                OperationCommand::LeaveRoom { scope } => {
                                    let result = crate::operations::leave_room(
                                        &operation_client,
                                        account,
                                        &scope,
                                    )
                                    .await;
                                    OperationResult::RoomLeft { scope, result }
                                }
                                OperationCommand::LeaveSpace { scope, forget } => {
                                    let (is_last_owner, result) =
                                        crate::operations::leave_space(&operation_client, account, &scope, forget).await;
                                    OperationResult::SpaceLeft { scope, is_last_owner, result }
                                }
                                OperationCommand::DeleteSpace { scope } => {
                                    let result = crate::operations::delete_space(&operation_client, account, &scope).await;
                                    OperationResult::SpaceDeleted { scope, result }
                                }
                                OperationCommand::AnswerInvite { scope, accept } => {
                                    let result = crate::operations::answer_invite(
                                        &operation_client,
                                        account,
                                        &scope,
                                        accept,
                                    )
                                    .await;
                                    OperationResult::InviteAnswered { scope, accept, result }
                                }
                                OperationCommand::Thread { tag, root, want } => {
                                    if tag.is_cancelled()
                                        || operation_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    let result = crate::threads::open_scoped(
                                        &operation_client,
                                        &operation_timelines,
                                        account,
                                        tag.scope(),
                                        &root,
                                        want,
                                    )
                                    .await;
                                    OperationResult::Thread { tag, root, result }
                                }
                                OperationCommand::ThreadRead { tag, root, target, public } => {
                                    if tag.is_cancelled()
                                        || operation_stop.load(std::sync::atomic::Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    // A duplicate intent coalesces; a new one drops the old future first.
                                    if receipt.as_ref().is_some_and(|(active, _)| active.is_same(&tag)) {
                                        continue;
                                    }
                                    let client = operation_client.clone();
                                    let timelines = operation_timelines.clone();
                                    let guard = tag.clone();
                                    receipt = Some((
                                        tag,
                                        Box::pin(async move {
                                            let result = crate::threads::mark_read_target(
                                                &client, &timelines, account, &guard, &root, &target, public,
                                            )
                                            .await;
                                            OperationResult::ThreadRead { tag: guard, root, target, result }
                                        }),
                                    ));
                                    continue;
                                }
                                OperationCommand::StickerPacks { scope } => {
                                    let result = crate::stickers::packs(&operation_client, account, &scope).await;
                                    OperationResult::StickerPacks { scope, result }
                                }
                                OperationCommand::SendSticker { scope, sticker } => {
                                    let result = crate::stickers::send(&operation_client, account, &scope, &sticker).await;
                                    OperationResult::StickerSent { scope, result }
                                }
                                OperationCommand::StickerEdit { edit } => {
                                    OperationResult::StickerEdited { result: crate::stickers::edit(&operation_client, edit).await }
                                }
                                OperationCommand::ThreadSend { scope, root, reply_to, body, attachment } => {
                                    let result = crate::threads::send_scoped(&operation_client, &operation_timelines, account, &scope, &root, reply_to.as_ref(), &body, attachment.as_ref()).await;
                                    OperationResult::ThreadSent { scope, root, result }
                                }
                                OperationCommand::SetNotificationMode { scope, mode } => {
                                    let result = crate::notify::set_notification_mode(&operation_client, account, &scope, mode).await;
                                    OperationResult::NotificationModeSet { scope, result }
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
                    push_task.abort();
                    if worker_logout.load(std::sync::atomic::Ordering::Acquire) {
                        crate::push::remove(&queue_client).await;
                        let out =
                            tokio::time::timeout(Duration::from_secs(15), sync.session.logout())
                                .await;
                        worker_revoked.store(matches!(out, Ok(Ok(()))), std::sync::atomic::Ordering::Release);
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
                revoked,
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
        WorkerControl::new(self.stop.clone(), self.logout.clone(), self.done.clone(), self.revoked.clone())
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

    /// Fixture: owned handles so a test can act as the app does while `sliding::run` borrows `self`.
    #[cfg(test)]
    pub(crate) fn live_handles_for_test(&self) -> (matrix_sdk::Client, Timelines) {
        (self.session.client.clone().expect("store-backed session"), self.timelines.clone())
    }

    #[cfg(test)]
    pub(crate) async fn mark_room_for_test(
        &self,
        scope: &RoomScope,
        read: bool,
    ) -> Result<(), crate::operations::OperationError> {
        let client = self.session.client.as_ref().expect("store-backed session");
        crate::operations::mark_room(client, &self.timelines, self.session.account_id(), scope, read, true, false).await
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

    /// Read receipt of one thread (its root as `thread_id`).
    pub async fn mark_thread_read(
        &self,
        scope: &RoomScope,
        root: &matrix_domain::EventId,
        public: bool,
    ) -> Result<bool, crate::operations::OperationError> {
        let client = self.session.client.as_ref().ok_or(crate::operations::OperationError::Unavailable)?;
        crate::threads::mark_read_scoped(client, &self.timelines, self.session.account_id(), scope, root, public).await
    }

    /// Read receipt `main` for the room's own timeline (the same call the open view makes).
    pub async fn mark_main_read(&self, scope: &RoomScope, public: bool) -> Result<bool, crate::operations::OperationError> {
        use crate::operations::OperationError as E;
        let client = self.session.client.as_ref().ok_or(E::Unavailable)?;
        let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| E::WrongScope)?;
        let room = client.get_room(&id).ok_or(E::Unavailable)?;
        let timeline = self.timelines.get(&room).await.map_err(|_| E::Unavailable)?;
        crate::threads::mark_main(&room, &timeline, public).await.map_err(|_| E::Transport)
    }

    /// A text into a thread (plain, or a reply to one of its events); waits for the server.
    pub async fn send_in_thread(
        &self,
        scope: &RoomScope,
        root: &matrix_domain::EventId,
        reply_to: Option<&matrix_domain::EventId>,
        body: &str,
        attachment: Option<&crate::sending::Attachment>,
    ) -> Result<matrix_domain::EventId, crate::operations::OperationError> {
        let client = self.session.client.as_ref().ok_or(crate::operations::OperationError::Unavailable)?;
        crate::threads::send_scoped(client, &self.timelines, self.session.account_id(), scope, root, reply_to, body, attachment).await
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
        let _ = crate::threads::mark_main(&room, &timeline, public).await;
        let _ = timeline.mark_as_read(ReceiptType::FullyRead).await;
    }

    /// Fixture: runs the live `sliding::run` until `Ready` and a quiet moment; returns what the
    /// room list changed compared to the previous call (the run itself empties and refills it
    /// while it starts, so the net difference is what a caller can rely on).
    #[cfg(test)]
    pub(crate) async fn sliding_round(&mut self) -> Result<Vec<RoomListDiff>, SyncError> {
        self.sliding_round_after(async {}).await
    }

    /// Like `sliding_round`, but `action` runs once the run is `Ready`: what it changes on the
    /// server reaches the room list as a live delivery, as in the app (sliding sync only counts
    /// unread from deliveries it sees, so messages sent before the run show as one).
    #[cfg(test)]
    pub(crate) async fn sliding_round_after(
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
            .map(|(_, room)| RoomListDiff::Upsert(Box::new(room.clone())))
            .collect();
        diffs.extend(
            before
                .iter()
                .filter(|(id, _)| !self.previous.contains_key(*id))
                .map(|(_, room)| RoomListDiff::Remove(room.scope.clone())),
        );
        Ok(diffs)
    }

    /// Fixture: one sliding round for a bare client, the only way tests catch a client up.
    #[cfg(test)]
    pub(crate) async fn sliding_catch_up(client: &matrix_sdk::Client) -> Result<(), SyncError> {
        Self::from_client(matrix_domain::AccountId::new(0), client.clone()).sliding_round().await.map(drop)
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
            // Peer markers are drawn in small rooms only (cost: every item is scanned); the Timeline reads them from the SDK store and receipt events.
            let peer_receipts = if room.state() == RoomState::Joined && room.joined_members_count() <= crate::receipts::MAX_RECEIPT_ROOM {
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
            let latest = matrix_sdk_ui::timeline::RoomExt::latest_event(&room).await;
            let children = children_of.remove(&id).unwrap_or_default();
            let (tags, favourite_order) = match room.tags().await {
                Ok(Some(tags)) => (crate::room_tags::group_names(&tags), crate::room_tags::favourite_order(&tags)),
                _ => (Vec::new(), None),
            };
            // Unknown levels (not synced yet) or no own user id: no rights.
            let (edit_rights, child_rights) = match (room.power_levels().await, client.user_id()) {
                (Ok(levels), Some(me)) if room.state() == RoomState::Joined => (
                    crate::EditRights::from_levels(&levels, me),
                    room.is_space() && levels.user_can_send_state(me, matrix_sdk::ruma::events::StateEventType::SpaceChild),
                ),
                _ => (crate::EditRights::default(), false),
            };
            current.insert(
                id.clone(),
                RoomSummary {
                    scope: RoomScope {
                        account_id: self.session.account_id(),
                        room_id: RoomId::new(id.clone()),
                    },
                    name: room_label(room.display_name().await.ok(), &id),
                    unread: room.num_unread_notifications(),
                    info: crate::RoomInfo {
                        edit_rights,
                        child_rights,
                        public_read_receipts: peer_receipts,
                        preview: latest_event_text(&latest),
                        last_event_ts: latest_event_ts(&latest),
                        invited: room.state() == RoomState::Invited,
                        topic: room.topic(),
                        topic_view: room_topic_view(&room).await,
                        avatar: room_avatar(&room).await,
                        marked_unread: room.is_marked_unread(),
                        joined_members: room.joined_members_count(),
                        mentions: room.num_unread_mentions(),
                        is_space: room.is_space(),
                        children,
                        encrypted: (!room.encryption_state().is_unknown())
                            .then(|| room.encryption_state().is_encrypted()),
                        tags,
                        favourite_order,
                        notification_mode: crate::notify::mode_from_sdk(room.cached_user_defined_notification_mode()),
                        aliases: room
                            .canonical_alias()
                            .into_iter()
                            .chain(room.alt_aliases())
                            .map(|a| a.to_string())
                            .collect(),
                        dm_user_id: room.direct_targets().iter().map(|u| u.to_string()).min(),
                    },
                },
            );
        }
        let mut changes: Vec<_> = current
            .iter()
            .filter(|(id, room)| self.previous.get(*id) != Some(*room))
            .map(|(_, room)| RoomListDiff::Upsert(Box::new(room.clone())))
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
        // The order of the space (`order`, age, room id); a child the state does not list yet goes last, by id.
        for (parent, list) in &mut children {
            let ranked = match rooms.iter().find(|r| r.room_id().as_str() == parent) {
                Some(space) => crate::operations::ordered_children(space).await,
                None => Vec::new(),
            };
            let rank = |id: &RoomId| ranked.iter().position(|(c, _)| c.as_str() == id.as_str()).unwrap_or(usize::MAX);
            list.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.as_str().cmp(b.as_str())));
        }
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
fn latest_event_text(latest: &matrix_sdk_ui::timeline::LatestEventValue) -> Option<String> {
    use matrix_sdk_ui::timeline::LatestEventValue;
    let (LatestEventValue::Remote { content, .. } | LatestEventValue::Local { content, .. }) = latest
    else {
        return None;
    };
    if let Some(poll) = content.as_poll() {
        return Some(poll.results().question);
    }
    let message = content.as_message()?;
    Some(match message.msgtype() {
        MessageType::Image(_) => "Image".to_owned(),
        _ => message.body().to_owned(),
    })
}

/// Timestamp (ms since epoch) of the latest *message*; None for no event, a non-message event (state, redaction)
/// or an invite (its stamp is the local receipt time, not the event's), so the row never shows a time without a message.
pub(crate) fn latest_event_ts(latest: &matrix_sdk_ui::timeline::LatestEventValue) -> Option<u64> {
    use matrix_sdk_ui::timeline::LatestEventValue::{Local, Remote};
    match latest {
        Remote { timestamp, content, .. } | Local { timestamp, content, .. } if content.as_message().is_some() => {
            Some(timestamp.0.into())
        }
        _ => None,
    }
}

/// Name shown in the room list: the SDK-computed display name; «Stanza vuota» for an unnamed room with
/// nobody else in it (SDK `Empty`), the raw id only as a last resort.
fn room_label(name: Option<matrix_sdk::RoomDisplayName>, id: &str) -> String {
    use matrix_sdk::RoomDisplayName as N;
    match name {
        Some(N::Named(n) | N::Aliased(n) | N::Calculated(n) | N::EmptyWas(n)) if !n.is_empty() => n,
        Some(N::Empty) => "Stanza vuota".to_owned(),
        _ => id.to_owned(),
    }
}

/// The topic rendered for the details panel: `m.topic` text/html if the state event has it, else the plain text as markdown.
async fn room_topic_view(room: &matrix_sdk::Room) -> Option<crate::rich_text::Topic> {
    let plain = room.topic().filter(|t| !t.trim().is_empty())?;
    // Read from the local state store; invites carry stripped state and the plain text only.
    let event = room
        .get_state_event_static::<matrix_sdk::ruma::events::room::topic::RoomTopicEventContent>()
        .await
        .ok()
        .flatten()
        .and_then(|e| serde_json::to_value(e).ok());
    let html = event
        .as_ref()
        .and_then(|e| e.pointer("/content/m.topic/m.text"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find(|r| r.get("mimetype").and_then(|m| m.as_str()) == Some("text/html")))
        .and_then(|r| r.get("body"))
        .and_then(|b| b.as_str());
    Some(crate::rich_text::topic(&plain, html))
}

/// Avatar to show: the room's own, else for a 1:1 DM the other person's (as Element X does).
async fn room_avatar(room: &matrix_sdk::Room) -> Option<String> {
    // The sliding-sync avatar may be server-derived, so read the actual state event.
    if let Ok(Some(event)) = room
        .get_state_event_static::<matrix_sdk::ruma::events::room::avatar::RoomAvatarEventContent>()
        .await
    {
        let own = serde_json::to_value(event).ok();
        if let Some(url) = own
            .as_ref()
            .and_then(|e| e.pointer("/content/url"))
            .and_then(|v| v.as_str())
            .filter(|url| !url.is_empty())
        {
            return Some(url.to_owned());
        }
    }
    let heroes = room.heroes().await;
    let [hero] = heroes.as_slice() else {
        return None;
    };
    if !room.is_direct().await.unwrap_or(false) {
        return None;
    }
    if let Some(a) = nonempty(hero.avatar_url.as_deref()) {
        return Some(a);
    }
    let member = room.get_member_no_sync(&hero.user_id).await.ok().flatten()?;
    nonempty(member.avatar_url())
}

/// An empty mxc string means "no avatar".
fn nonempty(uri: Option<&matrix_sdk::ruma::MxcUri>) -> Option<String> {
    uri.filter(|u| !u.as_str().is_empty()).map(|u| u.to_string())
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

/// One save in its own task; its answer goes to `done` (see [`next_media_command`]).
fn spawn_save(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: &tokio::sync::mpsc::UnboundedSender<crate::media::MediaEvent>,
    tag: crate::RequestTag,
    event: matrix_domain::EventId,
    path: std::path::PathBuf,
) {
    let (client, stop, done) = (client.clone(), stop.clone(), done.clone());
    tokio::spawn(async move {
        let save = crate::video_file::save_attachment(
            &client,
            account,
            &tag,
            &event,
            &path,
            crate::media::MAX_SAVE_BYTES,
        );
        if let Some(result) = until_stopped(&stop, save).await {
            let _ = done.send(crate::media::MediaEvent::Saved { tag, event, result });
        }
    });
}

/// The next media command; a finished save is forwarded to the UI while waiting for it.
async fn next_media_command(
    stop: &std::sync::atomic::AtomicBool,
    requests: &mut crate::Receiver<crate::media::MediaCommand>,
    saved: &mut tokio::sync::mpsc::UnboundedReceiver<crate::media::MediaEvent>,
    results: &mut crate::Sender<crate::media::MediaEvent>,
) -> Option<crate::media::MediaCommand> {
    loop {
        tokio::select! {
            Some(done) = saved.recv() => {
                let _ = until_stopped(stop, results.send(done)).await;
            }
            command = shutdown_receive(stop, requests) => return command,
        }
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
            // Replies live in the thread panel only; the root stays with its summary.
            None => builder.with_focus(TimelineFocus::Live { hide_threaded_events: true }),
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

async fn thread_info(
    room: &matrix_sdk::Room,
    content: &matrix_sdk_ui::timeline::MsgLikeContent,
) -> Option<crate::history::ThreadInfo> {
    use matrix_sdk_ui::timeline::TimelineDetails;
    let summary = content.thread_summary.as_ref()?;
    let latest_sender = match &summary.latest_event {
        TimelineDetails::Ready(e) => Some(history_sender(room, &e.sender).await),
        _ => None,
    };
    Some(crate::history::ThreadInfo { num_replies: summary.num_replies, latest_sender, unread: 0 })
}

/// The rows of `loaded`, for the disposable-server tests in `tests/`.
#[doc(hidden)]
pub async fn rows_for_test(room: &matrix_sdk::Room, timeline: &std::sync::Arc<matrix_sdk_ui::timeline::Timeline>) -> Vec<crate::history::HistoryEvent> {
    loaded(room, timeline).await.rows
}

/// One line for an undecryptable row; the cause (public variant fields only) picks between
/// «keys may still arrive» and «will never be recoverable». Never includes message content.
fn utd_text(message: &matrix_sdk_ui::timeline::EncryptedMessage) -> &'static str {
    match message {
        matrix_sdk_ui::timeline::EncryptedMessage::MegolmV1AesSha2 { cause, .. } => utd_cause_text(*cause),
        _ => UTD_DEFAULT,
    }
}
const UTD_DEFAULT: &str = "Cifrato · chiavi non disponibili";
fn utd_cause_text(cause: matrix_sdk_crypto::types::events::UtdCause) -> &'static str {
    use matrix_sdk_crypto::types::events::UtdCause::*;
    match cause {
        HistoricalMessageAndDeviceIsUnverified => "Cifrato · chiavi non ancora arrivate o recuperabili",
        SentBeforeWeJoined | HistoricalMessageAndBackupIsDisabled => "Cifrato · inviato prima della tua verifica, non recuperabile",
        _ => UTD_DEFAULT,
    }
}

/// Translate the items. Never downloads: media rows keep only the sources of the event.
pub(crate) async fn loaded(room: &matrix_sdk::Room, timeline: &std::sync::Arc<matrix_sdk_ui::timeline::Timeline>) -> Loaded {
    use crate::history::{HistoryEvent, ReactionCount};
    use matrix_sdk_ui::timeline::{MsgLikeKind, VirtualTimelineItem};
    let mut out = Loaded {
        rows: Vec::new(),
        reactions: Vec::new(),
        at_start: false,
    };
    let items = timeline.items().await;
    // The originals of replies that are already loaded (what a click on the quote scrolls to).
    let by_id: std::collections::HashMap<&str, &matrix_sdk_ui::timeline::EventTimelineItem> =
        items.iter().filter_map(|i| i.as_event()).filter_map(|e| Some((e.event_id()?.as_str(), e))).collect();
    for item in items.iter() {
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
            // Membership, profile and room-state items: shown as status rows, never as bubbles.
            if let Some(row) = status_row(room, event, id).await {
                out.rows.push(row);
            }
            continue;
        };
        let (mut video, mut audio, mut styles, mut attachment_name, mut attachment_size) = (None, None, Vec::new(), None, None);
        let mut mentions = Vec::new();
        let mut poll = None;
        let mut media = None;
        let body = match &content.kind {
            MsgLikeKind::Message(message) => {
                let msgtype = message.msgtype();
                attachment_name = match msgtype {
                    MessageType::Image(m) => Some(crate::history::proposed_name(m.filename())),
                    MessageType::Video(m) => Some(crate::history::proposed_name(m.filename())),
                    MessageType::Audio(m) => Some(crate::history::proposed_name(m.filename())),
                    MessageType::File(m) => Some(crate::history::proposed_name(m.filename())),
                    _ => None,
                };
                attachment_size = match msgtype {
                    MessageType::Image(m) => m.info.as_deref().and_then(|i| i.size),
                    MessageType::Video(m) => m.info.as_deref().and_then(|i| i.size),
                    MessageType::Audio(m) => m.info.as_deref().and_then(|i| i.size),
                    MessageType::File(m) => m.info.as_deref().and_then(|i| i.size),
                    _ => None,
                }
                .map(u64::from);
                video = match msgtype {
                    MessageType::Video(v) => Some(video_info(v)),
                    _ => None,
                };
                // The waveform is what the sender attached; a decode fills it in when it sent none.
                audio = match msgtype {
                    MessageType::Audio(a) => {
                        let waveform = event
                            .original_json()
                            .and_then(|raw| raw.get_field::<serde_json::Value>("content").ok().flatten())
                            .map(|content| crate::history::waveform_of(&content))
                            .unwrap_or_default();
                        Some(audio_info(a, waveform))
                    }
                    _ => None,
                };
                let at_room = message.mentions().is_some_and(|m| m.room);
                media = media_of(msgtype);
                let (body, spans, pills) = if media.is_some() {
                    caption_rich(room, msgtype, at_room).await
                } else {
                    message_rich(room, msgtype, at_room).await
                };
                (styles, mentions) = (spans, pills);
                body
            }
            // Shown as an inline image; the alt text stays in the accessible label, never as caption.
            MsgLikeKind::Sticker(_) => {
                media = Some(crate::history::Media { kind: crate::history::MediaKind::Image, label: "Sticker".into() });
                String::new()
            }
            MsgLikeKind::Poll(state) => {
                let built = poll_from_results(&state.results(), room.own_user_id().as_str(), event.is_own());
                let question = built.question.clone();
                poll = Some(built);
                question
            }
            MsgLikeKind::Redacted => "[Messaggio eliminato]".into(),
            // A UTD stays a real row with server identity, never ciphertext or silence.
            MsgLikeKind::UnableToDecrypt(message) => utd_text(message).into(),
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
            redacted: matches!(content.kind, MsgLikeKind::Redacted),
            timestamp_ms: event.timestamp().0.into(),
            sender: history_sender(room, event.sender()).await,
            video,
            audio,
            media,
            styles,
            mentions,
            highlighted: event.is_highlighted(),
            thread: thread_info(room, content).await,
            thread_root: content.thread_root.as_ref().map(|r| matrix_domain::EventId::new(r.as_str())),
            reply_to: match &content.in_reply_to {
                Some(r) => Some(reply_ref(room, timeline, id, r, by_id.get(r.event_id.as_str()).copied()).await),
                None => None,
            },
            status: false,
            status_row: None,
            utd: matches!(content.kind, MsgLikeKind::UnableToDecrypt(_)),
            attachment_name,
            attachment_size,
            poll,
        });
    }
    out
}

/// Reply ids whose original we asked the server for, with the time: one request each, a retry only after `REPLY_RETRY`.
static REPLY_REQUESTED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<matrix_sdk::ruma::OwnedEventId, std::time::Instant>>> = std::sync::LazyLock::new(Default::default);

/// The quote of reply `id`: the original from the timeline's own items, else the SDK detail, else a
/// background `fetch_details_for_event` (the next `loaded` shows the result).
async fn reply_ref(
    room: &matrix_sdk::room::Room,
    timeline: &std::sync::Arc<matrix_sdk_ui::timeline::Timeline>,
    id: &matrix_sdk::ruma::EventId,
    r: &matrix_sdk_ui::timeline::InReplyToDetails,
    in_timeline: Option<&matrix_sdk_ui::timeline::EventTimelineItem>,
) -> crate::history::ReplyRef {
    use crate::history::{ReplyDetail, ReplySource, ReplyState};
    use matrix_sdk_ui::timeline::{EmbeddedEvent, MsgLikeKind, TimelineDetails};
    let detail = match &r.event {
        TimelineDetails::Ready(_) => ReplyDetail::Ready,
        TimelineDetails::Pending => ReplyDetail::Pending,
        TimelineDetails::Unavailable => ReplyDetail::Unavailable,
        TimelineDetails::Error(_) => ReplyDetail::Error,
    };
    let asked = REPLY_REQUESTED.lock().unwrap().get(id).map(|t| t.elapsed());
    let (source, fetch) = crate::history::reply_plan(detail, in_timeline.is_some(), asked);
    if fetch {
        REPLY_REQUESTED.lock().unwrap().insert(id.to_owned(), std::time::Instant::now());
        let (timeline, id) = (timeline.clone(), id.to_owned());
        tokio::spawn(async move {
            let _ = timeline.fetch_details_for_event(&id).await;
        });
    }
    let own;
    let orig: Option<&EmbeddedEvent> = match (source, &r.event) {
        (ReplySource::Timeline, _) => {
            own = EmbeddedEvent::from_timeline_item(in_timeline.expect("planned from the timeline"));
            Some(&own)
        }
        (ReplySource::Detail, TimelineDetails::Ready(e)) => Some(e),
        _ => None,
    };
    let (mut author, mut preview, mut placeholder) = (None, None, false);
    let mut state = match source {
        ReplySource::Loading => ReplyState::Loading,
        _ => ReplyState::Unavailable,
    };
    if let Some(orig) = orig {
        let text = match orig.content.as_msglike().map(|m| &m.kind) {
            Some(MsgLikeKind::Message(m)) => {
                let media = media_of(m.msgtype());
                let body = if media.is_some() { caption_rich(room, m.msgtype(), false).await.0 } else { message_rich(room, m.msgtype(), false).await.0 };
                placeholder = media.is_some() && body.trim().is_empty();
                Some(crate::history::quote_preview(&body, media.as_ref(), 80))
            }
            Some(MsgLikeKind::Sticker(_)) => {
                placeholder = true;
                Some("Sticker".to_owned())
            }
            Some(MsgLikeKind::Poll(p)) => Some(crate::history::quote_preview(&p.results().question, None, 80)),
            Some(MsgLikeKind::Redacted) => {
                state = ReplyState::Deleted;
                None
            }
            Some(MsgLikeKind::UnableToDecrypt(_)) => {
                state = ReplyState::Encrypted;
                None
            }
            _ => None,
        };
        if text.is_some() {
            let s = history_sender(room, &orig.sender).await;
            author = Some(s.name.unwrap_or(s.id));
            preview = text.filter(|t| !t.is_empty());
            state = ReplyState::Resolved;
        }
    }
    crate::history::ReplyRef { event_id: matrix_domain::EventId::new(r.event_id.as_str()), author, preview, placeholder, state }
}

/// The SDK result (already counted: last vote wins, votes after the end ignored) as a domain `Poll`.
fn poll_from_results(r: &matrix_sdk_ui::timeline::PollResult, own_user: &str, is_own_event: bool) -> matrix_domain::Poll {
    use matrix_sdk::ruma::events::poll::start::PollKind;
    poll_from_parts(
        &r.question,
        r.kind == PollKind::Disclosed,
        r.max_selections,
        r.answers.iter().map(|a| (a.id.as_str(), a.text.as_str())),
        &r.votes,
        r.end_time.is_some(),
        own_user,
        is_own_event,
    )
}

// Split from `poll_from_results` because the SDK's `PollResultAnswer` cannot be named in a test.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_from_parts<'a>(
    question: &str,
    disclosed: bool,
    max_selections: u64,
    answers: impl Iterator<Item = (&'a str, &'a str)>,
    votes: &std::collections::HashMap<String, Vec<String>>,
    ended: bool,
    own_user: &str,
    is_own_event: bool,
) -> matrix_domain::Poll {
    let mut voters = std::collections::HashSet::new();
    let answers: Vec<_> = answers
        .map(|(id, text)| {
            let who = votes.get(id).map(Vec::as_slice).unwrap_or_default();
            voters.extend(who.iter().map(String::as_str));
            matrix_domain::PollAnswer {
                id: id.to_owned(),
                text: text.to_owned(),
                votes: who.len() as u32,
                own: who.iter().any(|u| u == own_user),
            }
        })
        .collect();
    // 0 on the wire would make the poll unvotable.
    let max_selections = max_selections.clamp(1, u32::MAX.into()) as u32;
    matrix_domain::Poll {
        question: question.to_owned(),
        answers,
        max_selections,
        multiple: max_selections > 1,
        disclosed,
        ended,
        voters: voters.len() as u32,
        can_end: is_own_event && !ended,
    }
}

/// A status row from a membership, profile or room-state item; `None` for kinds we do not word
/// (unknown or hidden state stays out of the timeline).
async fn status_row(
    room: &matrix_sdk::Room,
    event: &matrix_sdk_ui::timeline::EventTimelineItem,
    id: &matrix_sdk::ruma::EventId,
) -> Option<crate::history::HistoryEvent> {
    use crate::status_line::{Status, StatusRow};
    use matrix_sdk::ruma::events::{StateEventContentChange::Original, room::member::MembershipState};
    use matrix_sdk_ui::timeline::{AnyOtherStateEventContentChange as State, MembershipChange as M, TimelineItemContent as C};
    let actor = history_sender(room, event.sender()).await;
    let label = |s: &crate::history::HistorySender| crate::status_line::display_label(s.name.as_deref(), &s.id);
    // `shown` is the person whose avatar leads the row; `subject` is who the sentence is about.
    let mut reason = None;
    let (status, shown, subject) = match event.content() {
        C::MembershipChange(change) => {
            let status = match change.change()? {
                M::Joined | M::InvitationAccepted => Status::Joined,
                M::Left => Status::Left,
                M::None if matches!(change.content(), Original { content, .. } if content.membership == MembershipState::Leave) => {
                    crate::status_line::unclassified_leave(event.sender() == change.user_id())
                }
                M::Banned | M::KickedAndBanned => Status::Banned,
                M::Unbanned => Status::Unbanned,
                M::Kicked => Status::Kicked,
                M::Invited => Status::Invited,
                M::InvitationRejected => Status::InvitationRejected,
                M::InvitationRevoked => Status::InvitationRevoked,
                _ => return None,
            };
            let mut who = history_sender(room, change.user_id()).await;
            who.name = who.name.or_else(|| change.display_name());
            let name = label(&who);
            reason = match change.content() {
                Original { content, .. } => content.reason.clone(),
                _ => None,
            };
            (status, who, name)
        }
        C::ProfileChange(change) => {
            let who = history_sender(room, change.user_id()).await;
            let status = match (change.displayname_change(), change.avatar_url_change()) {
                (Some(name), _) => Status::DisplayName { old: name.old.clone(), new: name.new.clone() },
                (None, Some(_)) => Status::MemberAvatar,
                (None, None) => return None,
            };
            let name = label(&who);
            (status, who, name)
        }
        C::OtherState(state) => {
            let status = match state.content() {
                State::RoomCreate(Original { .. }) if room.is_direct().await.unwrap_or(false) => Status::CreatedDirect,
                State::RoomCreate(Original { .. }) => Status::Created,
                State::RoomName(Original { content, .. }) => Status::RoomName(content.name.clone()),
                State::RoomTopic(Original { content, .. }) => Status::RoomTopic(if content.topic.is_empty() {
                    String::new()
                } else {
                    crate::rich_text::topic(&content.topic, content.topic_block.text.find_html()).text.replace('\n', " ")
                }),
                State::RoomAvatar(Original { .. }) => Status::RoomAvatar,
                _ => return None,
            };
            let name = label(&actor);
            (status, actor.clone(), name)
        }
        _ => return None,
    };
    // An invitation for you needs attention: `highlighted` keeps it out of the «hide status rows» filter.
    let for_me = matches!(status, Status::Invited) && shown.id == room.own_user_id().as_str();
    Some(crate::history::HistoryEvent {
        poll: None,
        event_id: matrix_domain::EventId::new(id.as_str()),
        body: String::new(),
        outgoing: false,
        redacted: false,
        timestamp_ms: event.timestamp().0.into(),
        sender: shown,
        video: None,
        audio: None,
        media: None,
        styles: Vec::new(),
        mentions: Vec::new(),
        highlighted: for_me,
        thread: None,
        thread_root: None,
        reply_to: None,
        status: true,
        status_row: Some(StatusRow { status, actor: label(&actor), subject, reason }),
        utd: false,
        attachment_name: None,
        attachment_size: None,
    })
}

/// Grow the timeline backwards until at least `want` rows are loaded or the room starts.
/// Saved chunks come first, then the server; a failed server step still returns what the
/// saved chunks added.
pub(crate) async fn fill(
    room: &matrix_sdk::Room,
    timeline: &std::sync::Arc<matrix_sdk_ui::timeline::Timeline>,
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

/// Saved messages of one room, as the room's Timeline holds them. With fewer than a PAGE of visible
/// saved rows the loop also takes network pagination steps, each cut at 1 s and at most 10, before
/// the first emission; an unreachable server only delays it.
async fn saved_page(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account_id: matrix_domain::AccountId,
    tag: crate::RequestTag,
) -> Result<crate::history::HistoryPage, crate::history::HistoryError> {
    let room = joined(client, &tag, account_id)?;
    let timeline = timelines.get(&room).await?;
    let mut now = loaded(&room, &timeline).await;
    // The timeline starts from the newest saved chunk only (the reply may sit there while its root is in
    // an older one): step back through the older saved chunks until a page is full; the SDK reads the disk first.
    // shortcut: a step that has to leave the disk is dropped after 1 s, and at most 10 steps.
    for _ in 0..10 {
        if now.rows.iter().filter(|r| !r.status).count() >= PAGE || now.at_start {
            break;
        }
        let step = tokio::time::timeout(
            Duration::from_secs(1),
            timeline.paginate_backwards(PAGE as u16),
        )
        .await;
        if !matches!(step, Ok(Ok(_))) {
            break;
        }
        // The timeline applies the chunk in its own task; a disk read lands within milliseconds.
        tokio::time::sleep(Duration::from_millis(150)).await;
        now = loaded(&room, &timeline).await;
    }
    crate::threads::annotate_unread(timelines, &room, &mut now.rows).await;
    let next = cursor(&now.rows, now.at_start);
    Ok(crate::history::HistoryPage {
        tag,
        events: now.rows,
        next,
    })
}

/// Everything the Timeline holds, after the SDK filled what sync left missing (a first
/// page when it holds less than one).
pub(crate) async fn open_page(
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
    let (mut now, _) = fill(&room, &timeline, PAGE).await;
    crate::threads::annotate_unread(timelines, &room, &mut now.rows).await;
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

/// Name shown on a bubble. Single place for the policy: a unique display name stands alone; an
/// SDK-ambiguous one keeps the user id (Matrix spec, open point with Marco); no name → `None`,
/// the UI then falls back to the id.
fn disambiguated_name(name: Option<&str>, ambiguous: bool, id: &str) -> Option<String> {
    name.map(|n| if ambiguous { format!("{n} ({id})") } else { n.to_owned() })
}

pub(crate) async fn history_sender(
    room: &matrix_sdk::room::Room,
    id: &matrix_sdk::ruma::UserId,
) -> crate::history::HistorySender {
    let member = room.get_member_no_sync(id).await.ok().flatten();
    let name = member
        .as_ref()
        .and_then(|m| disambiguated_name(m.display_name(), m.name_ambiguous(), id.as_str()));
    crate::history::HistorySender::new(id.as_str(), name.as_deref())
        .with_avatar(
            member
                .as_ref()
                .and_then(|m| m.avatar_url())
                .map(|u| u.to_string()),
        )
}

pub(crate) async fn fetch_history(
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

/// Audio facts of an `m.audio` row: duration, size, the voice marker and the waveform.
fn audio_info(
    a: &matrix_sdk::ruma::events::room::message::AudioMessageEventContent,
    waveform: Vec<f32>,
) -> crate::history::AudioInfo {
    let info = a.info.as_deref();
    crate::history::AudioInfo {
        duration_ms: info.and_then(|i| i.duration).map(|d| d.as_millis() as u64),
        size: info.and_then(|i| i.size).map(Into::into),
        voice: a.voice.is_some(),
        waveform,
    }
}

/// Members (joined or invited) that the `matrix.to` user links of `html` point at, named as on their bubbles.
async fn mention_senders(
    room: &matrix_sdk::room::Room,
    html: &str,
) -> std::collections::HashMap<String, crate::history::HistorySender> {
    use matrix_sdk::ruma::{MatrixToUri, events::room::member::MembershipState, matrix_uri::MatrixId};
    let mut out = std::collections::HashMap::new();
    // shortcut: at most 32 links per message are looked up, the rest stay plain text.
    for part in html.split("https://matrix.to/#/").skip(1).take(32) {
        let raw: String = part.chars().take_while(|c| !"\"'<> ".contains(*c)).collect();
        let Ok(uri) = MatrixToUri::parse(&format!("https://matrix.to/#/{raw}")) else { continue };
        let MatrixId::User(id) = uri.id() else { continue };
        if out.contains_key(id.as_str()) {
            continue;
        }
        let Some(member) = room.get_member_no_sync(id).await.ok().flatten() else { continue };
        if matches!(member.membership(), MembershipState::Join | MembershipState::Invite) {
            out.insert(id.to_string(), history_sender(room, id).await);
        }
    }
    out
}

/// Row text, styles and mention pills of a message: its sanitized HTML body when the sender gave
/// one, else `message_label`. `at_room` is the event's `m.mentions.room`.
async fn message_rich(
    room: &matrix_sdk::room::Room,
    msgtype: &MessageType,
    at_room: bool,
) -> (String, Vec<crate::rich_text::StyleSpan>, Vec<crate::rich_text::MentionSpan>) {
    use matrix_sdk::ruma::events::room::message::MessageFormat;
    let formatted = match msgtype {
        MessageType::Text(t) => t.formatted.as_ref(),
        MessageType::Notice(n) => n.formatted.as_ref(),
        _ => None,
    };
    if let Some(f) = formatted.filter(|f| f.format == MessageFormat::Html) {
        let users = mention_senders(room, &f.body).await;
        let ctx = crate::rich_text::MentionCtx { users: &users, me: room.own_user_id().as_str(), at_room };
        let (text, styles, mentions) = crate::rich_text::from_html_mentions(&f.body, &ctx);
        if !text.is_empty() {
            return (text, styles, mentions);
        }
    }
    let label = message_label(msgtype);
    let pills = if at_room { crate::rich_text::at_room_spans(&label, &[], &[]) } else { Vec::new() };
    (label, Vec::new(), pills)
}

/// Kind and label of an attachment message; None for every other msgtype.
fn media_of(msgtype: &MessageType) -> Option<crate::history::Media> {
    use crate::history::{Media, MediaKind};
    let kind = match msgtype {
        MessageType::Image(_) => MediaKind::Image,
        MessageType::Video(_) => MediaKind::Video,
        MessageType::Audio(_) => MediaKind::Audio,
        MessageType::File(_) => MediaKind::File,
        _ => return None,
    };
    Some(Media { kind, label: message_label(msgtype) })
}

/// Caption of an attachment, from the SDK only (`caption()` is None when the body is the file name):
/// its sanitized HTML when it gives some text, else the plain text, else nothing (blank = none).
async fn caption_rich(
    room: &matrix_sdk::room::Room,
    msgtype: &MessageType,
    at_room: bool,
) -> (String, Vec<crate::rich_text::StyleSpan>, Vec<crate::rich_text::MentionSpan>) {
    use matrix_sdk::ruma::events::room::message::MessageFormat;
    let (plain, formatted) = caption_sources(msgtype);
    if let Some(f) = formatted.filter(|f| f.format == MessageFormat::Html) {
        let users = mention_senders(room, &f.body).await;
        let ctx = crate::rich_text::MentionCtx { users: &users, me: room.own_user_id().as_str(), at_room };
        let (text, styles, mentions) = crate::rich_text::from_html_mentions(&f.body, &ctx);
        if !text.trim().is_empty() {
            return (text, styles, mentions);
        }
    }
    match plain.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => (c.to_owned(), Vec::new(), if at_room { crate::rich_text::at_room_spans(c, &[], &[]) } else { Vec::new() }),
        None => (String::new(), Vec::new(), Vec::new()),
    }
}

fn caption_sources(
    msgtype: &MessageType,
) -> (Option<&str>, Option<&matrix_sdk::ruma::events::room::message::FormattedBody>) {
    match msgtype {
        MessageType::Image(m) => (m.caption(), m.formatted_caption()),
        MessageType::Video(m) => (m.caption(), m.formatted_caption()),
        MessageType::Audio(m) => (m.caption(), m.formatted_caption()),
        MessageType::File(m) => (m.caption(), m.formatted_caption()),
        _ => (None, None),
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
            "Message without text".into()
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
                    "Voice note"
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
        MessageType::Location(_) => "Location".into(),
        MessageType::VerificationRequest(_) => "Verification request".into(),
        MessageType::Image(_) => "Image".into(),
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
    let mut events: Vec<_> = now.rows.drain(from..).collect();
    crate::threads::annotate_unread(timelines, &room, &mut events).await;
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

#[cfg(test)]
mod utd_text_tests {
    use super::*;
    use matrix_sdk_crypto::types::events::UtdCause as C;
    #[test]
    fn cause_picks_the_one_line() {
        assert!(utd_cause_text(C::HistoricalMessageAndDeviceIsUnverified).contains("non ancora arrivate"));
        assert!(utd_cause_text(C::SentBeforeWeJoined).contains("non recuperabile"));
        assert!(utd_cause_text(C::HistoricalMessageAndBackupIsDisabled).contains("non recuperabile"));
        assert_eq!(utd_cause_text(C::Unknown), UTD_DEFAULT);
        let all = [UTD_DEFAULT, utd_cause_text(C::HistoricalMessageAndDeviceIsUnverified), utd_cause_text(C::SentBeforeWeJoined)];
        assert!(all.iter().all(|t| !t.contains('\n')), "one line each");
        assert!(all[1] != all[2] && all[0] != all[1] && all[0] != all[2], "the cases read differently");
    }
}
