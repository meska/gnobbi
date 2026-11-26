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
    /// What the attachment is, never the caption: «Image», «Video · 0:12», «File · a.pdf · 2 MB».
    /// The first part is an English key for the UI to translate (see `Media::translated`); the
    /// rest (time, size, file name) is the same in every language.
    pub label: String,
    /// A picture whose format can move (GIF, WebP, APNG): «Add to my stickers» keeps the original.
    pub animated: bool,
    /// Pixel size the sender declared for a picture or video, to lay it out before it is loaded.
    pub dims: Option<(u32, u32)>,
    /// Set when the sender's client marked this event as one item of an album (see `docs/decisioni.md`).
    pub album: Option<crate::sending::Album>,
}

/// Formats that can be animated; a WebP may still be a still one, the original is kept either way.
pub fn can_animate(mimetype: Option<&str>) -> bool {
    matches!(mimetype, Some("image/gif" | "image/webp" | "image/apng"))
}

impl Media {
    /// `label` with its first part passed through `tr`.
    // shortcut: splits at the first « · »; a label kind never contains one, upgrade if one does.
    pub fn translated(&self, tr: impl Fn(&str) -> String) -> String {
        match self.label.split_once(" · ") {
            Some((kind, rest)) => format!("{} · {rest}", tr(kind)),
            None => tr(&self.label),
        }
    }
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
/// MSC3245 marker. The waveform (0..=1, from the MSC1767 amplitudes 0..=1024) is what the sender
/// sent; when it sent none the bubble takes it from the same decode that feeds the player.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub duration_ms: Option<u64>,
    pub size: Option<u64>,
    pub voice: bool,
    pub waveform: Vec<f32>,
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
    /// Set on a status row: what it says, worded by the UI (`body` is empty).
    pub status_row: Option<crate::status_line::StatusRow>,
    /// Set on a poll start: `body` is its question. Private user data: never log.
    pub poll: Option<matrix_domain::Poll>,
    /// The event could not be decrypted (no keys): the only rows a key recovery can fix.
    pub utd: bool,
    /// The reactions under the message, in order of first appearance.
    pub reactions: Vec<Reaction>,
}

/// Names kept per reaction: the popup shows 10 and «+N»; `Reaction::count` is the true number.
pub const MAX_REACTION_SENDERS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionSender {
    pub user_id: String,
    pub name: Option<String>,
}

/// One emoji under a message. `pending`: our own reaction is still on its way to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub key: String,
    pub count: usize,
    pub senders: Vec<ReactionSender>,
    pub mine: bool,
    pub pending: bool,
}

/// Groups `(key, [(user, pending)])` as the Timeline holds them (first appearance order). A sender
/// counts once per key; empty keys and groups are dropped (a redacted reaction leaves the group).
pub fn aggregate_reactions(groups: Vec<(String, Vec<(String, bool)>)>, me: &str) -> Vec<Reaction> {
    let mut out: Vec<Reaction> = Vec::new();
    for (key, who) in groups {
        if key.is_empty() {
            continue;
        }
        let at = match out.iter().position(|r| r.key == key) {
            Some(at) => at,
            None => {
                out.push(Reaction { key, count: 0, senders: Vec::new(), mine: false, pending: false });
                out.len() - 1
            }
        };
        let r = &mut out[at];
        for (user, pending) in who {
            if r.senders.iter().any(|s| s.user_id == user) {
                continue;
            }
            r.count += 1;
            if user == me {
                r.mine = true;
                r.pending = pending;
            }
            if r.senders.len() < MAX_REACTION_SENDERS {
                r.senders.push(ReactionSender { user_id: user, name: None });
            }
        }
    }
    out.retain(|r| r.count > 0);
    out
}

#[cfg(test)]
mod reaction_tests {
    use super::*;
    fn g(key: &str, who: &[(&str, bool)]) -> (String, Vec<(String, bool)>) {
        (key.into(), who.iter().map(|(u, p)| ((*u).into(), *p)).collect())
    }

    #[test]
    fn counts_flags_and_keeps_first_appearance_order() {
        let r = aggregate_reactions(vec![g("🎉", &[("@b:t", false)]), g("👍", &[("@a:t", false), ("@b:t", false)])], "@a:t");
        assert_eq!(r.iter().map(|r| (r.key.as_str(), r.count, r.mine)).collect::<Vec<_>>(), [("🎉", 1, false), ("👍", 2, true)]);
        assert_eq!(r[1].senders[0].user_id, "@a:t");
    }

    #[test]
    fn the_same_sender_counts_once_and_the_same_key_merges() {
        let r = aggregate_reactions(vec![g("👍", &[("@b:t", false), ("@b:t", false)]), g("👍", &[("@c:t", false)])], "@a:t");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].count, 2);
        assert!(!r[0].mine);
    }

    #[test]
    fn a_removed_reaction_leaves_nothing_and_pending_is_only_ours() {
        assert!(aggregate_reactions(vec![g("👍", &[]), g("", &[("@b:t", false)])], "@a:t").is_empty());
        let r = aggregate_reactions(vec![g("👍", &[("@a:t", true), ("@b:t", true)])], "@a:t");
        assert!(r[0].mine && r[0].pending);
        let r = aggregate_reactions(vec![g("👍", &[("@b:t", true)])], "@a:t");
        assert!(!r[0].pending);
    }

    #[test]
    fn names_are_capped_but_the_count_is_true() {
        let who: Vec<(String, bool)> = (0..30).map(|n| (format!("@u{n}:t"), false)).collect();
        let r = aggregate_reactions(vec![("👍".into(), who)], "@a:t");
        assert_eq!((r[0].count, r[0].senders.len()), (30, MAX_REACTION_SENDERS));
    }
}

