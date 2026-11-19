//! View-scoped read-only history DTOs. Opaque cursors are not event IDs.
use crate::RequestTag;
use matrix_domain::EventId;

pub struct HistoryRequest {
    pub tag: RequestTag,
    pub before: Option<String>,
    /// Room opening: saved messages first, then only what was missed.
    pub open: bool,
}

/// Event author: Matrix user ID plus display name when the room member is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySender {
    pub id: String,
    pub name: Option<String>,
    /// mxc URI of the member avatar when the room member has one; not downloaded here.
    pub avatar: Option<String>,
}

impl HistorySender {
    pub fn new(id: &str, name: Option<&str>) -> Self {
        Self {
            id: id.to_owned(),
            name: name
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned),
            avatar: None,
        }
    }

    pub fn with_avatar(mut self, avatar: Option<String>) -> Self {
        self.avatar = avatar;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
}

/// Attachment of an `m.image` / `m.video` / `m.audio` / `m.file` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub kind: MediaKind,
    /// What the attachment is, never the caption: «Immagine», «Video · 0:12», «File · a.pdf · 2 MB».
    pub label: String,
}

/// Name offered in the save dialog: the sender controls it, so keep one safe path component.
pub fn proposed_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last.chars().filter(|c| !c.is_control()).collect();
    let clean: String = clean.trim_start_matches(['.', ' ']).chars().take(120).collect();
    if clean.is_empty() { "allegato".into() } else { clean }
}

/// «m:ss» of a duration, as drawn on a video bubble.
pub fn duration_label(duration_ms: u64) -> String {
    let s = duration_ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Video facts from the event `info`; the thumbnail is only a source to fetch later.
#[derive(Debug, Clone, Default)]
pub struct VideoInfo {
    pub duration_ms: Option<u64>,
    pub size: Option<u64>,
    #[cfg(feature = "sdk")]
    pub thumbnail: Option<matrix_sdk::ruma::events::room::MediaSource>,
}

/// Audio facts from the event `info`. An `m.audio` row is a voice note when it carries the
/// MSC3245 marker. The waveform is what the sender sent; when it sent none the bubble takes
/// it from the same decode that feeds the player.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub duration_ms: Option<u64>,
    pub size: Option<u64>,
    pub voice: bool,
    pub waveform: Vec<f32>,
}

/// The waveform a client attaches to a voice note: MSC1767 (`org.matrix.msc1767.audio`)
/// or the MSC3245 `info.waveform` field. Empty when the sender sent none.
#[cfg(feature = "sdk")]
pub fn waveform_of(content: &serde_json::Value) -> Vec<f32> {
    let peaks = content
        .get("org.matrix.msc1767.audio")
        .and_then(|audio| audio.get("waveform"))
        .or_else(|| content.get("info").and_then(|info| info.get("waveform")));
    peaks
        .and_then(|w| serde_json::from_value::<Vec<f32>>(w.clone()).ok())
        .map(|w| w.into_iter().map(|v| v.clamp(0.0, 1.0)).collect())
        .unwrap_or_default()
}

pub struct HistoryEvent {
    pub event_id: EventId,
    pub body: String,
    pub outgoing: bool,
    /// The event was redacted (deleted): `body` is the placeholder, never the old text.
    pub redacted: bool,
    pub timestamp_ms: u64,
    pub sender: HistorySender,
    pub video: Option<VideoInfo>,
    /// Set on an `m.audio` row: the bubble draws its own player, never the video one.
    pub audio: Option<AudioInfo>,
    /// Set on attachment rows: `body`, `styles` and `mentions` are then the caption, empty when there is none.
    pub media: Option<Media>,
    /// File name of an image, video, audio or file message: the row can be saved.
    pub attachment_name: Option<String>,
    /// Size declared by the sender for that attachment, when it says one.
    pub attachment_size: Option<u64>,
    /// Bold, italic, code… of a formatted body, as byte ranges of `body` (empty for plain text).
    pub styles: Vec<crate::rich_text::StyleSpan>,
    /// User, «@room» and room mentions of a formatted body, as byte ranges of `body`.
    pub mentions: Vec<crate::rich_text::MentionSpan>,
    /// The push rules say this message is for us (Element's `tweaks.highlight`).
    pub highlighted: bool,
    /// Set on the root of a thread.
    pub thread: Option<ThreadInfo>,
    /// Set on a reply inside a thread: the thread's root.
    pub thread_root: Option<EventId>,
    /// Set on a reply: what it answers.
    pub reply_to: Option<ReplyRef>,
    /// A status row (join, invite, rename…): `body` is its wording and `sender` the person it shows.
    pub status: bool,
    /// Set on a poll start: `body` is its question. Private user data: never log.
    pub poll: Option<matrix_domain::Poll>,
    /// The event could not be decrypted (no keys): the only rows a key recovery can fix.
    pub utd: bool,
}

/// The message a reply answers. `author`/`preview` are None while the original is not loaded or was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyRef {
    pub event_id: EventId,
    pub author: Option<String>,
    pub preview: Option<String>,
}

