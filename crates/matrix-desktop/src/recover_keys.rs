//! «Recupera chiavi» on an undecryptable bubble: which bubble has the key field open, and the key
//! typed there until the shell queues it. UI-only; the recovery itself is `DeviceVerification`.
use matrix_domain::ScopedTimelineId;
use zeroize::Zeroizing;

pub const RECOVER_KEYS_LABEL: &str = "Recupera chiavi";
// Same wording as the sign-in step; without `live` nothing is ever offered, so the text is never shown.
#[cfg(feature = "live")]
pub use crate::signin_form::{RECOVERY_KEY_LABEL, RECOVERY_PENDING};
#[cfg(not(feature = "live"))]
pub const RECOVERY_KEY_LABEL: &str = "";
#[cfg(not(feature = "live"))]
pub const RECOVERY_PENDING: &str = "";

#[derive(Default)]
pub struct RecoverKeys {
    offered: bool,
    pending: bool,
    message: Option<&'static str>,
    open: Option<ScopedTimelineId>,
    submitted: Option<Zeroizing<String>>,
}

impl RecoverKeys {
    /// Called every frame from the shell's verification model; a withdrawn offer closes the field.
    pub fn set_state(&mut self, offered: bool, pending: bool, message: Option<&'static str>) {
        self.offered = offered;
        self.pending = pending;
        self.message = message;
        if !offered {
            self.open = None;
            self.submitted = None;
        }
    }
    pub fn offered(&self) -> bool {
        self.offered
    }
    /// Opens the key field under `id` (one field at a time).
    pub fn open(&mut self, id: &ScopedTimelineId) -> bool {
        if !self.offered {
            return false;
        }
        self.open = Some(id.clone());
        true
    }
    pub fn close(&mut self) {
        self.open = None;
    }
    pub fn is_open(&self, id: &ScopedTimelineId) -> bool {
        self.offered && self.open.as_ref() == Some(id)
    }
    /// The one status line under the field: Pending or the last failure.
    pub fn status(&self) -> Option<&'static str> {
        if self.pending {
            Some(RECOVERY_PENDING)
        } else {
            self.message
        }
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    /// A key may be handed over: field open and no attempt running.
    pub fn accepts_key(&self) -> bool {
        self.offered && !self.pending && self.open.is_some()
    }
    /// Hands over the typed key; refused while an attempt is running.
    pub fn submit(&mut self, key: Zeroizing<String>) -> bool {
        if !self.accepts_key() {
            return false;
        }
        self.submitted = Some(key);
        true
    }
    /// The shell takes the key once; nothing keeps it afterwards.
    pub fn take_submitted(&mut self) -> Option<Zeroizing<String>> {
        self.submitted.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId, RoomScope, TimelineId};

    fn id(n: &str) -> ScopedTimelineId {
        ScopedTimelineId {
            scope: RoomScope {
                account_id: AccountId::new(1),
                room_id: RoomId::new("!r:fixture.invalid"),
            },
            id: TimelineId::Server(matrix_domain::EventId::new(format!("${n}:fixture.invalid"))),
        }
    }

    #[test]
    fn field_opens_only_while_offered_and_closes_when_withdrawn() {
        let mut r = RecoverKeys::default();
        assert!(!r.open(&id("a")), "nothing offered yet");
        r.set_state(true, false, None);
        assert!(r.open(&id("a")) && r.is_open(&id("a")) && !r.is_open(&id("b")));
        assert!(
            r.open(&id("b")) && !r.is_open(&id("a")),
            "one field at a time"
        );
        r.set_state(false, false, None);
        assert!(
            !r.is_open(&id("b")),
            "recovery complete: gone from every bubble"
        );
    }

    #[test]
    fn the_key_is_handed_over_once_and_not_while_pending() {
        let mut r = RecoverKeys::default();
        r.set_state(true, false, None);
        assert!(!r.submit(Zeroizing::new("k".into())), "no field open");
        r.open(&id("a"));
        r.set_state(true, true, None);
        assert!(
            !r.submit(Zeroizing::new("k".into())),
            "an attempt is running"
        );
        assert_eq!(r.status(), Some(RECOVERY_PENDING));
        r.set_state(true, false, Some("errore"));
        assert_eq!(r.status(), Some("errore"));
        assert!(r.submit(Zeroizing::new("EsTu key".into())));
        assert_eq!(
            r.take_submitted().as_deref().map(String::as_str),
            Some("EsTu key")
        );
        assert!(r.take_submitted().is_none(), "taken once");
        assert!(r.submit(Zeroizing::new("again".into())));
        r.set_state(false, false, None);
        assert!(
            r.take_submitted().is_none(),
            "a withdrawn offer drops an unsent key"
        );
    }
}
