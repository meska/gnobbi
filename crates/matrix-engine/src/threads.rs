//! Threads through `matrix-sdk-ui`: the thread view is a `TimelineFocus::Thread` timeline kept next
//! to the room's own, the list is `ThreadListService`. DTOs carry no SDK type.
use crate::history::HistoryError;
use crate::room_sync::{PAGE, Timelines, fill, loaded};
use matrix_domain::EventId;
use matrix_sdk_ui::timeline::thread_list_service::{ThreadListItemEvent, ThreadListService};

pub use crate::history::ThreadPage;

pub struct ThreadSummary {
    pub root: EventId,
    pub root_sender: String,
    pub root_body: Option<String>,
    pub latest_sender: Option<String>,
    pub latest_body: Option<String>,
    pub num_replies: u32,
}

pub(crate) async fn open(
    timelines: &Timelines,
    room: &matrix_sdk::Room,
    root: &EventId,
    want: usize,
) -> Result<ThreadPage, HistoryError> {
    let root_id = matrix_sdk::ruma::EventId::parse(root.as_str()).map_err(|_| HistoryError::WrongScope)?;
    let main = loaded(room, &*timelines.get(room).await?).await;
    let thread = timelines.thread(room, &root_id).await?;
    let (mut now, status) = fill(room, &thread, want.max(PAGE)).await;
    // A thread that cannot be read at all is an error; a partial one is shown.
    if now.rows.is_empty() && let Err(e) = status {
        return Err(e);
    }
    // No `annotate_unread` here: it paginates every thread in view (network) and the read below zeroes this one.
    let mut root_row = main.rows.into_iter().find(|r| r.event_id == *root);
    // The room's own timeline keeps the root's summary current; the thread's copy goes stale.
    if let Some(i) = now.rows.iter().position(|r| r.event_id == *root) {
        let own = now.rows.remove(i);
        root_row = root_row.or(Some(own));
    }
    Ok(ThreadPage { root: root_row, replies: now.rows, reactions: now.reactions, at_start: now.at_start })
}

