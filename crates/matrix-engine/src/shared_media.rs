//! The «Media, file e link» tabs of a room: its history paged back with the SDK's `Room::messages`.
//! An unencrypted room lets the server drop what is not a message with a URL (`contains_url`); an encrypted room has the URL
//! inside the ciphertext, so its decrypted pages are filtered here. The Links tab always reads the body. One pass reads at most
//! `PASS_PAGES` pages and then stops until the next ask, so nothing loops until the room ends.
use crate::operations::{MoreHits, SharedItem, SharedKind, SharedState, SharedTab};
use matrix_sdk::{Client, ruma::{OwnedRoomId, api::client::filter::{RoomEventFilter, UrlFilter}}};
use serde_json::Value;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::{sync::watch, task::JoinHandle};

#[cfg(test)]
#[path = "shared_media_tests.rs"]
mod tests;

/// Events asked for per page.
pub const PAGE_EVENTS: u32 = 50;
/// Pages one pass reads at most, whatever it finds.
pub const PASS_PAGES: usize = 10;
/// A pass stops as soon as it has this many items.
pub const PASS_ITEMS: usize = 30;

fn is_edit(event: &Value) -> bool {
    event["content"]["m.relates_to"]["rel_type"] == "m.replace"
}

/// Characters of a message kept on the Links tab: the rest is never shown and a body can be 64 KB.
const LINK_TEXT_CHARS: usize = 200;

/// The first web address in `body`. Not `matrix_desktop::links::message_links`: that one decides what is clickable in the chat, this
/// one what the Links tab lists, so edge cases (`http://a`, a trailing parenthesis) can differ. shortcut: unaligned, align if reported.
/// (`http(s)://`, without the punctuation around it).
fn first_url(body: &str) -> Option<&str> {
    body.split_whitespace()
        .map(|w| w.trim_matches(|c: char| "()[]<>\"'.,;!".contains(c)))
        .find(|w| {
            let lower = w.get(..8).unwrap_or(w).to_ascii_lowercase();
            (lower.starts_with("http://") || lower.starts_with("https://")) && w.len() > 8
        })
}

/// What `event` is on `tab`, if anything. Redacted messages and edits are never listed.
pub fn classify(tab: SharedTab, event: &Value) -> Option<SharedItem> {
    if event["type"] != "m.room.message" || is_edit(event) || !event["unsigned"]["redacted_because"].is_null() {
        return None;
    }
    let content = &event["content"];
    let info = &content["info"];
    let body = content["body"].as_str()?;
    let (kind, name, url) = match (tab, content["msgtype"].as_str()?) {
        (SharedTab::Media, "m.image") => (SharedKind::Image, None, None),
        (SharedTab::Media, "m.video") => (SharedKind::Video, None, None),
        (SharedTab::Files, "m.file") => (SharedKind::File, None, None),
        // A voice note is a message, not a file.
        (SharedTab::Files, "m.audio") if content.get("org.matrix.msc3245.voice").is_none() => (SharedKind::File, None, None),
        (SharedTab::Links, "m.text" | "m.notice" | "m.emote") => (SharedKind::Link, Some(body.chars().take(LINK_TEXT_CHARS).collect()), Some(first_url(body)?.to_owned())),
        _ => return None,
    };
    // With a `filename` the body is the caption (MSC2530).
    let name = name.unwrap_or_else(|| content["filename"].as_str().unwrap_or(body).to_owned());
    Some(SharedItem {
        event_id: event["event_id"].as_str()?.to_owned(),
        kind,
        sender: event["sender"].as_str()?.to_owned(),
        sender_name: String::new(),
        ts: event["origin_server_ts"].as_u64()?,
        name,
        size: info["size"].as_u64(),
        duration_ms: info["duration"].as_u64(),
        url,
    })
}

/// What the server is asked to drop. `encrypted`: nothing (the type and the URL are hidden in the ciphertext).
pub fn filter(tab: SharedTab, encrypted: bool) -> RoomEventFilter {
    let mut filter = RoomEventFilter::default();
    if !encrypted {
        filter.types = Some(vec!["m.room.message".to_owned()]);
        if tab != SharedTab::Links {
            filter.url_filter = Some(UrlFilter::EventsWithUrl);
        }
    }
    filter
}

/// One pass over the history: `fetch(from)` answers a page of events and the token of the next one (none at the room's start).
/// Reads pages until `PASS_ITEMS` items are found, the room starts or `PASS_PAGES` pages are read.
pub async fn pass<F, Fut>(tab: SharedTab, mut from: Option<String>, mut fetch: F) -> Result<(Vec<SharedItem>, Option<String>, bool), ()>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<(Vec<Value>, Option<String>), ()>>,
{
    let mut items = Vec::new();
    for _ in 0..PASS_PAGES {
        let (events, next) = fetch(from.clone()).await?;
        items.extend(events.iter().filter_map(|e| classify(tab, e)));
        match next {
            None => return Ok((items, None, true)),
            Some(next) => from = Some(next),
        }
        if items.len() >= PASS_ITEMS {
            break;
        }
    }
    Ok((items, from, false))
}

