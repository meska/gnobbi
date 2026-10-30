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
    open(timelines, &room, root, want).await.map(Box::new).map_err(|e| match e {
        HistoryError::Transport => E::Transport,
        HistoryError::Unavailable => E::Unavailable,
        _ => E::WrongScope,
    })
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