/// The thread panel's request: same as `open`, from the account client and a scope.
pub(crate) async fn open_scoped(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    root: &EventId,
    want: usize,
    read: Option<bool>,
) -> Result<Box<ThreadPage>, crate::operations::OperationError> {
    use crate::operations::OperationError as E;
    if scope.account_id != account {
        return Err(E::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| E::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(E::Unavailable)?;
    let mut page = open(timelines, &room, root, want).await.map_err(|e| match e {
        HistoryError::Transport => E::Transport,
        HistoryError::Unavailable => E::Unavailable,
        _ => E::WrongScope,
    })?;
    // Read after the load (the receipt points at the newest event the thread holds); a failed
    // receipt is retried by the next request and does not hide the thread.
    if let Some(public) = read
        && mark_read_scoped(client, timelines, account, scope, root, public).await.is_ok()
        && let Some(info) = page.root.as_mut().and_then(|r| r.thread.as_mut())
    {
        info.unread = 0;
    }
    Ok(Box::new(page))
}

/// A text into the thread's own `Timeline`: the SDK adds `m.thread` (and the fallback) itself, and
/// `send_reply` points at the event answered. Returns once the server has the event.
pub(crate) async fn send_scoped(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    root: &EventId,
    reply_to: Option<&EventId>,
    body: &str,
) -> Result<EventId, crate::operations::OperationError> {
    use crate::operations::OperationError as E;
    use matrix_sdk::ruma::{EventId as Ruma, RoomId};
    if scope.account_id != account {
        return Err(E::WrongScope);
    }
    if body.trim().is_empty() || body.len() > 32768 {
        return Err(E::InvalidInput);
    }
    let room_id = RoomId::parse(scope.room_id.as_str()).map_err(|_| E::WrongScope)?;
    let root_id = Ruma::parse(root.as_str()).map_err(|_| E::InvalidInput)?;
    let reply = reply_to.map(|r| Ruma::parse(r.as_str()).map_err(|_| E::InvalidInput)).transpose()?;
    let room = client
        .get_room(&room_id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(E::Unavailable)?;
    let timeline = timelines.thread(&room, &root_id).await.map_err(|_| E::Unavailable)?;
    let content = crate::sending::markdown_content(body, &[]);
    let mut updates = client.send_queue().subscribe();
    match &reply {
        Some(target) => timeline.send_reply(content.into(), target.clone()).await.map(drop),
        None => timeline.send(content.into()).await.map(drop),
    }
    // The target must be in the thread's timeline: a hole (Tuwunel) makes the SDK refuse.
    .map_err(|_| if reply.is_some() { E::Unavailable } else { E::Transport })?;
    crate::operations::sent_event(&mut updates, &room, |e| {
        let r = &e["content"]["m.relates_to"];
        r["rel_type"] == "m.thread"
            && r["event_id"].as_str() == Some(root_id.as_str())
            && reply.as_ref().is_none_or(|t| r["m.in_reply_to"]["event_id"].as_str() == Some(t.as_str()))
    })
    .await
}

/// Fills `ThreadInfo.unread` of the rows that are thread roots. The SDK (sliding sync, 0.19.1) has no
/// per-thread counter, so it is read from the thread's own `Timeline`: replies by others after our
/// read marker there (implicit receipts included). Servers do send per-thread counts, but only in
/// classic sync, which the app does not use.
// shortcut: reads at most 3 older pages of each thread (until the marker or the whole thread shows); a
// longer unread tail counts what was read, and a page that is slow to apply is not waited for. One `Timeline` per root.
pub(crate) async fn annotate_unread(timelines: &Timelines, room: &matrix_sdk::Room, rows: &mut [crate::history::HistoryEvent]) {
    use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
    let Some(me) = room.client().user_id().map(ToOwned::to_owned) else {
        return;
    };
    for row in rows.iter_mut().filter(|r| r.thread.is_some()) {
        let Ok(root) = matrix_sdk::ruma::EventId::parse(row.event_id.as_str()) else {
            continue;
        };
        let Ok(thread) = timelines.thread(room, &root).await else {
            continue;
        };
        let Some(info) = row.thread.as_mut() else { continue };
        // Our markers that cover this thread: its own, and the unthreaded ones (they cover every thread).
        let mut markers = Vec::new();
        for kind in [ReceiptType::Read, ReceiptType::ReadPrivate] {
            for scope in [ReceiptThread::Thread(root.clone()), ReceiptThread::Unthreaded] {
                if let Ok(Some((id, _))) = room.load_user_receipt(kind.clone(), &scope, &me).await {
                    markers.push(id);
                }
            }
        }
        let mut now = loaded(room, &thread).await;
        let position = |now: &crate::room_sync::Loaded| now.rows.iter().rposition(|r| markers.iter().any(|m| m.as_str() == r.event_id.as_str()));
        // `at_start` is not reliable for threads: the whole thread is what the count says.
        let replies = |now: &crate::room_sync::Loaded| now.rows.iter().filter(|r| r.event_id != row.event_id).count();
        for _ in 0..3 {
            if position(&now).is_some() || replies(&now) >= info.num_replies as usize || now.at_start {
                break;
            }
            // One older page and a short wait for its rows (no polling to a deadline: a refresh must not stall).
            let before = now.rows.len();
            if thread.paginate_backwards(PAGE as u16).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            now = loaded(room, &thread).await;
            if now.rows.len() == before {
                break;
            }
        }
        // Replies by others after the marker; a reply of ours reads everything before it.
        let from = position(&now).map_or(0, |i| i + 1);
        info.unread = 0;
        for r in now.rows.iter().skip(from).filter(|r| r.event_id != row.event_id) {
            info.unread = if r.outgoing { 0 } else { info.unread + 1 };
        }
    }
}

/// Receipt for the thread's newest event, with the thread's root as `thread_id`: the SDK picks
/// `ReceiptThread::Thread` from the thread `Timeline`'s focus, the room's other threads stay unread.
pub(crate) async fn mark_read_scoped(
    client: &matrix_sdk::Client,
    timelines: &Timelines,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    root: &EventId,
    public: bool,
) -> Result<bool, crate::operations::OperationError> {
    use crate::operations::OperationError as E;
    use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
    if scope.account_id != account {
        return Err(E::WrongScope);
    }
    let room_id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| E::WrongScope)?;
    let root_id = matrix_sdk::ruma::EventId::parse(root.as_str()).map_err(|_| E::InvalidInput)?;
    let room = client
        .get_room(&room_id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(E::Unavailable)?;
    let timeline = timelines.thread(&room, &root_id).await.map_err(|_| E::Unavailable)?;
    let kind = if public { ReceiptType::Read } else { ReceiptType::ReadPrivate };
    timeline.mark_as_read(kind).await.map_err(|_| E::Transport)
}

/// Receipt `main` for the newest event outside every thread. The room's timeline would send an
/// unthreaded one (it still shows thread replies), which reads all threads as well.
pub(crate) async fn mark_main(room: &matrix_sdk::Room, timeline: &matrix_sdk_ui::timeline::Timeline, public: bool) -> Result<bool, matrix_sdk::Error> {
    use matrix_sdk::ruma::{api::client::receipt::create_receipt::v3::ReceiptType, events::receipt};
    use receipt::ReceiptThread;
    let (kind, stored) = if public { (ReceiptType::Read, receipt::ReceiptType::Read) } else { (ReceiptType::ReadPrivate, receipt::ReceiptType::ReadPrivate) };
    let items = timeline.items().await;
    let newest = items.iter().rev().filter_map(|i| i.as_event()).find(|e| {
        e.event_id().is_some() && e.content().as_msglike().is_none_or(|m| m.thread_root.is_none())
    });
    let (Some(id), Some(me)) = (newest.and_then(|e| e.event_id()), room.client().user_id().map(ToOwned::to_owned)) else {
        return Ok(false);
    };
    let current = room.load_user_receipt(stored, &ReceiptThread::Main, &me).await.ok().flatten();
    if current.is_some_and(|(have, _)| have == id) {
        return Ok(false);
    }
    room.send_single_receipt(kind, ReceiptThread::Main, id.to_owned()).await.map(|()| true)
}

fn body(event: &ThreadListItemEvent) -> Option<String> {
    event.content.as_ref()?.as_message().map(|m| m.body().to_owned())
}

pub(crate) async fn list(room: matrix_sdk::Room) -> Result<Vec<ThreadSummary>, HistoryError> {
    // shortcut: one page and no live updates; the list view (P331+) subscribes to the service.
    let service = ThreadListService::new(room);
    service.paginate().await.map_err(|_| HistoryError::Transport)?;
    Ok(service
        .items()
        .iter()
        .map(|i| ThreadSummary {
            root: EventId::new(i.root_event.event_id.as_str()),
            root_sender: i.root_event.sender.to_string(),
            root_body: body(&i.root_event),
            latest_sender: i.latest_event.as_ref().map(|e| e.sender.to_string()),
            latest_body: i.latest_event.as_ref().and_then(body),
            num_replies: i.num_replies,
        })
        .collect())
}
