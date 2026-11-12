//! The room list's disconnection icon: shown only after the sync has been down for `GRACE`.
//! The "is down" input is the existing `RoomSyncStatus`; the engine's own StatusGate already debounces
//! episodes, this only keeps the icon from flashing on a short blip.
use std::time::{Duration, Instant};

/// Down for less than this: no icon (= the engine's StatusGate hold, 5 s). Back up: hidden at once.
/// The delay the user sees is dominated by the engine (SyncFailed arrives 27–147 s after the drop, measured); GRACE only avoids flashing.
pub const GRACE: Duration = Duration::from_secs(5);

/// What the engine distinguishes while down: `SyncFailed` covers no network and server down alike (one state, no message).
pub fn reason(status: &matrix_engine::RoomSyncStatus) -> Option<&'static str> {
    use matrix_engine::RoomSyncStatus::*;
    match status {
        AuthenticationFailed => Some("Sessione scaduta o rifiutata"),
        SyncFailed => Some("Sync interrotta"),
        Connecting | Ready => None,
    }
}

#[derive(Default)]
pub struct Gate(Option<Instant>);

impl Gate {
    /// Feed the current state; true when a new outage just began (caller schedules a redraw at `GRACE`).
    pub fn observe(&mut self, down: bool, now: Instant) -> bool {
        match (down, self.0) {
            (true, None) => {
                self.0 = Some(now);
                true
            }
            (false, Some(_)) => {
                self.0 = None;
                false
            }
            _ => false,
        }
    }

    pub fn visible(&self, now: Instant) -> bool {
        self.0.is_some_and(|since| now.duration_since(since) >= GRACE)
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub fn node(reason: &str) -> (accesskit::NodeId, accesskit::Node) {
    // Live::Polite: announced on change (accesskit_unix does not forward it yet; the node itself is read on navigation).
    let mut node = accesskit::Node::new(accesskit::Role::Status);
    node.set_label(reason);
    node.set_live(accesskit::Live::Polite);
    (accesskit::NodeId(14), node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_only_for_the_failed_states() {
        use matrix_engine::RoomSyncStatus::*;
        assert_eq!(reason(&SyncFailed), Some("Sync interrotta"));
        assert_eq!(reason(&AuthenticationFailed), Some("Sessione scaduta o rifiutata"));
        assert_eq!((reason(&Ready), reason(&Connecting)), (None, None));
    }

    #[test]
    fn connected_has_no_icon() {
        let (mut g, t) = (Gate::default(), Instant::now());
        assert!(!g.observe(false, t));
        assert!(!g.visible(t + GRACE * 10));
    }

    #[test]
    fn short_outage_never_shows() {
        let (mut g, t) = (Gate::default(), Instant::now());
        assert!(g.observe(true, t));
        assert!(!g.visible(t + GRACE - Duration::from_millis(1)));
        g.observe(false, t + Duration::from_secs(2));
        assert!(!g.visible(t + GRACE));
    }

    #[test]
    fn long_outage_shows_and_reconnect_hides_at_once() {
        let (mut g, t) = (Gate::default(), Instant::now());
        g.observe(true, t);
        assert!(!g.observe(true, t + GRACE));
        assert!(g.visible(t + GRACE));
        g.observe(false, t + GRACE);
        assert!(!g.visible(t + GRACE));
    }
}
