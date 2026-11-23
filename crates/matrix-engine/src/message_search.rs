//! Message search, the single service behind the chat header's lens (ONE room) and the room list's
//! «Messaggi» section (ALL rooms). Unencrypted rooms of a one-room search ask the server (`POST /search`,
//! `room_events`); everything else (encrypted rooms, unknown state, all rooms) reads ONLY the local
//! message index (`search_index`), and the term never leaves the device. Built like `UserSearch`:
//! debounce, abort + generation, nothing here logs the term, an excerpt or an event ID.
use crate::operations::{MessageHit, MessageSearchState, MessageSource, MoreHits};
use matrix_sdk::{Client, ruma::{OwnedRoomId, RoomId}};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

#[cfg(test)]
#[path = "message_search_tests.rs"]
mod tests;

pub const DEBOUNCE: Duration = Duration::from_millis(275);
pub use crate::message_match::MIN_CHARS;
/// Wait after the index grew before the open answer is asked again (the SDK indexes a little after the cache).
pub const REFRESH: Duration = Duration::from_millis(1500);
pub const REFRESH_MAX: Duration = Duration::from_secs(6);
pub const PAGE: usize = 20;
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// One room's search: how long, and how many events, the history is paged back before the index is read.
const FILL_TIMEOUT: Duration = Duration::from_secs(10);
const FILL_EVENTS: u16 = 2000;
/// Newest matches taken from each room's index, and kept of all rooms together.
const PER_ROOM: usize = 30;
const ROOM_HITS: usize = 200;
const ALL_HITS: usize = 300;
const EXCERPT_CHARS: usize = 200;

pub use crate::message_match::find_ranges;

fn one_line(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the words of `term` are in `text`, in order and without overlaps (a hit is highlighted word by word).
pub fn word_ranges(text: &str, term: &str) -> Vec<std::ops::Range<usize>> {
    let mut all: Vec<_> = term.split_whitespace().flat_map(|w| find_ranges(text, w)).collect();
    all.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for r in all {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Every word of `term` is somewhere in `text`.
fn has_all_words(text: &str, term: &str) -> bool {
    term.split_whitespace().all(|w| !find_ranges(text, w).is_empty())
}

/// At most `EXCERPT_CHARS` characters of `line`, starting a little before the first match.
fn excerpt_of(line: &str, term: &str) -> String {
    if line.chars().count() <= EXCERPT_CHARS {
        return line.to_owned();
    }
    let first = word_ranges(line, term).first().map_or(0, |r| line[..r.start].chars().count());
    let skip = first.saturating_sub(40);
    let body: String = line.chars().skip(skip).take(EXCERPT_CHARS).collect();
    if skip > 0 { format!("…{body}") } else { body }
}

fn hit_of(event: &Value, body: &str, term: &str, ts: u64) -> Option<MessageHit> {
    let line = one_line(body);
    let excerpt = excerpt_of(&line, term);
    let relates = &event["content"]["m.relates_to"];
    Some(MessageHit {
        room_id: String::new(),
        event_id: event["event_id"].as_str()?.to_owned(),
        sender: event["sender"].as_str()?.to_owned(),
        sender_name: String::new(),
        ts,
        matches: word_ranges(&excerpt, term),
        excerpt,
        thread_root: (relates["rel_type"] == "m.thread")
            .then(|| relates["event_id"].as_str().map(str::to_owned))
            .flatten(),
    })
}

fn is_edit(event: &Value) -> bool {
    event["content"]["m.relates_to"]["rel_type"] == "m.replace"
}

/// Hits of one /search page. The server decides what matches (stemming, prefixes), so no local
/// «contains» check; edits (`m.replace`) are the same message as their original and never listed.
pub fn server_hits(results: &[Value], term: &str) -> Vec<MessageHit> {
    let mut seen = HashSet::new();
    results
        .iter()
        .filter(|e| e["type"] == "m.room.message" && !is_edit(e))
        .filter_map(|e| hit_of(e, e["content"]["body"].as_str()?, term, e["origin_server_ts"].as_u64()?))
        .filter(|h| seen.insert(h.event_id.clone()))
        .collect()
}

/// Hits among the given events (an indexed message with its edits and redactions), newest first: latest valid edit instead of the
/// original, redacted messages out, a message once.
pub fn local_hits(events: &[Value], term: &str) -> Vec<MessageHit> {
    let redacted: HashSet<&str> = events
        .iter()
        .filter(|e| e["type"] == "m.room.redaction")
        .filter_map(|e| e["redacts"].as_str().or_else(|| e["content"]["redacts"].as_str()))
        .collect();
    let senders: HashMap<&str, &str> = events
        .iter()
        .filter_map(|e| Some((e["event_id"].as_str()?, e["sender"].as_str()?)))
        .collect();
    // Newest edit per original, by the original's sender only (anything else is not an edit).
    let mut edits: HashMap<&str, (u64, &str)> = HashMap::new();
    for e in events.iter().filter(|e| e["type"] == "m.room.message" && is_edit(e)) {
        let (Some(of), Some(ts), Some(body)) = (
            e["content"]["m.relates_to"]["event_id"].as_str(),
            e["origin_server_ts"].as_u64(),
            e["content"]["m.new_content"]["body"].as_str(),
        ) else {
            continue;
        };
        if senders.get(of) == e["sender"].as_str().as_ref() && edits.get(of).is_none_or(|(t, _)| *t < ts) {
            edits.insert(of, (ts, body));
        }
    }
    let mut seen = HashSet::new();
    let mut hits: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "m.room.message" && !is_edit(e))
        .filter(|e| !e["event_id"].as_str().is_some_and(|id| redacted.contains(id)))
        .filter(|e| e["unsigned"]["redacted_because"].is_null())
        .filter_map(|e| {
            let body = match edits.get(e["event_id"].as_str()?) {
                Some((_, edited)) => edited,
                None => e["content"]["body"].as_str()?,
            };
            if !has_all_words(&one_line(body), term) {
                return None;
            }
            hit_of(e, body, term, e["origin_server_ts"].as_u64()?)
        })
        .filter(|h| seen.insert(h.event_id.clone()))
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.ts));
    hits
}

