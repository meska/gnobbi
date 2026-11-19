//! Exact public receipt markers, not inferred reading of earlier events.
use matrix_domain::EventId;

/// shortcut: rooms above this get no reader circles, when a cheaper per-room receipt query exists lift it.
pub const MAX_RECEIPT_ROOM: u64 = 30;

#[derive(Clone, PartialEq, Eq)]
pub struct PublicReadReceipt {
    pub event_id: EventId,
    pub readers: u64,
    /// Who the markers on this event belong to, oldest read first (own user excluded).
    pub users: Vec<crate::history::HistorySender>,
}

/// Other people's public read markers from the SDK Timeline items (the SDK folds threads into
/// the main view and keeps only each user's newest marker); `own_user` is not a peer.
#[cfg(feature = "sdk")]
pub(crate) async fn peer_receipts(
    timeline: &matrix_sdk_ui::timeline::Timeline,
    own_user: &str,
) -> Vec<PublicReadReceipt> {
    let mut out = Vec::new();
    for item in timeline.items().await.iter() {
        let Some(event) = item.as_event() else {
            continue;
        };
        let Some(id) = event.event_id() else {
            continue;
        };
        // The peer's marker may sit on their own newer event (reply, or reading up to it): it still covers earlier rows.
        let mut readers: Vec<_> = event.read_receipts().iter().filter(|(u, _)| u.as_str() != own_user).collect();
        if readers.is_empty() {
            continue;
        }
        readers.sort_by_key(|(_, r)| r.ts);
        let mut users = Vec::new();
        for (user, _) in &readers {
            users.push(crate::room_sync::history_sender(timeline.room(), user).await);
        }
        out.push(PublicReadReceipt { event_id: EventId::new(id.to_string()), readers: users.len() as u64, users });
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

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
            ReadVisibility {
                window_focused: false,
                ..seen
            },
            ReadVisibility {
                room_foreground: false,
                ..seen
            },
            ReadVisibility {
                bottom_gap_px: 5000.0,
                ..seen
            },
        ] {
            assert!(!hidden.sees_last_message());
        }
    }
}
