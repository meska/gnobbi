//! Text sends through the SDK send queue. `Timeline::send` queues the message (the queue picks the
//! transaction ID, keeps it in the store and resends it after a restart); this module follows the
//! queue's updates and gives the window retry, cancel and edit-before-send. Bodies never get logged.
use crate::sending::{
    Mention, SendControl, SendError, SendEvent, SendNotice, SendRequest, SendResult,
    map_error, markdown_content, queue_text, queued_text, room_for,
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, TransactionId};
use matrix_sdk::{
    Client,
    ruma::{OwnedRoomId, OwnedTransactionId},
    send_queue::{LocalEchoContent, RoomSendQueueUpdate, SendHandle, SendQueueUpdate},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// A queued text. `id` is the transaction ID the window knows it by: its own for a message it
/// sent, the queue's for one restored after a restart.
struct Entry {
    room: OwnedRoomId,
    id: TransactionId,
    handle: SendHandle,
    /// The last attempt failed and was reported; the message waits for a retry.
    failed: bool,
}

#[derive(Clone)]
pub(crate) struct Outbox {
    client: Client,
    account: AccountId,
    runtime: tokio::runtime::Handle,
    entries: Arc<Mutex<HashMap<OwnedTransactionId, Entry>>>,
    /// Attachments uploading now, with their progress.
    uploads: crate::sending::Uploads,
    /// Attachments withdrawn before their turn came: they are never sent.
    withdrawn: Arc<Mutex<std::collections::HashSet<String>>>,
}

fn notice(scope: RoomScope, transaction_id: TransactionId, notice: SendNotice) -> SendEvent {
    SendEvent::Notice { scope, transaction_id, notice }
}

impl Outbox {
    pub(crate) fn new(client: Client, account: AccountId, runtime: tokio::runtime::Handle) -> Self {
        Self { client, account, runtime, entries: Default::default(), uploads: Default::default(), withdrawn: Default::default() }
    }

    fn scope(&self, room: &OwnedRoomId) -> RoomScope {
        RoomScope { account_id: self.account, room_id: RoomId::new(room.as_str()) }
    }

    /// Messages still on their way out; one that failed and was reported does not count.
    pub(crate) fn in_flight(&self) -> usize {
        self.entries.lock().unwrap().values().filter(|e| !e.failed).count()
    }

    fn find(&self, id: &TransactionId) -> Option<(OwnedTransactionId, SendHandle)> {
        let entries = self.entries.lock().unwrap();
        let (txn, entry) = entries.iter().find(|(_, e)| &e.id == id)?;
        Some((txn.clone(), entry.handle.clone()))
    }

    /// Answer a request from the window. Attachments still go out directly and report at once.
    pub(crate) async fn accept(
        &self,
        timelines: &crate::room_sync::Timelines,
        request: &SendRequest,
    ) -> Option<SendEvent> {
        let outcome = async {
            let room = room_for(&self.client, self.account, request)?;
            if let Some(file) = &request.attachment {
                if self.withdrawn.lock().unwrap().remove(request.transaction_id.as_str()) {
                    return Err(SendError::Cancelled);
                }
                let timeline = timelines.get(&room).await.map_err(|_| SendError::Unavailable)?;
                return crate::sending::send_attachment(&self.client, &room, &timeline, request, file, Some(&self.uploads))
                    .await
                    .map(Some);
            }
            // The same request again is a retry of the queued message, never a second message.
            if self.retry(&request.transaction_id) {
                return Ok(None);
            }
            let timeline = timelines.get(&room).await.map_err(|_| SendError::Unavailable)?;
            let (txn, handle) = queue_text(&self.client, &room, &timeline, request).await?;
            self.entries.lock().unwrap().insert(
                txn,
                Entry {
                    room: room.room_id().to_owned(),
                    id: request.transaction_id.clone(),
                    handle,
                    failed: false,
                },
            );
            Ok(None)
        }
        .await;
        let result = |result| {
            SendEvent::Result(SendResult {
                scope: request.scope.clone(),
                transaction_id: request.transaction_id.clone(),
                result,
            })
        };
        match outcome {
            Ok(None) => None,
            Ok(Some(id)) => Some(result(Ok(id))),
            Err(error) => Some(result(Err(error))),
        }
    }

    /// Messages an earlier run left in the queue: shown again (failed ones wait for a retry), the
    /// others are sent by the room tasks `respawn_tasks_for_rooms_with_unsent_requests` restarts.
    pub(crate) async fn restore(&self) -> Vec<SendEvent> {
        let Ok(echoes) = self.client.send_queue().local_echoes().await else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (room, echoes) in echoes {
            for echo in echoes {
                let LocalEchoContent::Event { serialized_event, send_handle, send_error } =
                    echo.content
                else {
                    continue;
                };
                let Some((body, true)) = queued_text(serialized_event) else {
                    continue;
                };
                let id = TransactionId::new(echo.transaction_id.as_str());
                let failed = send_error.is_some();
                self.entries.lock().unwrap().insert(
                    echo.transaction_id,
                    Entry { room: room.clone(), id: id.clone(), handle: send_handle, failed },
                );
                out.push(notice(self.scope(&room), id, SendNotice::Restored { body, failed }));
            }
        }
        out
    }

    /// One update of any room's queue; only texts this outbox follows produce an event.
    pub(crate) fn update(&self, update: SendQueueUpdate) -> Option<SendEvent> {
        use RoomSendQueueUpdate as U;
        let mut entries = self.entries.lock().unwrap();
        let (txn, event) = match update.update {
            U::SentEvent { transaction_id, event_id } => {
                let entry = entries.remove(&transaction_id)?;
                let result = SendResult {
                    scope: self.scope(&entry.room),
                    transaction_id: entry.id,
                    result: Ok(EventId::new(event_id.as_str())),
                };
                return Some(SendEvent::Result(result));
            }
            U::SendError { transaction_id, error, .. } => {
                let entry = entries.get_mut(&transaction_id)?;
                entry.failed = true;
                let result = SendResult {
                    scope: self.scope(&entry.room),
                    transaction_id: entry.id.clone(),
                    result: Err(map_error(&error)),
                };
                return Some(SendEvent::Result(result));
            }
            U::CancelledLocalEvent { transaction_id } => {
                let entry = entries.remove(&transaction_id)?;
                return Some(notice(self.scope(&entry.room), entry.id, SendNotice::Cancelled));
            }
            U::RetryEvent { transaction_id } => {
                let entry = entries.get_mut(&transaction_id)?;
                entry.failed = false;
                (transaction_id, SendNotice::Retrying)
            }
            U::ReplacedLocalEvent { transaction_id, new_content } => {
                let (body, _) = queued_text(new_content)?;
                (transaction_id, SendNotice::Edited { body })
            }
            U::NewLocalEvent(_) | U::MediaUpload { .. } => return None,
        };
        let entry = entries.get(&txn)?;
        Some(notice(self.scope(&entry.room), entry.id.clone(), event))
    }

    /// The queue switched itself off after a recoverable error: switch it on again and tell the
    /// window which messages are being tried once more.
    pub(crate) async fn resume(&self) -> Vec<SendEvent> {
        self.client.send_queue().set_enabled(true).await;
        let mut retrying = Vec::new();
        for entry in self.entries.lock().unwrap().values_mut() {
            if std::mem::take(&mut entry.failed) {
                retrying.push(notice(self.scope(&entry.room), entry.id.clone(), SendNotice::Retrying));
            }
        }
        retrying
    }
}

impl SendControl for Outbox {
    fn retry(&self, id: &TransactionId) -> bool {
        let Some((_, handle)) = self.find(id) else {
            return false;
        };
        let client = self.client.clone();
        self.runtime.spawn(async move {
            // A wedged message needs unwedging; a recoverable failure only needs the queue back on.
            let _ = handle.unwedge().await;
            client.send_queue().set_enabled(true).await;
        });
        true
    }

    fn cancel(&self, id: &TransactionId) -> bool {
        let Some((_, handle)) = self.find(id) else {
            // An attachment: uploading now (abort its queued event) or still waiting for its turn (never start it).
            if self.uploads.lock().unwrap().contains_key(id.as_str()) {
                let client = self.client.clone();
                let txn = id.as_str().to_owned();
                self.runtime.spawn(async move {
                    use matrix_sdk::send_queue::LocalEchoContent;
                    let Ok(echoes) = client.send_queue().local_echoes().await else { return };
                    if let Some(echo) = echoes.into_values().flatten().find(|e| e.transaction_id.as_str() == txn)
                        && let LocalEchoContent::Event { send_handle, .. } = echo.content
                    {
                        let _ = send_handle.abort().await;
                    }
                });
            } else {
                self.withdrawn.lock().unwrap().insert(id.as_str().to_owned());
            }
            return true;
        };
        self.runtime.spawn(async move {
            let _ = handle.abort().await;
        });
        true
    }

    fn uploads(&self) -> Vec<(TransactionId, u64, u64)> {
        let uploads = self.uploads.lock().unwrap();
        uploads.iter().map(|(id, (sent, all))| (TransactionId::new(id.as_str()), *sent as u64, *all as u64)).collect()
    }

    fn edit(&self, id: &TransactionId, body: &str, mentions: &[Mention]) -> bool {
        if body.trim().is_empty() {
            return false;
        }
        let Some((_, handle)) = self.find(id) else {
            return false;
        };
        let content = markdown_content(body, mentions);
        self.runtime.spawn(async move {
            let _ = handle.edit(content.into()).await;
        });
        true
    }
}

/// The follow-up of the queue for one account, until the worker stops: accepts the window's
/// requests, turns queue updates into events, switches the queue back on after a recoverable
/// error (backoff 2 s doubling to 30 s) and, once stopping, waits for the messages still in flight.
/// False when a failure could not be reported or sends stay unconfirmed with nobody listening.
pub(crate) async fn run(
    outbox: &Outbox,
    timelines: &crate::room_sync::Timelines,
    sends: &mut crate::Receiver<SendRequest>,
    results: &mut crate::Sender<SendEvent>,
    stop: &std::sync::atomic::AtomicBool,
) -> bool {
    use std::sync::atomic::Ordering::Acquire;
    use tokio::{sync::broadcast::error::RecvError, time::Instant};
    let client = outbox.client.clone();
    let mut updates = client.send_queue().subscribe();
    let mut errors = client.send_queue().subscribe_errors();
    let mut clean = true;
    let emit = async |event: SendEvent, results: &mut crate::Sender<SendEvent>| {
        let failed = matches!(&event, SendEvent::Result(SendResult { result: Err(_), .. }));
        let delivered = matches!(
            crate::room_sync::until_stopped(stop, results.send(event)).await,
            Some(Ok(()))
        );
        failed && !delivered
    };
    for event in outbox.restore().await {
        clean &= !emit(event, results).await;
    }
    client.send_queue().respawn_tasks_for_rooms_with_unsent_requests().await;
    let (mut backoff, mut resume_at) = (Duration::from_secs(2), None::<Instant>);
    let mut accepting = true;
    loop {
        let stopping = stop.load(Acquire);
        if stopping && accepting {
            sends.close();
        }
        if stopping && outbox.in_flight() == 0 && !accepting {
            return clean;
        }
        // Nobody left to hear the outcome: a send still in flight must not count as flushed.
        if stopping && results.is_closed() && outbox.in_flight() > 0 {
            return false;
        }
        let events = tokio::select! {
            request = sends.recv(), if accepting => match request {
                Some(request) => outbox.accept(timelines, &request).await.into_iter().collect(),
                None => {
                    accepting = false;
                    Vec::new()
                }
            },
            update = updates.recv() => match update {
                Ok(update) => outbox.update(update).into_iter().collect(),
                Err(RecvError::Lagged(_)) => Vec::new(),
                Err(RecvError::Closed) => return clean,
            },
            error = errors.recv() => {
                if let Ok(error) = error && error.is_recoverable {
                    resume_at.get_or_insert(Instant::now() + backoff);
                }
                Vec::new()
            }
            _ = tokio::time::sleep_until(resume_at.unwrap_or_else(Instant::now)), if resume_at.is_some() => {
                resume_at = None;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                outbox.resume().await
            }
            _ = tokio::time::sleep(Duration::from_millis(25)) => Vec::new(),
        };
        for event in events {
            if matches!(&event, SendEvent::Result(SendResult { result: Ok(_), .. })) {
                backoff = Duration::from_secs(2);
            }
            clean &= !emit(event, results).await;
        }
    }
}

/// The outbox the window drives, for the disposable-server tests in `tests/`: accept one request (waits until it is
/// out, failed or withdrawn), and the cancel and progress side the window reads.
#[doc(hidden)]
pub struct TestOutbox(Outbox, crate::room_sync::Timelines);

#[doc(hidden)]
impl TestOutbox {
    pub fn new(client: Client, account: AccountId) -> Self {
        Self(Outbox::new(client, account, tokio::runtime::Handle::current()), Default::default())
    }

    pub async fn send(&self, request: &SendRequest) -> Option<SendEvent> {
        self.0.accept(&self.1, request).await
    }

    pub fn control(&self) -> &dyn SendControl {
        &self.0
    }
}
