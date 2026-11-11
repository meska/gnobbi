use crate::{MessageLinkRequests, MessageLinkTarget};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct LinkConfirmationRequest {
    generation: u64,
    source: MessageLinkTarget,
}

impl LinkConfirmationRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn source(&self) -> &MessageLinkTarget {
        &self.source
    }
    pub fn url(&self) -> &SafeWebUrl {
        self.source.url()
    }
}

#[derive(Default)]
pub struct LinkConfirmation {
    pending: Option<LinkConfirmationRequest>,
}

impl LinkConfirmation {
    pub(crate) fn request(&mut self, source: &MessageLinkTarget) -> bool {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("link confirmation IDs exhausted");
        self.pending = Some(LinkConfirmationRequest {
            generation,
            source: source.clone(),
        });
        true
    }

    pub fn capture(&self) -> Option<LinkConfirmationRequest> {
        self.pending.clone()
    }

    pub fn pending(&self) -> Option<&SafeWebUrl> {
        self.pending.as_ref().map(LinkConfirmationRequest::url)
    }

    pub fn confirm_current(
        &mut self,
        request: &LinkConfirmationRequest,
        sources: &MessageLinkRequests,
    ) -> Option<SafeWebUrl> {
        self.cancel_current(request, sources)
            .then(|| request.url().clone())
    }

    pub fn cancel_current(
        &mut self,
        request: &LinkConfirmationRequest,
        sources: &MessageLinkRequests,
    ) -> bool {
        if !sources.is_current(request.source())
            || self
                .pending
                .as_ref()
                .is_none_or(|p| p.generation != request.generation)
        {
            return false;
        }
        self.cancel();
        true
    }

    /// Dismiss the current UI state; captured callbacks must use cancel_current.
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

pub struct MessageLink {
    pub range: Range<usize>,
    pub url: SafeWebUrl,
}

/// Conservative plaintext detection; not HTML/Markdown or a rich-text renderer.
pub fn message_links(text: &str) -> Vec<MessageLink> {
    let mut offset = 0;
    let mut links = Vec::new();
    for token in text.split_inclusive(char::is_whitespace) {
        let mut candidate = token.trim();
        // Only remove an explicit enclosing pair. Bare URI punctuation (including
        // IPv6 brackets and balanced path parentheses) belongs to the URL.
        for (open, close) in [('(', ')'), ('[', ']'), ('<', '>'), ('"', '"')] {
            if let Some(rest) = candidate.strip_prefix(open) {
                let rest = rest.trim_end_matches(['.', ',', ';', '!']);
                if let Some(inner) = rest.strip_suffix(close) {
                    candidate = inner;
                }
                break;
            }
        }
        if let Some(url) = SafeWebUrl::parse(candidate) {
            let start = offset + token.find(candidate).expect("candidate is part of token");
            links.push(MessageLink {
                range: start..start + candidate.len(),
                url,
            });
        }
        offset += token.len();
    }
    links
}

/// Browser-only URL capability; no custom schemes, credentials or invisible characters.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeWebUrl(String);

impl SafeWebUrl {
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.len() > 4096 {
            return None;
        }
        let (scheme, rest) = raw.split_once("://")?;
        let authority = rest.split(['/', '?', '#']).next()?;
        if authority.is_empty() || authority.contains('@') {
            return None;
        }
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        if matrix_domain::has_unsafe_url_char(raw) {
            return None;
        }
        let parsed = url::Url::parse(raw).ok()?;
        if parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return None;
        }
        Some(Self(parsed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