/// What later pages continue from.
#[derive(Default)]
struct Paging {
    term: String,
    source: Option<MessageSource>,
    /// Server: `next_batch` of the last page.
    next: Option<String>,
    /// Local cache: hits not yet served.
    queue: Vec<MessageHit>,
    shown: Vec<MessageHit>,
    /// Local cache: the start of the room was not reached, older messages may be missing.
    partial: bool,
}

pub struct MessageSearch {
    client: Client,
    /// `None`: all rooms.
    room_id: Option<OwnedRoomId>,
    term: String,
    generation: Arc<Mutex<u64>>,
    tx: Arc<watch::Sender<MessageSearchState>>,
    paging: Arc<Mutex<Paging>>,
    task: Option<JoinHandle<()>>,
}

type Page = (Vec<MessageHit>, Option<String>);

impl MessageSearch {
    /// Fixed scope: one room. Another room means another `MessageSearch` (the old one aborts on drop).
    pub fn for_room(client: Client, room_id: OwnedRoomId) -> Self {
        Self::scoped(client, Some(room_id))
    }

    /// Every joined room, from the local index.
    pub fn everywhere(client: Client) -> Self {
        Self::scoped(client, None)
    }

    fn scoped(client: Client, room_id: Option<OwnedRoomId>) -> Self {
        Self {
            client,
            room_id,
            term: String::new(),
            generation: Arc::default(),
            tx: Arc::new(watch::channel(MessageSearchState::Idle).0),
            paging: Arc::default(),
            task: None,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<MessageSearchState> {
        self.tx.subscribe()
    }

    /// Same term: nothing. Under `MIN_CHARS`: `Idle`, no request.
    pub fn set_query(&mut self, term: &str) {
        let term = term.trim().to_owned();
        if term == self.term {
            return;
        }
        self.term = term.clone();
        let gen_now = self.restart(|| {
            if term.chars().count() < MIN_CHARS {
                MessageSearchState::Idle
            } else {
                MessageSearchState::Searching { term: term.clone() }
            }
        });
        if term.chars().count() < MIN_CHARS {
            return;
        }
        self.launch(term, gen_now, DEBOUNCE, None);
    }

    /// The index grew (backfill, new messages): the same term again, in place and without a spinner. Only when the
    /// answer is a first page that nobody extended (what a user paged through stays).
    /// shortcut: a refresh shows the first page again, upgrade when paging back needs to survive.
    /// True: a search is still running, ask again later.
    pub fn refresh(&mut self) -> bool {
        let state = self.tx.borrow().clone();
        let term = match &state {
            MessageSearchState::NoResults { term, .. } => term.clone(),
            MessageSearchState::Results { term, hits, more: MoreHits::None | MoreHits::Available, .. } if hits.len() <= PAGE => term.clone(),
            MessageSearchState::Searching { .. } => return true,
            _ => return false,
        };
        let gen_now = self.restart(|| state.clone());
        self.launch(term, gen_now, Duration::ZERO, Some(state));
        false
    }

    /// `keep`: what stays on screen when the search fails (a refresh must not turn a list into an error).
    fn launch(&mut self, term: String, gen_now: u64, debounce: Duration, keep: Option<MessageSearchState>) {
        let (client, room_id, paging) = (self.client.clone(), self.room_id.clone(), self.paging.clone());
        let (generation, tx) = (self.generation.clone(), self.tx.clone());
        self.task = Some(tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            // Decided on every query: a room can become encrypted. Unknown never reaches the server.
            let source = match &room_id {
                Some(id) => source_of(&client, id).await,
                None => MessageSource::LocalCache,
            };
            // One room from the index: page its history back first (sync can leave a gap), indexing it.
            let partial = match (&room_id, source) {
                (Some(id), MessageSource::LocalCache) => !crate::search_index::fill(&client, id, FILL_EVENTS, FILL_TIMEOUT).await,
                _ => false,
            };
            let first = tokio::time::timeout(TIMEOUT, async {
                match (source, &room_id) {
                    (MessageSource::Server, Some(id)) => server_page(&client, id, &term, None).await,
                    _ => indexed(&client, room_id.as_deref(), &term).await,
                }
            })
            .await
            .unwrap_or(Err(()));
            let state = match first {
                Err(()) => keep.unwrap_or(MessageSearchState::Error { term: term.clone() }),
                Ok((hits, _)) if hits.is_empty() => MessageSearchState::NoResults { term: term.clone(), source },
                Ok((mut hits, next)) => {
                    let mut p = Paging { term: term.clone(), source: Some(source), next, partial, ..Paging::default() };
                    if source == MessageSource::LocalCache {
                        p.queue = hits.split_off(hits.len().min(PAGE));
                    }
                    name_senders(&client, &mut hits).await;
                    p.shown = hits;
                    let state = results(&p);
                    *paging.lock().unwrap() = p;
                    state
                }
            };
            publish(&generation, &tx, gen_now, state);
        }));
    }