#[derive(Default)]
struct Paging {
    items: Vec<SharedItem>,
    next: Option<String>,
    done: bool,
}

pub struct SharedMedia {
    client: Client,
    room_id: OwnedRoomId,
    tab: SharedTab,
    paging: Arc<Mutex<Paging>>,
    tx: Arc<watch::Sender<SharedState>>,
    task: Option<JoinHandle<()>>,
}

fn state(p: &Paging, more: MoreHits) -> SharedState {
    SharedState { items: p.items.clone(), more: if p.done { MoreHits::None } else { more } }
}

impl SharedMedia {
    /// Starts reading at once: the first pass runs in the background.
    pub fn open(client: Client, room_id: OwnedRoomId, tab: SharedTab) -> Self {
        let tx = Arc::new(watch::channel(SharedState { items: Vec::new(), more: MoreHits::Loading }).0);
        let mut this = Self { client, room_id, tab, paging: Arc::default(), tx, task: None };
        this.more();
        this
    }

    pub fn subscribe(&self) -> watch::Receiver<SharedState> {
        self.tx.subscribe()
    }

    /// One more pass. Ignored while one runs or when the room's start was reached.
    pub fn more(&mut self) {
        if self.task.as_ref().is_some_and(|t| !t.is_finished()) || self.paging.lock().unwrap().done {
            return;
        }
        let (client, room_id, tab, paging, tx) = (self.client.clone(), self.room_id.clone(), self.tab, self.paging.clone(), self.tx.clone());
        let loading = state(&paging.lock().unwrap(), MoreHits::Loading);
        tx.send_replace(loading);
        self.task = Some(tokio::spawn(async move {
            let Some(room) = client.get_room(&room_id) else {
                eprintln!("Gnobbi: shared media: room not found");
                let failed = state(&paging.lock().unwrap(), MoreHits::Failed);
                tx.send_replace(failed);
                return;
            };
            // Decided on every pass: a room can become encrypted. Unknown counts as encrypted (nothing is filtered by the server).
            let encrypted = !matches!(room.latest_encryption_state().await, Ok(matrix_sdk::EncryptionState::NotEncrypted));
            let from = paging.lock().unwrap().next.clone();
            let found = pass(tab, from, |from| {
                let room = room.clone();
                async move {
                    let mut options = matrix_sdk::room::MessagesOptions::backward();
                    options.from = from;
                    options.limit = PAGE_EVENTS.into();
                    options.filter = filter(tab, encrypted);
                    let page = room.messages(options).await.map_err(|e| eprintln!("Gnobbi: shared media: page failed: {}", error_kind(&e)))?;
                    let events = page.chunk.iter().filter_map(|e| serde_json::from_str(e.raw().json().get()).ok()).collect();
                    Ok((events, page.end))
                }
            })
            .await;
            let next = match found {
                Ok((mut items, next, done)) => {
                    name_senders(&room, &mut items).await;
                    let mut p = paging.lock().unwrap();
                    let known: std::collections::HashSet<_> = p.items.iter().map(|i| i.event_id.clone()).collect();
                    p.items.extend(items.into_iter().filter(|i| !known.contains(&i.event_id)));
                    (p.next, p.done) = (next, done);
                    state(&p, MoreHits::Available)
                }
                Err(()) => state(&paging.lock().unwrap(), MoreHits::Failed),
            };
            tx.send_replace(next);
        }));
    }
}

impl Drop for SharedMedia {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

async fn name_senders(room: &matrix_sdk::Room, items: &mut [SharedItem]) {
    for item in items {
        let member = match matrix_sdk::ruma::UserId::parse(&item.sender) {
            Ok(user) => room.get_member_no_sync(&user).await.ok().flatten(),
            Err(_) => None,
        };
        item.sender_name = member.and_then(|m| m.display_name().map(str::to_owned)).unwrap_or_else(|| item.sender.clone());
    }
}

/// The kind of an error, never its text: that can carry a room ID, a URL or a name.
fn error_kind(e: &matrix_sdk::Error) -> &'static str {
    match e {
        matrix_sdk::Error::Http(_) => "http",
        matrix_sdk::Error::DecryptorError(_) => "decryption",
        matrix_sdk::Error::StateStore(_) | matrix_sdk::Error::EventCacheStore(_) => "store",
        _ => "other",
    }
}
