//! Exact public receipt markers, not inferred reading of earlier events.
use matrix_domain::EventId;

#[derive(Clone, PartialEq, Eq)]
pub struct PublicReadReceipt {
    pub event_id: EventId,
    pub readers: u64,
}

#[cfg(any(feature = "sdk", test))]
pub(crate) struct PeerReceipts {
    account: matrix_domain::AccountId,
    own_user: String,
    rooms: std::collections::HashMap<
        matrix_domain::RoomId,
        std::collections::HashMap<String, EventId>,
    >,
}

#[cfg(any(feature = "sdk", test))]
impl PeerReceipts {
    pub(crate) fn new(account: matrix_domain::AccountId, own_user: String) -> Self {
        Self {
            account,
            own_user,
            rooms: Default::default(),
        }
    }

    #[cfg(feature = "sdk")]
    pub(crate) fn observe_event(
        &mut self,
        scope: &matrix_domain::RoomScope,
        raw: &matrix_sdk::ruma::serde::Raw<matrix_sdk::ruma::events::AnySyncEphemeralRoomEvent>,
    ) {
        use matrix_sdk::ruma::events::{
            AnySyncEphemeralRoomEvent,
            receipt::{ReceiptThread, ReceiptType},
        };
        if let Ok(AnySyncEphemeralRoomEvent::Receipt(event)) = raw.deserialize() {
            for (event_id, types) in event.content {
                if let Some(peers) = types.get(&ReceiptType::Read) {
                    for (peer, receipt) in peers {
                        // shortcut: unthreaded markers only, add separate thread state when threads are supported.
                        if receipt.thread == ReceiptThread::Unthreaded {
                            self.observe(scope, peer.as_str(), &EventId::new(event_id.as_str()));
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn observe(
        &mut self,
        scope: &matrix_domain::RoomScope,
        peer: &str,
        event: &EventId,
    ) -> bool {
        if scope.account_id != self.account || peer == self.own_user {
            return false;
        }
        let peers = self.rooms.entry(scope.room_id.clone()).or_default();
        if peers.get(peer) == Some(event) {
            return false;
        }
        if peers.len() >= 128 && !peers.contains_key(peer) {
            return false;
        }
        peers.insert(peer.to_owned(), event.clone());
        true
    }

    pub(crate) fn snapshot(&self, scope: &matrix_domain::RoomScope) -> Vec<PublicReadReceipt> {
        if scope.account_id != self.account {
            return Vec::new();
        }
        let mut counts = std::collections::BTreeMap::<&str, (EventId, u64)>::new();
        if let Some(peers) = self.rooms.get(&scope.room_id) {
            for event in peers.values() {
                counts
                    .entry(event.as_str())
                    .or_insert_with(|| (event.clone(), 0))
                    .1 += 1;
            }
        }
        counts
            .into_values()
            .map(|(event_id, readers)| PublicReadReceipt { event_id, readers })
            .collect()
    }

    pub(crate) fn retain_rooms(&mut self, rooms: &[matrix_domain::RoomId]) {
        self.rooms.retain(|room, _| rooms.contains(room));
    }
}

/// What the user can see right now; receipts leave only when the newest message is really in view.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReadVisibility {
    pub window_focused: bool,
    pub room_foreground: bool,
    /// Pixels between the viewport bottom and the end of the timeline.
    pub bottom_gap_px: f32,
    /// «Conferme di lettura» setting: off sends the private receipt instead of the public one.
    pub public_receipts: bool,
}

impl ReadVisibility {
    /// Further than this from the bottom, the newest message is not in view.
    pub const NEAR_BOTTOM_PX: f32 = 200.0;

    pub fn sees_last_message(&self) -> bool {
        self.window_focused && self.room_foreground && self.bottom_gap_px <= Self::NEAR_BOTTOM_PX
    }
}

/// Newest-event markers: fully_read always; the read receipt is public or, with the setting off, private.
#[cfg(feature = "sdk")]
pub(crate) fn marker_receipts(
    event: matrix_sdk::ruma::OwnedEventId,
    public: bool,
) -> matrix_sdk::room::Receipts {
    let receipts = matrix_sdk::room::Receipts::new().fully_read_marker(event.clone());
    if public {
        receipts.public_read_receipt(event)
    } else {
        receipts.private_read_receipt(event)
    }
}

/// Last event we marked as read per room; a failed send is not recorded, so the next open retries it.
#[cfg(any(feature = "sdk", test))]
#[derive(Default)]
pub(crate) struct SentMarkers(std::collections::HashMap<matrix_domain::RoomId, EventId>);

#[cfg(any(feature = "sdk", test))]
impl SentMarkers {
    pub(crate) fn is_new(&self, room: &matrix_domain::RoomId, event: &EventId) -> bool {
        self.0.get(room) != Some(event)
    }

    pub(crate) fn record(&mut self, room: matrix_domain::RoomId, event: EventId) {
        self.0.insert(room, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, EventId, RoomId, RoomScope};

    #[cfg(feature = "sdk")]
    #[test]
    fn sync_event_keeps_only_valid_unthreaded_public_peer_markers() {
        use matrix_sdk::ruma::{events::AnySyncEphemeralRoomEvent, serde::Raw};
        let scope = RoomScope {
            account_id: AccountId::new(54),
            room_id: RoomId::new("!a:test"),
        };
        let mut receipts = PeerReceipts::new(scope.account_id, "@alice:test".into());
        for value in [
            serde_json::json!({"type":"m.receipt","content":{"$public":{"m.read":{"@bob:test":{},"@alice:test":{}}}}}),
            serde_json::json!({"type":"m.receipt","content":{"$private":{"m.read.private":{"@bob:test":{}}}}}),
            serde_json::json!({"type":"m.receipt","content":{"$thread":{"m.read":{"@bob:test":{"thread_id":"$root"}}}}}),
            serde_json::json!({"type":"m.receipt","content":{"$main":{"m.read":{"@bob:test":{"thread_id":"main"}}}}}),
            serde_json::json!({"type":"m.receipt","content":{"$custom":{"custom":{"@bob:test":{}}}}}),
            serde_json::json!({"type":"m.receipt","content":{"not-event":{"m.read":{"@bob:test":{}}}}}),
            serde_json::json!({"type":"m.receipt"}),
            serde_json::json!({"type":"m.typing","content":{"user_ids":["@bob:test"]}}),
        ] {
            let raw =
                Raw::<AnySyncEphemeralRoomEvent>::from_json_string(value.to_string()).unwrap();
            receipts.observe_event(&scope, &raw);
        }
        assert!(
            receipts.snapshot(&scope)
                == vec![PublicReadReceipt {
                    event_id: EventId::new("$public"),
                    readers: 1
                }]
        );
    }

    #[test]
    fn bounded_peer_cache_keeps_existing_peers_updatable_at_capacity() {
        let scope = RoomScope {
            account_id: AccountId::new(44),
            room_id: RoomId::new("!a:test"),
        };
        let mut receipts = PeerReceipts::new(scope.account_id, "@alice:test".into());
        for index in 0..128 {
            assert!(receipts.observe(
                &scope,
                &format!("@peer{index}:test"),
                &EventId::new("$first")
            ));
        }
        assert!(!receipts.observe(&scope, "@overflow:test", &EventId::new("$overflow")));
        assert!(receipts.observe(&scope, "@peer0:test", &EventId::new("$new")));
        let snapshot = receipts.snapshot(&scope);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.iter().map(|r| r.readers).sum::<u64>(), 128);
        assert!(!snapshot.iter().any(|r| r.event_id.as_str() == "$overflow"));
    }

    #[test]
    fn peer_markers_are_scoped_deduplicated_and_never_inferred_from_own_marker() {
        let scope = RoomScope {
            account_id: AccountId::new(44),
            room_id: RoomId::new("!a:test"),
        };
        let event = EventId::new("$a");
        let mut receipts = PeerReceipts::new(scope.account_id, "@alice:test".into());
        assert!(!receipts.observe(&scope, "@alice:test", &event));
        assert!(receipts.observe(&scope, "@bob:test", &event));
        assert!(!receipts.observe(&scope, "@bob:test", &event));
        assert!(
            receipts.snapshot(&scope)
                == vec![PublicReadReceipt {
                    event_id: event.clone(),
                    readers: 1
                }]
        );
        let other = RoomScope {
            room_id: RoomId::new("!b:test"),
            ..scope.clone()
        };
        assert!(receipts.snapshot(&other).is_empty());
        let foreign = RoomScope {
            account_id: AccountId::new(99),
            ..scope.clone()
        };
        assert!(!receipts.observe(&foreign, "@mallory:test", &event));
        assert!(receipts.snapshot(&foreign).is_empty());
        assert!(receipts.observe(&scope, "@carol:test", &event));
        assert_eq!(receipts.snapshot(&scope)[0].readers, 2);
        let newer = EventId::new("$b");
        assert!(receipts.observe(&scope, "@bob:test", &newer));
        assert!(
            receipts.snapshot(&scope)
                == vec![
                    PublicReadReceipt {
                        event_id: event,
                        readers: 1
                    },
                    PublicReadReceipt {
                        event_id: newer,
                        readers: 1
                    }
                ]
        );
        receipts.retain_rooms(&[]);
        assert!(receipts.snapshot(&scope).is_empty());
    }

    #[test]
    fn last_message_is_seen_only_with_focus_foreground_room_and_timeline_near_bottom() {
        let seen = ReadVisibility {
            window_focused: true,
            room_foreground: true,
            bottom_gap_px: 0.0,
            public_receipts: true,
        };
        assert!(seen.sees_last_message());
        assert!(
            ReadVisibility {
                bottom_gap_px: ReadVisibility::NEAR_BOTTOM_PX,
                ..seen
            }
            .sees_last_message()
        );
        for hidden in [
            ReadVisibility { window_focused: false, ..seen },
            ReadVisibility { room_foreground: false, ..seen },
            ReadVisibility { bottom_gap_px: 5000.0, ..seen },
        ] {
            assert!(!hidden.sees_last_message());
        }
    }

    #[test]
    fn sent_marker_dedupes_per_event_and_failure_is_retried() {
        let (room, a, b) = (
            RoomId::new("!a:test"),
            EventId::new("$a"),
            EventId::new("$b"),
        );
        let mut sent = SentMarkers::default();
        assert!(sent.is_new(&room, &a));
        // failed send: nothing recorded, the next open tries again
        assert!(sent.is_new(&room, &a));
        sent.record(room.clone(), a.clone());
        assert!(!sent.is_new(&room, &a), "same event is not sent twice");
        assert!(sent.is_new(&room, &b), "a newer event is sent");
    }

    #[cfg(feature = "sdk")]
    #[test]
    fn setting_off_sends_the_private_receipt_not_the_public_one() {
        let id = matrix_sdk::ruma::event_id!("$e:x").to_owned();
        let on = marker_receipts(id.clone(), true);
        assert_eq!((on.public_read_receipt.as_ref(), on.private_read_receipt.as_ref()), (Some(&id), None));
        let off = marker_receipts(id.clone(), false);
        assert_eq!((off.public_read_receipt.as_ref(), off.private_read_receipt.as_ref()), (None, Some(&id)));
        assert_eq!(off.fully_read.as_ref(), Some(&id));
    }
}
