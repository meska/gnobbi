//! The account's message index: the SDK's search index (tantivy, one per room) in a directory next to
//! the account store, encrypted at rest with the store's own passphrase (AES-256-CTR + HMAC per file,
//! `EncryptedDirectory`). The SDK feeds it with every event that enters the event cache; this module
//! adds what the SDK does not do: indexing what the cache already holds, and a bounded, interruptible
//! backfill of the room history. Logout removes the directory with the rest of the account.
//! Nothing here logs a term, an excerpt or an event ID.
use matrix_sdk::{Client, deserialized_responses::TimelineEvent, ruma::RoomId, search_index::SearchIndexStoreKind};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// Events one room's backfill pass asks the server for, and how long it may take.
const BACKFILL_EVENTS: u16 = 300;
const BACKFILL_TIMEOUT: Duration = Duration::from_secs(15);
/// Rooms of one pass (most recent first) and the pause between two rooms: the pass never competes with the user.
const BACKFILL_ROOMS: usize = 60;
const BACKFILL_PAUSE: Duration = Duration::from_millis(300);
/// Marker file: `w <room>` = the cache of the room was indexed, `d <room>` = its history was fully fetched.
const MARKERS: &str = "rooms.txt";

/// Where the SDK keeps the index: encrypted on disk. Windows cannot name a directory after a room ID
/// (`!a:b`), so there it stays in memory and is built again at each start.
pub fn store_kind(dir: &Path, passphrase: &str) -> SearchIndexStoreKind {
    if cfg!(windows) {
        return SearchIndexStoreKind::InMemory;
    }
    SearchIndexStoreKind::EncryptedDirectory(dir.to_owned(), passphrase.to_owned())
}

/// Adds the events to the index of `room_id` (what is already there is skipped).
pub async fn index_events(client: &Client, room_id: &RoomId, events: Vec<TimelineEvent>) -> bool {
    if events.is_empty() {
        return true;
    }
    let (Some(room), Ok((cache, _keep))) = (client.get_room(room_id), client.event_cache().room(room_id).await) else {
        return false;
    };
    let rules = room.clone_info().room_version_rules_or_default().redaction;
    client.search_index().lock().await.bulk_handle_timeline_event(events.into_iter(), &cache, room_id, &rules).await.is_ok()
}

/// Indexes every event the event cache holds for the room.
async fn index_saved(client: &Client, room_id: &RoomId) -> bool {
    match crate::room_sync::stored_events(client, room_id).await {
        Ok((events, ..)) => index_events(client, room_id, events).await,
        Err(_) => false,
    }
}

/// Pages the room's history back until `events` more events came or the room start was reached
/// (at most `timeout`), indexing them. True only when the room start was reached. Offline or
/// failing: false.
pub async fn fill(client: &Client, room_id: &RoomId, events: u16, timeout: Duration) -> bool {
    let Some(room) = client.get_room(room_id) else { return false };
    let fetch = async {
        let (cache, _keep) = room.event_cache().await.ok()?;
        cache.pagination().run_backwards_until(events).await.ok()
    };
    let Ok(Some(outcome)) = tokio::time::timeout(timeout, fetch).await else { return false };
    // The SDK indexes these on its own too, a little later: doing it here makes the search that follows complete.
    index_events(client, room_id, outcome.events).await && outcome.reached_start
}

/// What was already done for each room, kept next to the index (the marker file goes with it at logout).
struct Markers {
    file: Option<PathBuf>,
    done: HashSet<String>,
}

impl Markers {
    fn open(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(MARKERS));
        let done = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        Self { file, done }
    }

    fn has(&self, kind: char, room: &RoomId) -> bool {
        self.done.contains(&format!("{kind} {room}"))
    }

    fn set(&mut self, kind: char, room: &RoomId) {
        let line = format!("{kind} {room}");
        if let Some(file) = &self.file
            && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file)
        {
            let _ = writeln!(f, "{line}");
        }
        self.done.insert(line);
    }
}

/// Background pass after the first sync: for the most recent rooms, index what the cache holds (once)
/// and fetch the older history (`BACKFILL_EVENTS` per pass, until the room start). One room at a time,
/// a pause between rooms, ends at once when `stop` is set (account closing, or the app going to
/// the background on iOS, which stops the whole worker). Returns how many rooms it worked on.
pub async fn backfill(client: &Client, dir: Option<&Path>, stop: &AtomicBool) -> usize {
    let mut markers = Markers::open(dir);
    let mut rooms: Vec<_> = client.joined_rooms().into_iter().map(|r| (r.recency_stamp().map_or(0, u64::from), r.room_id().to_owned())).collect();
    rooms.sort_by_key(|(recency, _)| std::cmp::Reverse(*recency));
    let mut worked = 0;
    for (_, id) in rooms.into_iter().take(BACKFILL_ROOMS) {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let mut did = false;
        if !markers.has('w', &id) {
            if index_saved(client, &id).await {
                markers.set('w', &id);
            }
            did = true;
        }
        if !markers.has('d', &id) && !stop.load(Ordering::Acquire) {
            if fill(client, &id, BACKFILL_EVENTS, BACKFILL_TIMEOUT).await {
                markers.set('d', &id);
            }
            did = true;
        }
        if did {
            worked += 1;
            tokio::time::sleep(BACKFILL_PAUSE).await;
        }
    }
    worked
}
