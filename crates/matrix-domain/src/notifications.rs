//! Local unread alerts with an optional message preview. No SDK, UI or OS dependencies.
//! Markdown/HTML are already reduced to plain text by the engine; here we only normalize.
use crate::{AccountId, RoomScope};
use unicode_segmentation::UnicodeSegmentation;

/// Longest body, in grapheme clusters (ellipsis included); within the 100-140 range.
pub const PREVIEW_MAX: usize = 120;
const FIXED_BODY: &str = "Nuovi messaggi";

/// What a message holds, as far as a notification may say. No `Debug`: text must not reach logs.
pub enum PreviewContent {
    Text(String),
    Image,
    Video,
    Voice,
    Audio,
    File,
    Sticker,
    Location,
    Poll,
    /// Not decrypted (missing keys): never shown.
    Undecrypted,
    Other,
}

/// One notifying message. `sender` is only used in rooms (`direct == false`).
pub struct MessagePreview {
    pub content: PreviewContent,
    pub sender: Option<String>,
    pub direct: bool,
}

fn clean(text: &str) -> String {
    let text: String = text
        .chars()
        .filter(|c| {
            c.is_whitespace()
                || !(c.is_control()
                    || matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        })
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The OS body. Without a usable preview (switch off, undecrypted, empty) it is a fixed string
/// or, with `count_preview`, a count.
pub fn notification_body(
    preview: Option<&MessagePreview>,
    show_preview: bool,
    count_preview: bool,
) -> String {
    let text = preview.filter(|_| show_preview).and_then(|p| {
        let text = match &p.content {
            PreviewContent::Text(t) => clean(t),
            PreviewContent::Image => "📷 Foto".into(),
            PreviewContent::Video => "🎥 Video".into(),
            PreviewContent::Voice => "🎤 Messaggio vocale".into(),
            PreviewContent::Audio => "🎵 Audio".into(),
            PreviewContent::File => "📎 File".into(),
            PreviewContent::Sticker => "Sticker".into(),
            PreviewContent::Location => "📍 Posizione".into(),
            PreviewContent::Poll => "Sondaggio".into(),
            PreviewContent::Undecrypted | PreviewContent::Other => return None,
        };
        if text.is_empty() {
            return None;
        }
        let sender = p.sender.as_deref().map(clean).filter(|s| !s.is_empty());
        Some(match sender.filter(|_| !p.direct) {
            Some(name) => format!("{name}: {text}"),
            None => text,
        })
    });
    match text {
        Some(text) if text.graphemes(true).count() > PREVIEW_MAX => {
            let head: String = text.graphemes(true).take(PREVIEW_MAX - 1).collect();
            format!("{}…", head.trim_end())
        }
        Some(text) => text,
        None if count_preview => "1 nuovo messaggio".into(),
        None => FIXED_BODY.into(),
    }
}

/// What the OS may show; which messages notify (and which rooms are muted) is decided by the
/// account's push rules, through the SDK.
pub struct NotificationPolicy {
    account: AccountId,
    enabled: bool,
    count_preview: bool,
    message_preview: bool,
}
impl NotificationPolicy {
    pub fn new(account: AccountId) -> Self {
        Self {
            account,
            enabled: false,
            count_preview: false,
            message_preview: true,
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn count_preview(&self) -> bool {
        self.count_preview
    }
    pub fn set_count_preview(&mut self, preview: bool) {
        self.count_preview = preview;
    }
    pub fn message_preview(&self) -> bool {
        self.message_preview
    }
    pub fn set_message_preview(&mut self, preview: bool) {
        self.message_preview = preview;
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    /// The alert for one message that the push rules say notifies in `scope`.
    pub fn alert(&self, scope: &RoomScope, preview: Option<MessagePreview>) -> Option<Notification> {
        (self.enabled && scope.account_id == self.account).then(|| Notification {
            body: notification_body(preview.as_ref(), self.message_preview, self.count_preview),
            scope: scope.clone(),
            origin: None,
        })
    }
}

/// The finished title/body pair for the OS; the body is built once and never logged.
pub struct Notification {
    body: String,
    /// The room the alert is about (the worker's own account ID).
    scope: RoomScope,
    /// Catalog ID and display name of the account it came from; set by the window.
    origin: Option<(AccountId, String)>,
}
impl Notification {
    /// Names the account the alert came from; its name becomes the title.
    pub fn from_account(mut self, id: AccountId, name: String) -> Self {
        self.origin = Some((id, name));
        self
    }
    /// Where a click goes: the catalog account (None when unnamed) and the room.
    pub fn target(&self) -> (Option<AccountId>, &RoomScope) {
        (self.origin.as_ref().map(|(id, _)| *id), &self.scope)
    }
    pub fn title(&self) -> &str {
        self.origin.as_ref().map_or("Gnobbi", |(_, name)| name)
    }
    pub fn body(&self) -> String {
        self.body.clone()
    }
}
