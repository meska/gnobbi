//! Captured controls for the existing local demo, never Matrix delivery.
use crate::{DemoEvent, DemoHistory, DemoSendState};
use matrix_domain::{ScopedTimelineId, TimelineId};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct OfflineDeliveryTarget {
    instance: u64,
    event: ScopedTimelineId,
    text: String,
    state: DemoSendState,
}
impl OfflineDeliveryTarget {
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn event(&self) -> &ScopedTimelineId {
        &self.event
    }
    pub fn state(&self) -> DemoSendState {
        self.state
    }
}
#[derive(Default)]
pub struct OfflineDelivery {
    sources: HashMap<ScopedTimelineId, OfflineDeliveryTarget>,
}
impl OfflineDelivery {
    pub fn clear(&mut self) {
        self.sources.clear();
    }
    pub fn retain_current(&mut self, history: &DemoHistory) {
        self.sources.retain(|id, s| {
            history
                .events()
                .iter()
                .any(|e| e.id == *id && e.text == s.text && e.send_state == Some(s.state))
        });
    }
    pub fn target(&mut self, row: &DemoEvent) -> Option<OfflineDeliveryTarget> {
        if !matches!(row.id.id, TimelineId::Pending(_)) {
            return None;
        }
        let state = row.send_state?;
        if self
            .sources
            .get(&row.id)
            .is_none_or(|s| s.text != row.text || s.state != state)
        {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let instance = NEXT
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("offline controls exhausted");
            self.sources.insert(
                row.id.clone(),
                OfflineDeliveryTarget {
                    instance,
                    event: row.id.clone(),
                    text: row.text.clone(),
                    state,
                },
            );
        }
        self.sources.get(&row.id).cloned()
    }
    pub fn is_current(&self, target: &OfflineDeliveryTarget, history: &DemoHistory) -> bool {
        self.sources
            .get(&target.event)
            .is_some_and(|s| s.instance == target.instance)
            && history.events().iter().any(|e| {
                e.id == target.event && e.text == target.text && e.send_state == Some(target.state)
            })
    }
    pub fn apply(
        &mut self,
        target: &OfflineDeliveryTarget,
        history: &mut DemoHistory,
    ) -> Option<usize> {
        if !self.is_current(target, history) {
            return None;
        }
        let index = history.events().iter().position(|e| e.id == target.event)?;
        let changed = match target.state {
            DemoSendState::Pending => history.fail_local(&target.event),
            DemoSendState::Failed => history.retry_local(&target.event),
        };
        if !changed {
            return None;
        }
        self.sources.remove(&target.event);
        Some(index)
    }
}
