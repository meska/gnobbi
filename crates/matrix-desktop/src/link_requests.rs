//! Source-bound link occurrences, never an external-opening capability.
use crate::{LinkConfirmation, MessageLink, SafeWebUrl, message_links};
use matrix_domain::{RoomScope, ScopedTimelineId};
use std::{
    collections::HashMap,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

/// Where a link's text lives: a timeline message or the open room's topic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LinkSource {
    Timeline(ScopedTimelineId),
    RoomTopic(RoomScope),
}

#[derive(Clone)]
pub struct MessageLinkTarget {
    instance: u64,
    source: LinkSource,
    text: String,
    range: Range<usize>,
    url: SafeWebUrl,
}

impl MessageLinkTarget {
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn source(&self) -> &LinkSource {
        &self.source
    }
    /// The timeline event; None for a room topic.
    pub fn event(&self) -> Option<&ScopedTimelineId> {
        match &self.source {
            LinkSource::Timeline(event) => Some(event),
            LinkSource::RoomTopic(_) => None,
        }
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
    sources: HashMap<LinkSource, Vec<MessageLinkTarget>>,
}

impl MessageLinkRequests {
    pub fn targets(&mut self, event: &ScopedTimelineId, text: &str) -> Vec<MessageLinkTarget> {
        self.source_targets(LinkSource::Timeline(event.clone()), text)
    }

    /// The topic is the only source of a topic registry: any other source is dropped.
    pub fn topic_targets(&mut self, scope: &RoomScope, text: &str) -> Vec<MessageLinkTarget> {
        self.topic_rich_targets(scope, &matrix_engine::rich_text::Topic { text: text.to_owned(), ..Default::default() })
    }

    /// Like `topic_targets` for a rendered topic: the labelled links first (their real address, only
    /// if it is a safe web URL), then the bare URLs of the text that do not overlap a label.
    pub fn topic_rich_targets(
        &mut self,
        scope: &RoomScope,
        topic: &matrix_engine::rich_text::Topic,
    ) -> Vec<MessageLinkTarget> {
        let source = LinkSource::RoomTopic(scope.clone());
        self.sources.retain(|id, _| *id == source);
        let mut links: Vec<MessageLink> = topic
            .links
            .iter()
            .filter_map(|l| Some(MessageLink { range: l.range.clone(), url: SafeWebUrl::parse(&l.href)? }))
            .collect();
        let labels: Vec<_> = links.iter().map(|l| l.range.clone()).collect();
        links.extend(
            message_links(&topic.text)
                .into_iter()
                .filter(|b| !labels.iter().any(|l| l.start < b.range.end && b.range.start < l.end)),
        );
        links.sort_by_key(|l| l.range.start);
        // Same text with another address behind a label is a new occurrence.
        let same = self.sources.get(&source).is_some_and(|old| {
            old.len() == links.len()
                && old.iter().zip(&links).all(|(t, l)| t.text == topic.text && t.range == l.range && t.url == l.url)
        });
        if !same {
            let targets = self.instances(&source, &topic.text, links);
            self.sources.insert(source.clone(), targets);
        }
        self.sources[&source].clone()
    }

    fn instances(&self, source: &LinkSource, text: &str, links: Vec<MessageLink>) -> Vec<MessageLinkTarget> {
        links
            .into_iter()
            .map(|link| {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                let instance = NEXT
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                    .expect("link instance IDs exhausted");
                MessageLinkTarget {
                    instance,
                    source: source.clone(),
                    text: text.to_owned(),
                    range: link.range,
                    url: link.url,
                }
            })
            .collect()
    }

    fn source_targets(&mut self, source: LinkSource, text: &str) -> Vec<MessageLinkTarget> {
        if self
            .sources
            .get(&source)
            .is_none_or(|links| links.first().is_none_or(|l| l.text != text))
        {
            let targets = self.instances(&source, text, message_links(text));
            self.sources.insert(source.clone(), targets);
        }
        self.sources[&source].clone()
    }

    pub fn retain_event(&mut self, event: &ScopedTimelineId) {
        self.sources
            .retain(|id, _| matches!(id, LinkSource::Timeline(e) if e == event));
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
            let LinkSource::Timeline(id) = id else {
                return false;
            };
            current
                .get(id)
                .is_some_and(|text| links.first().is_some_and(|l| l.text == *text))
        });
    }

    pub fn is_current(&self, target: &MessageLinkTarget) -> bool {
        self.sources
            .get(&target.source)
            .is_some_and(|links| links.iter().any(|l| l.instance == target.instance))
    }

    pub fn focus(&mut self, target: &MessageLinkTarget) -> bool {
        if !self.is_current(target) {
            return false;
        }
        self.sources.retain(|id, _| *id == target.source);
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
