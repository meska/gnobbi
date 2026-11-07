//! Message search of ONE room (the chat header's lens), the single service behind it. Unencrypted
//! rooms ask the server (`POST /search`, `room_events`); encrypted rooms (or an unknown state) read
//! ONLY the SDK event cache, decrypted, and the term never leaves the device. Built like
//! `UserSearch`: debounce, abort + generation, nothing here logs the term, an excerpt or an event ID.
use crate::operations::{MessageHit, MessageSearchState, MessageSource, MoreHits};
use matrix_sdk::{Client, ruma::OwnedRoomId};
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
pub const PAGE: usize = 20;
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// Encrypted rooms: how long, and how many events, the cache is paged back before the search reads it.
const FILL_TIMEOUT: Duration = Duration::from_secs(10);
const FILL_EVENTS: u16 = 2000;
const EXCERPT_CHARS: usize = 200;

pub use crate::message_match::find_ranges;

fn one_line(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `EXCERPT_CHARS` characters of `line`, starting a little before the first match.
fn excerpt_of(line: &str, term: &str) -> String {
    if line.chars().count() <= EXCERPT_CHARS {
        return line.to_owned();
    }
    let first = find_ranges(line, term).first().map_or(0, |r| line[..r.start].chars().count());
    let skip = first.saturating_sub(40);
    let body: String = line.chars().skip(skip).take(EXCERPT_CHARS).collect();
    if skip > 0 { format!("…{body}") } else { body }
}

fn hit_of(event: &Value, body: &str, term: &str, ts: u64) -> Option<MessageHit> {
    let line = one_line(body);
    let excerpt = excerpt_of(&line, term);
    let relates = &event["content"]["m.relates_to"];
    Some(MessageHit {
        event_id: event["event_id"].as_str()?.to_owned(),
        sender: event["sender"].as_str()?.to_owned(),
        sender_name: String::new(),
        ts,
        matches: find_ranges(&excerpt, term),
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

/// Hits among all cached events of the room, newest first: latest valid edit instead of the
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
            if find_ranges(&one_line(body), term).is_empty() {
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
    room_id: OwnedRoomId,
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
        let (client, room_id, paging) = (self.client.clone(), self.room_id.clone(), self.paging.clone());
        let (generation, tx) = (self.generation.clone(), self.tx.clone());
        self.task = Some(tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            // Decided on every query: a room can become encrypted. Unknown never reaches the server.
            let source = source_of(&client, &room_id).await;
            // The cache is all an encrypted room has: page it back to the room start first (sync can leave a gap).
            let partial = source == MessageSource::LocalCache && !fill_cache(&client, &room_id).await;
            let first = tokio::time::timeout(TIMEOUT, async {
                match source {
                    MessageSource::Server => server_page(&client, &room_id, &term, None).await,
                    MessageSource::LocalCache => local_all(&client, &room_id, &term).await,
                }
            })
            .await
            .unwrap_or(Err(()));
            let state = match first {
                Err(()) => MessageSearchState::Error { term: term.clone() },
                Ok((hits, _)) if hits.is_empty() => MessageSearchState::NoResults { term: term.clone(), source },
                Ok((mut hits, next)) => {
                    let mut p = Paging { term: term.clone(), source: Some(source), next, partial, ..Paging::default() };
                    if source == MessageSource::LocalCache {
                        p.queue = hits.split_off(hits.len().min(PAGE));
                    }
                    name_senders(&client, &room_id, &mut hits).await;
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
                match p.source {
                    Some(MessageSource::Server) => server_page(&client, &room_id, &p.term, p.next.clone()).await,
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
                    name_senders(&client, &room_id, &mut hits).await;
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

async fn source_of(client: &Client, room_id: &matrix_sdk::ruma::RoomId) -> MessageSource {
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
    Ok((server_hits(&events, term), found.next_batch))
}

/// Pages the room's event cache back to the room start (at most `FILL_EVENTS` events, `FILL_TIMEOUT`).
/// True only when the start was reached: the cache then holds the whole room. Offline or failing: false,
/// and the search still reads what the cache has.
async fn fill_cache(client: &Client, room_id: &OwnedRoomId) -> bool {
    let Some(room) = client.get_room(room_id) else { return false };
    let Ok((cache, _keep)) = room.event_cache().await else { return false };
    let paging = tokio::time::timeout(FILL_TIMEOUT, cache.pagination().run_backwards_until(FILL_EVENTS)).await;
    matches!(paging, Ok(Ok(outcome)) if outcome.reached_start)
}

async fn local_all(client: &Client, room_id: &OwnedRoomId, term: &str) -> Result<Page, ()> {
    let events = {
        let guard = client.event_cache_store().lock().await.map_err(|_| ())?;
        guard.as_clean().ok_or(())?.get_room_events(room_id, None, None).await.map_err(|_| ())?
    };
    let events: Vec<Value> = events.iter().filter_map(|e| serde_json::from_str(e.raw().json().get()).ok()).collect();
    Ok((local_hits(&events, term), None))
}

async fn name_senders(client: &Client, room_id: &OwnedRoomId, hits: &mut [MessageHit]) {
    let room = client.get_room(room_id);
    for hit in hits {
        let member = match (&room, matrix_sdk::ruma::UserId::parse(&hit.sender)) {
            (Some(room), Ok(user)) => room.get_member_no_sync(&user).await.ok().flatten(),
            _ => None,
        };
        hit.sender_name = member.and_then(|m| m.display_name().map(str::to_owned)).unwrap_or_else(|| hit.sender.clone());
    }
}
