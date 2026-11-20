//! Local unread alerts with an optional message preview. No SDK, UI or OS dependencies.
//! Markdown/HTML are already reduced to plain text by the engine; here we only normalize.
use crate::{AccountId, RoomScope};
use unicode_segmentation::UnicodeSegmentation;

/// Longest body, in grapheme clusters (ellipsis included); within the 100-140 range.
pub const PREVIEW_MAX: usize = 120;
const FIXED_BODY: &str = "New messages";

static TRANSLATOR: std::sync::OnceLock<fn(&str) -> String> = std::sync::OnceLock::new();

/// The UI registers its translator once; the fixed texts below are English keys. Without one they stay English.
/// A body is built when the alert is made, so alerts already issued keep their language.
pub fn set_translator(f: fn(&str) -> String) {
    let _ = TRANSLATOR.set(f);
}

fn tr(key: &str) -> String {
    TRANSLATOR.get().map_or_else(|| key.to_owned(), |f| f(key))
}

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
    /// mxc of the sender's profile picture, when known.
    pub sender_avatar: Option<String>,
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
            PreviewContent::Image => tr("📷 Photo"),
            PreviewContent::Video => tr("🎥 Video"),
            PreviewContent::Voice => tr("🎤 Voice message"),
            PreviewContent::Audio => tr("🎵 Audio"),
            PreviewContent::File => tr("📎 File"),
            PreviewContent::Sticker => tr("Sticker"),
            PreviewContent::Location => tr("📍 Location"),
            PreviewContent::Poll => tr("Poll"),
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
        None if count_preview => tr("1 new message"),
        None => tr(FIXED_BODY),
    }
}

fn cut(text: &str, max: usize) -> String {
    let text = clean(text);
    if text.graphemes(true).count() <= max {
        return text;
    }
    let head: String = text.graphemes(true).take(max - 1).collect();
    format!("{}…", head.trim_end())
}

/// Title and optional subtitle of an alert. One account: the sender alone. Two or more: «Sender → Recipient»
/// (recipient = the receiving account's name, with its id when another connected account has the same name);
/// a group adds the room as subtitle (platforms without one join it with « · », see `Notification::title_line`).
pub fn notification_title(sender: Option<&str>, account_name: &str, account_id: &str, room_name: Option<&str>, other_accounts: &[&str], direct: bool) -> (String, Option<String>) {
    let sender = sender.map(|s| cut(s, 40)).filter(|s| !s.is_empty());
    let account = cut(account_name, 40);
    if other_accounts.is_empty() {
        return (sender.unwrap_or(if account.is_empty() { "Gnobbi".into() } else { account }), None);
    }
    let same = other_accounts.iter().any(|o| clean(o) == clean(account_name));
    let recipient = match (account.is_empty(), same) {
        (true, _) => cut(account_id, 40),
        (false, true) => format!("{account} ({})", cut(account_id, 40)),
        (false, false) => account,
    };
    let title = match sender {
        Some(s) => format!("{s} → {recipient}"),
        None => recipient,
    };
    let room = room_name.filter(|_| !direct).map(|r| cut(r, 60)).filter(|r| !r.is_empty());
    (title, room)
}

/// What the account's push rules say about one room (server side, shared by every device).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoomNotifyMode {
    #[default]
    AllMessages,
    MentionsAndKeywordsOnly,
    Mute,
}
impl RoomNotifyMode {
    pub const ALL: [Self; 3] = [Self::AllMessages, Self::MentionsAndKeywordsOnly, Self::Mute];
    /// On-screen label and accessibility name.
    pub fn label(self) -> &'static str {
        match self {
            Self::AllMessages => "All messages",
            Self::MentionsAndKeywordsOnly => "Mentions only",
            Self::Mute => "Muted",
        }
    }
}

/// What kind of event wants to notify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyKind {
    Normal,
    Mention,
    Keyword,
    Invite,
}

