//! Join progress per target, kept by the application (not by the popup that started it), so a join
//! goes on, and its late result is still seen, after the popup is closed. Pure logic, no GPUI.
//! Spec: the JOIN_STATE_MACHINE notes. Timeouts live in `matrix_engine::join`
//! (`JOIN_CONFIRM_WINDOW` 150 s, `JOIN_RECHECK` 10 s); the HTTP timeout is the client's 10 s.
use matrix_domain::RoomScope;
use matrix_engine::{join::JoinPhase, operations::OperationError};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinState {
    Pending,
    /// Request timed out: still in progress.
    Slow,
    Joined(RoomScope),
    Refused(OperationError),
    /// «Non confermato»: «Riprova» is a new `begin`.
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Begin {
    /// Already joined: just open it.
    Open(RoomScope),
    /// Send the `JoinRoom` command.
    Send,
    /// Double click, or a join already in flight: do nothing, no error.
    Ignore,
}

/// «Sei entrato in <name>»: a join that finished while nobody was waiting for it. Never moves focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub name: String,
    pub scope: RoomScope,
}

#[derive(Default)]
pub struct Joins {
    map: HashMap<String, (JoinState, String)>,
    notice: Option<Notice>,
}

impl Joins {
    /// `joined`: the scope when the sidebar already holds the room as joined.
    pub fn begin(&mut self, target: &str, name: &str, joined: Option<RoomScope>) -> Begin {
        if let Some(scope) = joined {
            self.map.insert(target.into(), (JoinState::Joined(scope.clone()), name.into()));
            return Begin::Open(scope);
        }
        if matches!(self.state(target), Some(JoinState::Pending | JoinState::Slow)) {
            return Begin::Ignore;
        }
        self.map.insert(target.into(), (JoinState::Pending, name.into()));
        Begin::Send
    }

    /// Folds an engine phase in. `waiting`: the user still waits on this join (popup open, last click on it).
    /// Returns the scope to open when the join succeeded and someone waits; a late success with nobody
    /// waiting only sets the `notice`. Phases for unknown targets (account switched) and stale ones are dropped.
    pub fn apply(&mut self, target: &str, phase: JoinPhase, waiting: bool) -> Option<RoomScope> {
        let (state, name) = self.map.get_mut(target)?;
        let open = matches!(state, JoinState::Pending | JoinState::Slow);
        match phase {
            JoinPhase::Slow if *state == JoinState::Pending => *state = JoinState::Slow,
            JoinPhase::Refused(e) if open => *state = JoinState::Refused(e),
            JoinPhase::Unconfirmed if open => *state = JoinState::Unconfirmed,
            // Also from Unconfirmed/Refused: the room can still arrive by sync.
            JoinPhase::Joined(scope) if !matches!(state, JoinState::Joined(_)) => {
                *state = JoinState::Joined(scope.clone());
                if waiting {
                    return Some(scope);
                }
                self.notice = Some(Notice { name: name.clone(), scope });
            }
            _ => {}
        }
        None
    }

    pub fn state(&self, target: &str) -> Option<&JoinState> {
        self.map.get(target).map(|(s, _)| s)
    }
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }
    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }
    /// Account switch or disconnect: the engine joins of the old account are stopped.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    const T: &str = "!big:matrix.org";
    fn scope() -> RoomScope {
        RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(T) }
    }
    fn started() -> Joins {
        let mut j = Joins::default();
        assert_eq!(j.begin(T, "Big", None), Begin::Send);
        j
    }

    #[test]
    fn begin_sends_once_and_goes_pending() {
        let j = started();
        assert_eq!(j.state(T), Some(&JoinState::Pending));
    }

    #[test]
    fn joined_directly_opens_the_room() {
        let mut j = started();
        assert_eq!(j.apply(T, JoinPhase::Joined(scope()), true), Some(scope()));
        assert_eq!(j.state(T), Some(&JoinState::Joined(scope())));
        assert!(j.notice().is_none());
    }

    #[test]
    fn double_click_is_a_noop_while_pending_or_slow() {
        let mut j = started();
        assert_eq!(j.begin(T, "Big", None), Begin::Ignore);
        j.apply(T, JoinPhase::Slow, true);
        assert_eq!(j.state(T), Some(&JoinState::Slow));
        assert_eq!(j.begin(T, "Big", None), Begin::Ignore);
        assert_eq!(j.state(T), Some(&JoinState::Slow));
    }

    #[test]
    fn already_joined_in_the_sidebar_opens_without_a_command() {
        let mut j = Joins::default();
        assert_eq!(j.begin(T, "Big", Some(scope())), Begin::Open(scope()));
        assert_eq!(j.begin(T, "Big", Some(scope())), Begin::Open(scope()));
    }

    #[test]
    fn refused_keeps_the_reason_and_retry_sends_again() {
        let mut j = started();
        assert_eq!(j.apply(T, JoinPhase::Refused(OperationError::Forbidden), true), None);
        assert_eq!(j.state(T), Some(&JoinState::Refused(OperationError::Forbidden)));
        assert_eq!(j.begin(T, "Big", None), Begin::Send);
        assert_eq!(j.state(T), Some(&JoinState::Pending));
    }

    #[test]
    fn timeout_then_joined_opens_when_the_user_waits() {
        let mut j = started();
        j.apply(T, JoinPhase::Slow, true);
        assert_eq!(j.apply(T, JoinPhase::Joined(scope()), true), Some(scope()));
        assert_eq!(j.state(T), Some(&JoinState::Joined(scope())));
    }

    #[test]
    fn popup_closed_during_pending_late_join_only_notifies() {
        let mut j = started();
        j.apply(T, JoinPhase::Slow, false);
        assert_eq!(j.apply(T, JoinPhase::Joined(scope()), false), None, "no navigation, no focus change");
        assert_eq!(j.notice(), Some(&Notice { name: "Big".into(), scope: scope() }));
        j.dismiss_notice();
        assert!(j.notice().is_none());
    }

    #[test]
    fn timeout_unconfirmed_then_retry_or_late_join() {
        let mut j = started();
        j.apply(T, JoinPhase::Slow, true);
        assert_eq!(j.apply(T, JoinPhase::Unconfirmed, true), None);
        assert_eq!(j.state(T), Some(&JoinState::Unconfirmed));
        // The sync can still bring the room: Joined wins over Unconfirmed.
        assert_eq!(j.apply(T, JoinPhase::Joined(scope()), false), None);
        assert!(j.notice().is_some());
        // And a plain retry from Unconfirmed sends again.
        let mut j = started();
        j.apply(T, JoinPhase::Slow, true);
        j.apply(T, JoinPhase::Unconfirmed, true);
        assert_eq!(j.begin(T, "Big", None), Begin::Send);
    }

    #[test]
    fn stale_or_unknown_phases_change_nothing() {
        let mut j = started();
        j.apply(T, JoinPhase::Joined(scope()), true);
        j.apply(T, JoinPhase::Slow, true); // late Slow after Joined
        j.apply(T, JoinPhase::Joined(scope()), false); // duplicate Joined: no second notice
        assert_eq!(j.state(T), Some(&JoinState::Joined(scope())));
        assert!(j.notice().is_none());
        j.clear();
        assert_eq!(j.apply(T, JoinPhase::Joined(scope()), true), None);
        assert_eq!(j.state(T), None);
    }
}
