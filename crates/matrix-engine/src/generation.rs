use matrix_domain::RoomScope;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// View work only; not send identity, transaction ID or retry attempt ID.
/// Clone to workers and check cancellation at bounded work/await boundaries.
#[derive(Clone)]
pub struct RequestTag {
    scope: RoomScope,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}
impl RequestTag {
    pub fn scope(&self) -> &RoomScope {
        &self.scope
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    /// Same attempt: the shared cancellation flag. A duplicate command coalesces instead of restarting.
    pub fn is_same(&self, other: &RequestTag) -> bool {
        self.scope == other.scope
            && self.generation == other.generation
            && Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct GenerationExhausted;

/// UI-owned view lifecycle. Cancellation is cooperative, not task abortion.
pub struct ViewRequests {
    next: Option<u64>,
    current: Option<RequestTag>,
}
impl Default for ViewRequests {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for ViewRequests {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl ViewRequests {
    pub fn new() -> Self {
        Self {
            next: Some(0),
            current: None,
        }
    }
    pub fn start(&mut self, scope: RoomScope) -> Result<RequestTag, GenerationExhausted> {
        let generation = self.next.ok_or(GenerationExhausted)?;
        self.cancel();
        let tag = RequestTag {
            scope,
            generation,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        self.next = generation.checked_add(1);
        self.current = Some(tag.clone());
        Ok(tag)
    }
    /// Cancel view work without affecting durable sends for its room.
    pub fn cancel(&mut self) {
        if let Some(old) = self.current.take() {
            old.cancelled.store(true, Ordering::Release);
        }
    }
    /// Check at consumption too: a cancelled worker may already have queued data.
    pub fn accepts(&self, tag: &RequestTag) -> bool {
        self.current.as_ref().is_some_and(|active| {
            active.scope == tag.scope
                && active.generation == tag.generation
                && Arc::ptr_eq(&active.cancelled, &tag.cancelled)
                && !tag.is_cancelled()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    #[test]
    fn exhausted_generation_cannot_wrap_or_cancel_current_request() {
        let mut views = ViewRequests::new();
        views.next = Some(u64::MAX);
        let scope = RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!offline:fixture"),
        };
        let last = views.start(scope.clone()).unwrap();
        assert_eq!(last.generation(), u64::MAX);
        assert_eq!(views.start(scope).err(), Some(GenerationExhausted));
        assert!(!last.is_cancelled());
        assert!(views.accepts(&last));
    }
}
