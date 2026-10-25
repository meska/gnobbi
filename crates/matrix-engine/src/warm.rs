//! Background warm-up of the event cache for the rooms the user most likely opens first.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) const RECENT: usize = 10;
/// One server page; a room that already holds this many saved messages opens full.
const PAGE: u16 = 25;

/// Declared size limit of the saved event cache of one account.
pub(crate) const CACHE_LIMIT: usize = 256 * 1024 * 1024;

/// Clears the saved event cache when it grew over `limit` bytes (the next open refills it).
/// Returns whether it cleared.
pub(crate) async fn prune(client: &matrix_sdk::Client, limit: usize) -> bool {
    use matrix_sdk::cross_process_lock::MappedCrossProcessLockState::{Clean, Dirty};
    // The store lock is released before clearing, which takes it again.
    let size = match client.event_cache_store().lock().await {
        Ok(Clean(store) | Dirty(store)) => store.get_size().await.ok().flatten(),
        Err(_) => None,
    };
    if size.is_none_or(|size| size <= limit) {
        return false;
    }
    client.event_cache().clear_all_rooms().await.is_ok()
}

/// Rooms to warm: those with unread (newest first, at most `recent`), then the `recent` newest.
pub(crate) fn pick<T: Clone, R: Ord + Clone>(rooms: &[(T, R, bool)], recent: usize) -> Vec<T> {
    let mut by_recency: Vec<&(T, R, bool)> = rooms.iter().collect();
    by_recency.sort_by(|a, b| b.1.cmp(&a.1));
    let unread = by_recency.iter().filter(|r| r.2).take(recent);
    let mut picked: Vec<&(T, R, bool)> = Vec::new();
    for room in unread.chain(by_recency.iter().take(recent)) {
        if !picked.iter().any(|p| std::ptr::eq(*p, *room)) {
            picked.push(room);
        }
    }
    picked.into_iter().map(|r| r.0.clone()).collect()
}

/// Fill the saved history of the picked rooms, one at a time, on the backend runtime. Stops
/// at once when `stop` (account closing) or `interrupt` (the user opened a room) is set.
/// Returns how many rooms it filled.
pub(crate) async fn warm(client: &matrix_sdk::Client, stop: &AtomicBool, interrupt: &AtomicBool) -> usize {
    prune(client, CACHE_LIMIT).await;
    let rooms: Vec<_> = client
        .joined_rooms()
        .into_iter()
        .map(|r| {
            let unread = r.num_unread_messages() > 0 || r.num_unread_notifications() > 0 || r.is_marked_unread();
            (r.room_id().to_owned(), r.recency_stamp().map_or(0, u64::from), unread)
        })
        .collect();
    let mut warmed = 0;
    for id in pick(&rooms, RECENT) {
        if stop.load(Ordering::Acquire) || interrupt.load(Ordering::Acquire) {
            break;
        }
        let Some(room) = client.get_room(&id) else { continue };
        let saved = crate::room_sync::stored_events(client, &id).await.map_or(0, |s| s.0.len());
        if saved >= usize::from(PAGE) {
            continue;
        }
        let fill = async {
            let (events, _drop) = room.event_cache().await.ok()?;
            events.pagination().run_backwards_once(PAGE).await.ok()
        };
        if matches!(tokio::time::timeout(Duration::from_secs(15), fill).await, Ok(Some(_))) {
            warmed += 1;
        }
    }
    warmed
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn picks_unread_first_then_newest_without_repeats() {
        // (id, recency, unread)
        let rooms: Vec<(u32, u32, bool)> = (0..30).map(|n| (n, n, n == 3 || n == 5)).collect();
        let got = pick(&rooms, 4);
        assert_eq!(got, vec![5, 3, 29, 28, 27, 26]);
    }

    #[test]
    fn caps_unread_rooms_too() {
        let rooms: Vec<(u32, u32, bool)> = (0..40).map(|n| (n, n, true)).collect();
        assert_eq!(pick(&rooms, 3), vec![39, 38, 37]);
    }

    #[test]
    fn small_account_returns_every_room_once() {
        let rooms = vec![(1u32, 1u32, false), (2, 2, true)];
        assert_eq!(pick(&rooms, 10), vec![2, 1]);
    }
}
