use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::media::{MediaCache, MediaKey};

fn key(account: u64, event: &str) -> MediaKey {
    MediaKey {
        scope: RoomScope {
            account_id: AccountId::new(account),
            room_id: RoomId::new("!media:test"),
        },
        event_id: EventId::new(event),
    }
}

#[test]
fn logout_clears_only_requested_accounts_cached_bytes() {
    let mut cache = MediaCache::new(16);
    cache.insert(key(1, "$same"), vec![1; 4]);
    cache.insert(key(2, "$same"), vec![2; 4]);
    cache.clear_account(AccountId::new(1));
    assert!(cache.get(&key(1, "$same")).is_none());
    assert_eq!(cache.get(&key(2, "$same")), Some(&[2, 2, 2, 2][..]));
    assert_eq!(cache.used_bytes(), 4);
}

#[test]
fn cache_rejects_oversized_and_empty_entries_without_evicting_existing_data() {
    let mut cache = MediaCache::new(3);
    assert!(cache.insert(key(1, "$a"), vec![1; 3]));
    assert!(!cache.insert(key(1, "$a"), vec![2; 4]));
    assert!(!cache.insert(key(1, "$b"), vec![]));
    assert_eq!(cache.used_bytes(), 3);
    assert_eq!(cache.get(&key(1, "$a")), Some(&[1, 1, 1][..]));
    assert!(cache.insert(key(1, "$a"), vec![2; 2]));
    assert_eq!(cache.used_bytes(), 2);
    assert_eq!(cache.get(&key(1, "$a")), Some(&[2, 2][..]));
    assert!(!MediaCache::new(0).insert(key(1, "$a"), vec![1]));
}

#[test]
fn cache_evicts_least_recently_used_bytes_within_budget() {
    let mut cache = MediaCache::new(6);
    assert!(cache.insert(key(1, "$a"), vec![1; 3]));
    assert!(cache.insert(key(1, "$b"), vec![2; 3]));
    assert_eq!(cache.get(&key(1, "$a")), Some(&[1, 1, 1][..]));
    assert!(cache.insert(key(1, "$c"), vec![3; 2]));
    assert!(cache.get(&key(1, "$b")).is_none());
    assert_eq!(cache.used_bytes(), 5);
    assert_eq!(cache.get(&key(1, "$a")), Some(&[1, 1, 1][..]));
}