/// Shown in the quote when the original is not available.
pub const REPLY_UNAVAILABLE: &str = "Messaggio non disponibile";

/// One-line quote text: the media label when there is no caption, else the first line cut at `max` chars.
pub fn quote_preview(text: &str, media: Option<&Media>, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let Some(line) = line else {
        return media.map_or_else(String::new, |m| match m.kind {
            MediaKind::Image => "Foto".into(),
            MediaKind::Video => "Video".into(),
            MediaKind::Audio => "Audio".into(),
            MediaKind::File => "File".into(),
        });
    };
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod quote_tests {
    use super::*;

    #[test]
    fn quote_is_one_truncated_line_or_media_label() {
        assert_eq!(quote_preview("\n  ciao\naltro", None, 10), "ciao");
        assert_eq!(quote_preview("abcdefghij", None, 5), "abcde…");
        assert_eq!(quote_preview("àèìòùàèìòù", None, 3), "àèì…");
        let m = |kind| Media { kind, label: "x".into() };
        assert_eq!(quote_preview("", Some(&m(MediaKind::Image)), 40), "Foto");
        assert_eq!(quote_preview("didascalia", Some(&m(MediaKind::Image)), 40), "didascalia");
        assert_eq!(quote_preview("", None, 40), "");
    }
}

/// The summary under a thread root, as the Timeline keeps it (live).
// shortcut: the SDK gives the count and the latest reply only, so one participant; upgrade if it exposes all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub num_replies: u32,
    pub latest_sender: Option<HistorySender>,
    /// Replies by others after our own read marker of the thread (0 while the thread is unknown).
    pub unread: u32,
}

/// One thread opened: `root` is None when the room's own timeline does not hold it (the thread
/// timeline alone does not return it reliably); `replies` are oldest first.
pub struct ThreadPage {
    pub root: Option<HistoryEvent>,
    pub replies: Vec<HistoryEvent>,
    pub reactions: Vec<ReactionCount>,
    /// The thread timeline says its start was reached. Servers get this wrong when the thread is
    /// bigger than the sync window: compare `replies.len()` with `ThreadSummary::num_replies`.
    pub at_start: bool,
}

/// How many people reacted to a message with one emoji.
pub struct ReactionCount {
    pub target: EventId,
    pub key: String,
    pub count: usize,
}

pub struct HistoryPage {
    pub tag: RequestTag,
    /// Chronological order, oldest first. No body/ID/cursor Debug or logging.
    pub events: Vec<HistoryEvent>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    Cancelled,
    WrongScope,
    Unavailable,
    Transport,
}

pub struct HistoryResult {
    pub tag: RequestTag,
    pub page: Result<HistoryPage, HistoryError>,
}

#[cfg(test)]
mod sender_avatar_tests {
    use super::*;

    #[test]
    fn avatar_is_absent_unless_given() {
        assert_eq!(HistorySender::new("@a:t", None).avatar, None);
        let s = HistorySender::new("@a:t", None).with_avatar(Some("mxc://t/a".into()));
        assert_eq!(s.avatar.as_deref(), Some("mxc://t/a"));
    }
}

#[cfg(test)]
mod proposed_name_tests {
    use super::proposed_name;

    #[test]
    fn keeps_one_safe_component() {
        assert_eq!(proposed_name("../x.pdf"), "x.pdf");
        assert_eq!(proposed_name("C:\\tmp\\y.txt"), "y.txt");
        assert_eq!(proposed_name(".bashrc"), "bashrc");
        assert_eq!(proposed_name("a\nb"), "ab");
        assert_eq!(proposed_name(""), "allegato");
        assert_eq!(proposed_name("dir/"), "allegato");
        assert_eq!(proposed_name(&"n".repeat(500)).len(), 120);
    }
}

#[cfg(all(test, feature = "sdk"))]
mod waveform_tests {
    use super::waveform_of;

    #[test]
    fn the_msc1767_block_wins_and_the_values_are_clamped() {
        let content = serde_json::json!({
            "msgtype": "m.audio",
            "org.matrix.msc1767.audio": { "duration": 3000, "waveform": [0.0, 0.5, 2.0, -1.0] },
            "info": { "waveform": [9.0] },
        });
        assert_eq!(waveform_of(&content), vec![0.0, 0.5, 1.0, 0.0]);
    }

    #[test]
    fn the_msc3245_info_field_is_the_fallback() {
        let content = serde_json::json!({"msgtype":"m.audio","info":{"waveform":[0.25,0.75]}});
        assert_eq!(waveform_of(&content), vec![0.25, 0.75]);
    }

    #[test]
    fn a_note_without_a_waveform_is_empty_never_an_error() {
        assert!(waveform_of(&serde_json::json!({"msgtype":"m.audio"})).is_empty());
        assert!(waveform_of(&serde_json::json!({"waveform": "nonsense"})).is_empty());
        assert!(waveform_of(&serde_json::json!(5)).is_empty());
    }
}