/// The message a reply answers. `author`/`preview` are None unless `state` is `Resolved`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyRef {
    pub event_id: EventId,
    pub author: Option<String>,
    pub preview: Option<String>,
    /// `preview` is an English key standing for an attachment without caption («Photo»), not the
    /// original's text: the UI translates it.
    pub placeholder: bool,
    pub state: ReplyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyState {
    Resolved,
    /// A request for the original is running.
    Loading,
    /// The original was redacted.
    Deleted,
    /// The original is in the room but cannot be decrypted.
    Encrypted,
    /// The original does not exist, is hidden, or the request failed.
    Unavailable,
}

/// What the SDK says about the original (`TimelineDetails` of `in_reply_to`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyDetail {
    Ready,
    Pending,
    Unavailable,
    Error,
}

/// Where the quote reads the original from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplySource {
    /// An event already in the room's timeline (always the freshest copy).
    Timeline,
    /// The SDK's own `in_reply_to` detail.
    Detail,
    Loading,
    Unavailable,
}

/// How long a failed request waits before the next one; and how long an unconfirmed request still shows «loading».
pub const REPLY_RETRY: std::time::Duration = std::time::Duration::from_secs(60);
const REPLY_SETTLE: std::time::Duration = std::time::Duration::from_secs(10);

/// Decide where a quote comes from and whether to start a request. `asked` is the age of our last request for this reply.
pub fn reply_plan(detail: ReplyDetail, in_timeline: bool, asked: Option<std::time::Duration>) -> (ReplySource, bool) {
    use ReplyDetail::*;
    if in_timeline {
        return (ReplySource::Timeline, false);
    }
    match (detail, asked) {
        (Ready, _) => (ReplySource::Detail, false),
        (Pending, _) => (ReplySource::Loading, false),
        (Unavailable, None) => (ReplySource::Loading, true),
        (Unavailable, Some(a)) if a < REPLY_SETTLE => (ReplySource::Loading, false),
        (Error, None) => (ReplySource::Unavailable, true),
        (Error, Some(a)) if a >= REPLY_RETRY => (ReplySource::Unavailable, true),
        (Unavailable | Error, Some(_)) => (ReplySource::Unavailable, false),
    }
}

/// One-line quote text: the media label when there is no caption, else the first line cut at `max` chars.
pub fn quote_preview(text: &str, media: Option<&Media>, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let Some(line) = line else {
        return media.map_or_else(String::new, |m| match m.kind {
            MediaKind::Image => "Photo".into(),
            MediaKind::Video => "Video".into(),
            MediaKind::Audio if m.label.starts_with("Voice message") => "Voice message".into(), // label chosen by message_label
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
        let m = |kind| Media { kind, label: "x".into(), animated: false, dims: None, album: None };
        assert_eq!(quote_preview("", Some(&m(MediaKind::Image)), 40), "Photo");
        assert_eq!(quote_preview("didascalia", Some(&m(MediaKind::Image)), 40), "didascalia");
        assert_eq!(quote_preview("", None, 40), "");
        let a = |label: &str| Media { kind: MediaKind::Audio, label: label.into(), animated: false, dims: None, album: None };
        assert_eq!(quote_preview("", Some(&a("Voice message · 0:04")), 40), "Voice message");
        assert_eq!(quote_preview("", Some(&a("Audio · 0:04")), 40), "Audio");
    }

    #[test]
    fn reply_plan_resolves_in_levels() {
        use std::time::Duration as D;
        use {ReplyDetail::*, ReplySource as S};
        assert_eq!(reply_plan(Ready, false, None), (S::Detail, false));
        // The original is already in the timeline: no request, whatever the SDK detail says.
        assert_eq!(reply_plan(Unavailable, true, None), (S::Timeline, false));
        assert_eq!(reply_plan(Error, true, Some(D::from_secs(1))), (S::Timeline, false));
        // Not in the timeline: one request, «loading» meanwhile.
        assert_eq!(reply_plan(Unavailable, false, None), (S::Loading, true));
        assert_eq!(reply_plan(Unavailable, false, Some(D::from_secs(1))), (S::Loading, false));
        assert_eq!(reply_plan(Pending, false, Some(D::from_secs(1))), (S::Loading, false));
        // Failed: «unavailable», retried only after the wait.
        assert_eq!(reply_plan(Error, false, Some(D::from_secs(5))), (S::Unavailable, false));
        assert_eq!(reply_plan(Error, false, Some(REPLY_RETRY)), (S::Unavailable, true));
        assert_eq!(reply_plan(Unavailable, false, Some(D::from_secs(30))), (S::Unavailable, false));
    }

    /// D4: an original that is not in memory goes from «loading» to its text, or to «unavailable» when the fetch fails or times out.
    #[test]
    fn reply_not_in_memory_leaves_loading() {
        use std::time::Duration as D;
        use {ReplyDetail::*, ReplySource as S};
        // First read starts the single-event fetch; while it runs the quote says «loading».
        assert_eq!(reply_plan(Unavailable, false, None), (S::Loading, true));
        assert_eq!(reply_plan(Pending, false, Some(D::from_secs(2))), (S::Loading, false));
        // Fetched: the quote reads the text (no new request).
        assert_eq!(reply_plan(Ready, false, None), (S::Detail, false));
        // Failed or timed out: «unavailable», no request storm.
        assert_eq!(reply_plan(Error, false, Some(D::from_secs(1))), (S::Unavailable, false));
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