    /// Only in `Results` with `Available` or `Failed`: the next page, no debounce, hits appended.
    pub fn more(&mut self) {
        if !matches!(
            *self.tx.borrow(),
            MessageSearchState::Results { more: MoreHits::Available | MoreHits::Failed, .. }
        ) {
            return;
        }
        let mut p = {
            let p = self.paging.lock().unwrap();
            Paging { term: p.term.clone(), source: p.source, next: p.next.clone(), queue: p.queue.clone(), shown: p.shown.clone(), partial: p.partial }
        };
        let gen_now = self.restart(|| {
            let mut loading = results(&p);
            if let MessageSearchState::Results { more, .. } = &mut loading {
                *more = MoreHits::Loading;
            }
            loading
        });
        let (client, room_id, paging) = (self.client.clone(), self.room_id.clone(), self.paging.clone());
        let (generation, tx) = (self.generation.clone(), self.tx.clone());
        self.task = Some(tokio::spawn(async move {
            let page = tokio::time::timeout(TIMEOUT, async {
                match (&p.source, &room_id) {
                    (Some(MessageSource::Server), Some(id)) => server_page(&client, id, &p.term, p.next.clone()).await,
                    _ => Ok((p.queue.drain(..p.queue.len().min(PAGE)).collect(), None)),
                }
            })
            .await
            .unwrap_or(Err(()));
            let state = match page {
                // The shown hits stay; only the tail failed.
                Err(()) => {
                    let mut failed = results(&p);
                    if let MessageSearchState::Results { more, .. } = &mut failed {
                        *more = MoreHits::Failed;
                    }
                    failed
                }
                Ok((mut hits, next)) => {
                    let known: HashSet<_> = p.shown.iter().map(|h| h.event_id.clone()).collect();
                    hits.retain(|h| !known.contains(&h.event_id));
                    name_senders(&client, &mut hits).await;
                    p.shown.extend(hits);
                    if p.source == Some(MessageSource::Server) {
                        p.next = next;
                    }
                    results(&p)
                }
            };
            if !matches!(state, MessageSearchState::Results { more: MoreHits::Failed, .. }) {
                *paging.lock().unwrap() = p;
            }
            publish(&generation, &tx, gen_now, state);
        }));
    }

    /// Aborts the running task, bumps the generation and publishes `state` under the same lock.
    fn restart(&mut self, state: impl FnOnce() -> MessageSearchState) -> u64 {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        let mut g = self.generation.lock().unwrap();
        *g += 1;
        self.tx.send_replace(state());
        *g
    }
}

