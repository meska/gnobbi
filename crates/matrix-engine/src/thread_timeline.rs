//! Isolated adapter over `matrix-sdk-ui` for thread timelines (feature `threads`).
//! The caller passes a `Room` from the per-account client (SQLite store and session already set
//! up by `auth`); nothing here builds a client, touches the main timeline or logs content.
use matrix_sdk::{Room, ruma::{OwnedEventId, OwnedUserId}};
use matrix_sdk_ui::timeline::{Error, RoomExt, Timeline, TimelineFocus};

/// One event of a thread, root first. `body` is None for non-message content.
#[derive(Clone, PartialEq, Eq)]
pub struct ThreadEvent {
    pub event_id: Option<OwnedEventId>,
    pub sender: OwnedUserId,
    pub body: Option<String>,
}

/// A `Timeline` focused on the thread whose root is `root`.
pub async fn open(room: &Room, root: OwnedEventId) -> Result<Timeline, Error> {
    room.timeline_builder().with_focus(TimelineFocus::Thread { root_event_id: root }).build().await
}

/// Snapshot of the thread events currently loaded in `timeline`.
pub async fn events(timeline: &Timeline) -> Vec<ThreadEvent> {
    timeline
        .items()
        .await
        .iter()
        .filter_map(|item| item.as_event())
        .map(|e| ThreadEvent {
            event_id: e.event_id().map(ToOwned::to_owned),
            sender: e.sender().to_owned(),
            body: e.content().as_message().map(|m| m.body().to_owned()),
        })
        .collect()
}

/// Loads up to `count` older events; true once the thread root has been reached.
pub async fn load_older(timeline: &Timeline, count: u16) -> Result<bool, Error> {
    timeline.paginate_backwards(count).await
}
