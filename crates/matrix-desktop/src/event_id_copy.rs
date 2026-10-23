//! The painted server stamp is a source capability, not a lookup of today's row.
use matrix_domain::{ScopedTimelineId, TimelineId};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct EventIdCopyTarget {
    instance: u64,
    event: ScopedTimelineId,
    text: String,
}
impl EventIdCopyTarget {
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn event(&self) -> &ScopedTimelineId {
        &self.event
    }
}

#[derive(Default)]
pub struct EventIdCopies {
    sources: HashMap<ScopedTimelineId, EventIdCopyTarget>,
}
impl EventIdCopies {
    pub fn clear(&mut self) {
        self.sources.clear();
    }
    pub fn retain_current<'a>(
        &mut self,
        events: impl IntoIterator<Item = (&'a ScopedTimelineId, &'a str)>,
    ) {
        let current: HashMap<_, _> = events.into_iter().collect();
        self.sources
            .retain(|id, source| current.get(id).is_some_and(|text| *text == source.text));
    }
    pub fn target(&mut self, event: &ScopedTimelineId, text: &str) -> Option<EventIdCopyTarget> {
        if !matches!(event.id, TimelineId::Server(_)) {
            return None;
        }
        if self.sources.get(event).is_none_or(|s| s.text != text) {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let instance = NEXT
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("event copy instances exhausted");
            self.sources.insert(
                event.clone(),
                EventIdCopyTarget {
                    instance,
                    event: event.clone(),
                    text: text.to_owned(),
                },
            );
        }
        self.sources.get(event).cloned()
    }
    pub fn resolve<'a>(&self, target: &'a EventIdCopyTarget) -> Option<&'a str> {
        if self
            .sources
            .get(&target.event)
            .is_none_or(|s| s.instance != target.instance)
        {
            return None;
        }
        match &target.event.id {
            TimelineId::Server(id) => Some(id.as_str()),
            _ => None,
        }
    }
}
