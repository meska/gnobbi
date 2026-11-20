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

/// Openers tried in order until one exits 0. egui's own `ctx.open_url` does nothing here: eframe is built without the `links`
/// feature (no webbrowser crate), so it only logs a warning. shortcut: no Windows opener beyond `explorer`; iOS has its own `open_url`.
const OPENERS: &[&[&str]] = if cfg!(target_os = "macos") {
    &[&["open"]]
} else if cfg!(windows) {
    &[&["explorer"]]
} else {
    &[&["xdg-open"], &["gio", "open"], &["sensible-browser"]]
};

/// Runs the openers through `run(argv)` (true = opened); stops at the first success.
pub fn open_with(url: &SafeWebUrl, run: &mut dyn FnMut(&[&str]) -> bool) -> bool {
    OPENERS.iter().any(|argv| {
        let mut full = argv.to_vec();
        full.push(url.as_str());
        run(&full)
    })
}

/// iOS has no opener command: UIApplication opens it. Must run on the main thread, which is the UI thread (`MainThreadMarker`);
/// the answer comes back through the completion handler.
#[cfg(target_os = "ios")]
pub fn open_url(url: &SafeWebUrl) -> std::sync::mpsc::Receiver<bool> {
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSDictionary, NSString, NSURL};
    use objc2_ui_kit::UIApplication;
    let (tx, rx) = std::sync::mpsc::channel();
    let tx2 = tx.clone();
    let opened = MainThreadMarker::new().and_then(|mtm| {
        let nsurl = NSURL::URLWithString(&NSString::from_str(url.as_str()))?;
        let done = block2::RcBlock::new(move |ok: objc2::runtime::Bool| { let _ = tx2.send(ok.as_bool()); });
        // SAFETY: main thread (marker); the block is copied by UIKit; empty options.
        unsafe { UIApplication::sharedApplication(mtm).openURL_options_completionHandler(&nsurl, &NSDictionary::new(), Some(&done)) };
        Some(())
    });
    if opened.is_none() { let _ = tx.send(false); }
    rx
}

/// Opens in the system browser off the UI thread; the receiver yields the outcome once.
#[cfg(not(target_os = "ios"))]
pub fn open_url(url: &SafeWebUrl) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    let url = url.clone();
    std::thread::spawn(move || {
        let ok = open_with(&url, &mut |a| {
            std::process::Command::new(a[0]).args(&a[1..]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
        });
        let _ = tx.send(ok);
    });
    rx
}
