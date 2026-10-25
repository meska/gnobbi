//! Source-bound link occurrences, never an external-opening capability.
use crate::{LinkConfirmation, SafeWebUrl, message_links};
use matrix_domain::ScopedTimelineId;
use std::{
    collections::HashMap,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct MessageLinkTarget {
    instance: u64,
    event: ScopedTimelineId,
    text: String,
    range: Range<usize>,
    url: SafeWebUrl,
}

impl MessageLinkTarget {
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn event(&self) -> &ScopedTimelineId {
        &self.event
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
    pub fn url(&self) -> &SafeWebUrl {
        &self.url
    }
}

#[derive(Default)]
pub struct MessageLinkRequests {
    sources: HashMap<ScopedTimelineId, Vec<MessageLinkTarget>>,
}

impl MessageLinkRequests {
    pub fn targets(&mut self, event: &ScopedTimelineId, text: &str) -> Vec<MessageLinkTarget> {
        if self
            .sources
            .get(event)
            .is_none_or(|links| links.first().is_none_or(|l| l.text != text))
        {
            let links = message_links(text)
                .into_iter()
                .map(|link| {
                    static NEXT: AtomicU64 = AtomicU64::new(1);
                    let instance = NEXT
                        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                        .expect("link instance IDs exhausted");
                    MessageLinkTarget {
                        instance,
                        event: event.clone(),
                        text: text.to_owned(),
                        range: link.range,
                        url: link.url,
                    }
                })
                .collect();
            self.sources.insert(event.clone(), links);
        }
        self.sources[event].clone()
    }

    pub fn retain_event(&mut self, event: &ScopedTimelineId) {
        self.sources.retain(|id, _| id == event);
    }

    pub fn clear(&mut self) {
        self.sources.clear();
    }

    pub fn retain_current<'a>(
        &mut self,
        events: impl IntoIterator<Item = (&'a ScopedTimelineId, &'a str)>,
    ) {
        let current: HashMap<_, _> = events.into_iter().collect();
        self.sources.retain(|id, links| {
            current
                .get(id)
                .is_some_and(|text| links.first().is_some_and(|l| l.text == *text))
        });
    }

    pub fn is_current(&self, target: &MessageLinkTarget) -> bool {
        self.sources
            .get(&target.event)
            .is_some_and(|links| links.iter().any(|l| l.instance == target.instance))
    }

    pub fn focus(&mut self, target: &MessageLinkTarget) -> bool {
        if !self.is_current(target) {
            return false;
        }
        self.retain_event(target.event());
        true
    }

    pub fn request(
        &mut self,
        target: &MessageLinkTarget,
        confirmation: &mut LinkConfirmation,
    ) -> bool {
        if !self.focus(target) {
            return false;
        }
        confirmation.request(target)
    }
}

/// The link occurrence covering a byte index of the message text.
pub fn link_at(targets: &[MessageLinkTarget], index: usize) -> Option<&MessageLinkTarget> {
    targets.iter().find(|t| t.range().contains(&index))
}

/// Right-click menu on a link: copy the address, or open it (still behind the confirmation).
#[derive(Default)]
pub struct LinkMenu {
    target: Option<MessageLinkTarget>,
}

impl LinkMenu {
    pub const ITEMS: [&'static str; 2] = ["Copia link", "Apri"];

    pub fn open(&mut self, target: MessageLinkTarget) {
        self.target = Some(target);
    }

    pub fn target(&self) -> Option<&MessageLinkTarget> {
        self.target.as_ref()
    }

    pub fn close(&mut self) {
        self.target = None;
    }

    pub fn copy(&mut self) -> Option<String> {
        self.target.take().map(|t| t.url().as_str().to_owned())
    }

    pub fn open_link(
        &mut self,
        sources: &mut MessageLinkRequests,
        confirmation: &mut LinkConfirmation,
    ) -> bool {
        self.target
            .take()
            .is_some_and(|t| sources.request(&t, confirmation))
    }
}