impl Drop for MessageSearch {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

fn results(p: &Paging) -> MessageSearchState {
    let more = match p.source {
        Some(MessageSource::LocalCache) if !p.queue.is_empty() => MoreHits::Available,
        Some(MessageSource::Server) if p.next.is_some() => MoreHits::Available,
        _ => MoreHits::None,
    };
    MessageSearchState::Results {
        term: p.term.clone(),
        source: p.source.unwrap_or(MessageSource::LocalCache),
        hits: p.shown.clone(),
        more,
        partial: p.partial,
    }
}

/// A task writes only while its generation is the current one.
fn publish(generation: &Mutex<u64>, tx: &watch::Sender<MessageSearchState>, gen_now: u64, state: MessageSearchState) {
    let g = generation.lock().unwrap();
    if *g == gen_now {
        tx.send_replace(state);
    }
}

async fn source_of(client: &Client, room_id: &RoomId) -> MessageSource {
    use matrix_sdk::EncryptionState;
    match client.get_room(room_id) {
        Some(room) => match room.latest_encryption_state().await {
            Ok(EncryptionState::NotEncrypted) => MessageSource::Server,
            _ => MessageSource::LocalCache,
        },
        None => MessageSource::LocalCache,
    }
}

async fn server_page(client: &Client, room_id: &OwnedRoomId, term: &str, next: Option<String>) -> Result<Page, ()> {
    use matrix_sdk::ruma::{
        UInt,
        api::client::search::search_events::v3::{Categories, Criteria, OrderBy, Request, SearchKeys},
    };
    let mut criteria = Criteria::new(term.to_owned());
    criteria.keys = Some(vec![SearchKeys::ContentBody]);
    criteria.filter.rooms = Some(vec![room_id.clone()]);
    criteria.filter.limit = Some(UInt::from(PAGE as u32));
    criteria.order_by = Some(OrderBy::Recent);
    let mut categories = Categories::new();
    categories.room_events = Some(criteria);
    let mut request = Request::new(categories);
    request.next_batch = next;
    let found = client.send(request).await.map_err(|_| ())?.search_categories.room_events;
    let events: Vec<Value> = found
        .results
        .iter()
        .filter_map(|r| serde_json::from_str(r.result.as_ref()?.json().get()).ok())
        .collect();
    let mut hits = server_hits(&events, term);
    hits.iter_mut().for_each(|h| h.room_id = room_id.to_string());
    Ok((hits, found.next_batch))
}

/// Hits of the local index, newest first: the matches of one room, or of every joined room. Each match is
/// read back from the event cache with its edits (the index only says which messages match), so the text
/// shown is the latest edit and a redacted message is not listed.
async fn indexed(client: &Client, room_id: Option<&RoomId>, term: &str) -> Result<Page, ()> {
    use matrix_sdk::ruma::events::relation::RelationType;
    let rooms: Vec<_> = match room_id {
        Some(id) => client.get_room(id).into_iter().collect(),
        None => client.joined_rooms(),
    };
    let (per_room, total) = if room_id.is_some() { (ROOM_HITS, ROOM_HITS) } else { (PER_ROOM, ALL_HITS) };
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for room in rooms {
        let found = room.search(term, per_room, None).await.map_err(|_| ())?;
        if found.is_empty() {
            continue;
        }
        let Ok((cache, _keep)) = client.event_cache().room(room.room_id()).await else { continue };
        for (_, event_id) in found {
            let load = async |id: &matrix_sdk::ruma::EventId| {
                let (event, related) = cache.find_event_with_relations(id, Some(vec![RelationType::Replacement])).await.ok()??;
                Some(std::iter::once(&event).chain(&related).filter_map(|e| serde_json::from_str::<Value>(e.raw().json().get()).ok()).collect::<Vec<_>>())
            };
            let Some(mut events) = load(&event_id).await else { continue };
            // The index keeps an edited message under the ID of its latest edit: the message is the original.
            if let Some(original) = events.first().filter(|e| is_edit(e)).and_then(|e| e["content"]["m.relates_to"]["event_id"].as_str()).and_then(|id| matrix_sdk::ruma::EventId::parse(id).ok()) {
                let Some(whole) = load(&original).await else { continue };
                events = whole;
            }
            for mut hit in local_hits(&events, term) {
                if seen.insert(hit.event_id.clone()) {
                    hit.room_id = room.room_id().to_string();
                    hits.push(hit);
                }
            }
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.ts));
    hits.truncate(total);
    Ok((hits, None))
}

async fn name_senders(client: &Client, hits: &mut [MessageHit]) {
    for hit in hits {
        let room = matrix_sdk::ruma::RoomId::parse(&hit.room_id).ok().and_then(|id| client.get_room(&id));
        let member = match (&room, matrix_sdk::ruma::UserId::parse(&hit.sender)) {
            (Some(room), Ok(user)) => room.get_member_no_sync(&user).await.ok().flatten(),
            _ => None,
        };
        hit.sender_name = member.and_then(|m| m.display_name().map(str::to_owned)).unwrap_or_else(|| hit.sender.clone());
    }
}