/// The one notification decision; the alert and the preview both ask it, and when it is false nothing is built.
pub fn should_notify(mode: RoomNotifyMode, low_priority: bool, kind: NotifyKind) -> bool {
    match mode {
        RoomNotifyMode::Mute => false,
        RoomNotifyMode::MentionsAndKeywordsOnly if kind == NotifyKind::Normal => false,
        // Low priority keeps only direct mentions and invites; keywords stay quiet (product choice).
        _ => !low_priority || matches!(kind, NotifyKind::Mention | NotifyKind::Invite),
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
            sender: preview.as_ref().and_then(|p| p.sender.clone()),
            sender_avatar: preview.as_ref().and_then(|p| p.sender_avatar.clone()),
            direct: preview.as_ref().is_some_and(|p| p.direct),
            title: None,
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
    sender: Option<String>,
    sender_avatar: Option<String>,
    direct: bool,
    title: Option<(String, Option<String>)>,
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
    /// Sender, recipient and room of this alert, formatted by `notification_title`.
    pub fn titled(mut self, account_name: &str, account_id: &str, room_name: Option<&str>, other_accounts: &[&str]) -> Self {
        self.title = Some(notification_title(self.sender.as_deref(), account_name, account_id, room_name, other_accounts, self.direct));
        self
    }
    /// Picture for the OS alert: the sender's, else the room's; none for system alerts without either.
    pub fn avatar_mxc<'a>(&'a self, room_avatar: Option<&'a str>) -> Option<&'a str> {
        self.sender_avatar.as_deref().or(room_avatar)
    }
    pub fn title(&self) -> &str {
        match (&self.title, &self.origin) {
            (Some((title, _)), _) => title,
            (None, Some((_, name))) => name,
            _ => "Gnobbi",
        }
    }
    pub fn subtitle(&self) -> Option<&str> {
        self.title.as_ref()?.1.as_deref()
    }
    /// One line for platforms without a subtitle.
    pub fn title_line(&self) -> String {
        match self.subtitle() {
            Some(room) => format!("{} · {room}", self.title()),
            None => self.title().to_owned(),
        }
    }
    pub fn body(&self) -> String {
        self.body.clone()
    }
}

#[cfg(test)]
mod title_tests {
    use super::notification_title as t;

    #[test]
    fn titles() {
        assert_eq!(t(Some("Meska"), "Marco", "@m:a.it", Some("Casa"), &[], false), ("Meska".into(), None));
        assert_eq!(t(None, "Marco", "@m:a.it", None, &[], true).0, "Marco");
        assert_eq!(t(Some("Meska"), "Marco", "@m:a.it", Some("Casa"), &["Luca"], true), ("Meska → Marco".into(), None));
        assert_eq!(t(Some("Meska"), "Marco", "@m:a.it", Some("Casa"), &["Luca"], false), ("Meska → Marco".into(), Some("Casa".into())));
        assert_eq!(t(Some("Meska"), "Marco", "@meska:example.org", None, &["Marco"], true).0, "Meska → Marco (@meska:example.org)");
        assert_eq!(t(Some("Meska"), "", "@m:a.it", None, &["Luca"], true).0, "Meska → @m:a.it");
        let long = "x".repeat(100);
        let (title, room) = t(Some(&long), &long, "@m:a.it", Some(&long), &["Luca"], false);
        assert_eq!(title.chars().count(), 40 + 3 + 40);
        assert_eq!(room.unwrap().chars().count(), 60);
    }
}

#[cfg(test)]
mod decision_tests {
    use super::{NotifyKind::*, RoomNotifyMode::*, *};

    #[test]
    fn decision_table() {
        for low in [false, true] {
            for kind in [Normal, Mention, Keyword, Invite] {
                assert!(!should_notify(Mute, low, kind), "Mute never notifies");
            }
        }
        // (mode, low priority, kind) -> expected
        for (mode, low, kind, want) in [
            (AllMessages, false, Normal, true),
            (AllMessages, true, Normal, false),
            (AllMessages, true, Keyword, false),
            (AllMessages, true, Mention, true),
            (AllMessages, true, Invite, true),
            (AllMessages, false, Keyword, true),
            (MentionsAndKeywordsOnly, false, Normal, false),
            (MentionsAndKeywordsOnly, false, Keyword, true),
            (MentionsAndKeywordsOnly, false, Mention, true),
            (MentionsAndKeywordsOnly, true, Normal, false),
            (MentionsAndKeywordsOnly, true, Keyword, false),
            (MentionsAndKeywordsOnly, true, Mention, true),
            (MentionsAndKeywordsOnly, false, Invite, true),
            (MentionsAndKeywordsOnly, true, Invite, true),
        ] {
            assert_eq!(should_notify(mode, low, kind), want, "{mode:?} low={low} {kind:?}");
        }
    }

    #[test]
    fn labels_are_the_agreed_ones() {
        assert_eq!(RoomNotifyMode::ALL.map(RoomNotifyMode::label), ["All messages", "Mentions only", "Muted"]);
    }
}
