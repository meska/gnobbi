//! Threads through `matrix-sdk-ui`: the thread view is a `TimelineFocus::Thread` timeline kept next
//! to the room's own, the list is `ThreadListService`. DTOs carry no SDK type.
use crate::history::{HistoryError, HistoryEvent, ReactionCount};
use crate::room_sync::{PAGE, Timelines, fill, loaded};
use matrix_domain::EventId;
use matrix_sdk_ui::timeline::thread_list_service::{ThreadListItemEvent, ThreadListService};

/// One thread opened: `root` is None when the room's own timeline does not hold it (the thread
/// timeline alone does not return it reliably); `replies` are oldest first.
pub struct ThreadPage {
    pub root: Option<HistoryEvent>,
    pub replies: Vec<HistoryEvent>,
    pub reactions: Vec<ReactionCount>,
    /// The thread timeline says its start was reached. Servers get this wrong when the thread is
    /// bigger than the sync window: compare `replies.len()` with `ThreadSummary::num_replies`.
    pub at_start: bool,
}

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
