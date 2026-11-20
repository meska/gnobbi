//! State of one join of a (large, federated) room, as the UI reads it. Pure: no egui, no engine calls.
//! A client timeout is NOT a failure: `JoinPhase::Slow` keeps the join «in corso» until the server's
//! real outcome arrives (sync / membership recheck, both already done by the engine's `join_tracked`).
use matrix_desktop::i18n;
use matrix_domain::RoomScope;
use matrix_engine::{join::JoinPhase, operations::OperationError};

/// Why a join failed; each cause has its own one-line text. `Generic` is the catch-all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Offline,
    ServerUnreachable,
    RoomMissing,
    Generic,
}

impl Cause {
    pub fn line(self) -> &'static str {
        match self {
            Self::Offline => i18n::t("You are offline"),
            Self::ServerUnreachable => i18n::t("Server unreachable"),
            Self::RoomMissing => i18n::t("Room not found"),
            Self::Generic => i18n::t("Connection failed"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinState {
    Idle,
    /// Request sent, client timeout not reached.
    Joining,
    /// Client timeout reached, server still joining: not an error.
    StillJoining,
    Joined(RoomScope),
    Failed(Cause),
}

impl Default for JoinState {
    fn default() -> Self {
        Self::Idle
    }
}

impl JoinState {
    /// «Riprova» / first click: allowed unless a join is already running (double click is a no-op).
    pub fn start(&mut self) -> bool {
        if matches!(self, Self::Joining | Self::StillJoining | Self::Joined(_)) {
            return false;
        }
        *self = Self::Joining;
        true
    }

    /// Folds an engine phase in. `online`: the client has network right now (the engine cannot tell
    /// offline from unreachable, the caller can). Late phases of a finished join are dropped,
    /// except a success: the room can still arrive by sync after a failure.
    pub fn apply(&mut self, phase: JoinPhase, online: bool) {
        let running = matches!(self, Self::Joining | Self::StillJoining);
        match phase {
            JoinPhase::Slow if *self == Self::Joining => *self = Self::StillJoining,
            JoinPhase::Joined(scope) if !matches!(self, Self::Joined(_)) => *self = Self::Joined(scope),
            JoinPhase::Refused(e) if running => *self = Self::Failed(refusal_cause(&e, online)),
            // Window over without proof: no answer from the server (or none to the client at all).
            JoinPhase::Unconfirmed if running => *self = Self::Failed(unreachable_cause(online)),
            _ => {}
        }
    }

    /// The single row the UI shows; `None` for `Idle`.
    pub fn line(&self) -> Option<&'static str> {
        match self {
            Self::Idle => None,
            Self::Joining => Some(i18n::t("Joining…")),
            Self::StillJoining => Some(i18n::t("Still joining…")),
            Self::Joined(_) => Some(i18n::t("Already joined")),
            Self::Failed(c) => Some(c.line()),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Joining | Self::StillJoining)
    }

    pub fn error(&self) -> Option<Cause> {
        match self {
            Self::Failed(c) => Some(*c),
            _ => None,
        }
    }
}

fn unreachable_cause(online: bool) -> Cause {
    if online { Cause::ServerUnreachable } else { Cause::Offline }
}

fn refusal_cause(e: &OperationError, online: bool) -> Cause {
    match e {
        OperationError::Unavailable => Cause::RoomMissing,
        OperationError::Transport => unreachable_cause(online),
        _ => Cause::Generic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    fn scope() -> RoomScope {
        RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!big:matrix.org") }
    }
    fn started() -> JoinState {
        let mut s = JoinState::default();
        assert!(s.start());
        s
    }

    #[test]
    fn quick_join_goes_straight_to_joined() {
        let mut s = started();
        assert_eq!(s.line(), Some("Joining…"));
        s.apply(JoinPhase::Joined(scope()), true);
        assert_eq!(s, JoinState::Joined(scope()));
    }

    #[test]
    fn client_timeout_is_still_joining_not_an_error() {
        let mut s = started();
        s.apply(JoinPhase::Slow, true);
        assert_eq!(s, JoinState::StillJoining);
        assert_eq!(s.error(), None);
        assert!(s.is_running());
        assert_eq!(s.line(), Some("Still joining…"));
        assert!(!s.start(), "double click while running does nothing");
    }

    #[test]
    fn real_outcome_after_timeout_succeeds_or_fails() {
        let mut ok = started();
        ok.apply(JoinPhase::Slow, true);
        ok.apply(JoinPhase::Joined(scope()), true);
        assert_eq!(ok, JoinState::Joined(scope()));

        let mut ko = started();
        ko.apply(JoinPhase::Slow, true);
        ko.apply(JoinPhase::Refused(OperationError::Forbidden), true);
        assert_eq!(ko, JoinState::Failed(Cause::Generic));
    }

    #[test]
    fn causes_are_distinct_one_liners() {
        let cases = [
            (JoinPhase::Refused(OperationError::Unavailable), true, Cause::RoomMissing),
            (JoinPhase::Refused(OperationError::Transport), false, Cause::Offline),
            (JoinPhase::Refused(OperationError::Transport), true, Cause::ServerUnreachable),
            (JoinPhase::Unconfirmed, false, Cause::Offline),
            (JoinPhase::Unconfirmed, true, Cause::ServerUnreachable),
            (JoinPhase::Refused(OperationError::InvalidInput), true, Cause::Generic),
        ];
        let mut lines = Vec::new();
        for (phase, online, cause) in cases {
            let mut s = started();
            s.apply(phase, online);
            assert_eq!(s, JoinState::Failed(cause));
            lines.push(s.line().unwrap());
        }
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| !l.contains('\n')));
        assert_eq!(Cause::Generic.line(), "Connection failed");
    }

    #[test]
    fn retry_after_failure_and_late_success() {
        let mut s = started();
        s.apply(JoinPhase::Unconfirmed, true);
        assert!(s.start(), "Riprova");
        assert_eq!(s, JoinState::Joining);
        s.apply(JoinPhase::Unconfirmed, true);
        s.apply(JoinPhase::Slow, true); // stale: ignored
        assert_eq!(s.error(), Some(Cause::ServerUnreachable));
        s.apply(JoinPhase::Joined(scope()), true); // the sync delivered the room anyway
        assert_eq!(s, JoinState::Joined(scope()));
    }
}
