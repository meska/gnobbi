//! One-line list text: shared by the chat list rows (and any other list that must not wrap).
//! The row height comes from its content, so every row has the same height.

use crate::i18n;
use matrix_engine::status_line::{Status, StatusRow};

pub const AVATAR_PX: f32 = 40.;
pub const ROW_PAD_Y: f32 = matrix_domain::design::SPACING_LIST_ROW_PAD_Y;
pub const LIST_PAD_X_PX: f32 = 10.;
pub const AVATAR_GAP_PX: f32 = 10.;
pub const NAME_PX: f32 = 15.;
pub const NAME_LINE_PX: f32 = 20.;
pub const PREVIEW_PX: f32 = 12.;
pub const PREVIEW_LINE_PX: f32 = 16.;
pub const TEXT_GAP_PX: f32 = matrix_domain::design::SPACING_LIST_TEXT_GAP;
/// Name row (badge or time sits inside it, never taller) + gap + preview row.
pub const TEXT_BLOCK_PX: f32 = NAME_LINE_PX + TEXT_GAP_PX + PREVIEW_LINE_PX;
pub const ROW_HEIGHT_PX: f32 = ROW_PAD_Y * 2.
    + if TEXT_BLOCK_PX > AVATAR_PX {
        TEXT_BLOCK_PX
    } else {
        AVATAR_PX
    };

/// Line breaks and runs of whitespace become single spaces, so the text fits one ellipsized line.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Sender prefix of the compact deleted row: «Anna:», one line.
pub fn redacted_name(name: &str) -> String {
    format!("{}:", one_line(name))
}

/// The engine words a few whole-message placeholders in English; shown in the active language.
// shortcut: matched by exact text, so a message that says exactly «Location» is translated too; upgrade with a flag on the event if that matters.
pub fn engine_placeholder(text: &str) -> &str {
    match text {
        "Image" => i18n::t("Image"),
        "Location" => i18n::t("Location"),
        "Verification request" => i18n::t("Verification request"),
        "Message without text" => i18n::t("Message without text"),
        _ => text,
    }
}

/// Replaces each `{name}` of `template` with its value, in one pass: a value that contains `{x}` stays as it is.
fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        match args.iter().find(|(k, _)| tail[1..].starts_with(k) && tail[1 + k.len()..].starts_with('}')) {
            Some((k, v)) => {
                out.push_str(v);
                rest = &tail[k.len() + 2..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out + rest
}

/// The sentence of a status row in the active language («Anna è entrato/a nella stanza»), with
/// the reason when there is one. Worded when drawn, so a language change reaches rows already shown.
pub fn status_row_text(row: &StatusRow) -> String {
    use Status::*;
    let someone = |n: &str| if n.is_empty() { i18n::t("A user").to_owned() } else { n.to_owned() };
    let (actor, subject) = (someone(&row.actor), someone(&row.subject));
    let (a, s) = (actor.as_str(), subject.as_str());
    let two = |tpl: &str| fill(tpl, &[("actor", a), ("subject", s)]);
    let text = match &row.status {
        Created => two(i18n::t("{actor} created this room.")),
        CreatedDirect => two(i18n::t("{actor} created this chat.")),
        Invited => two(i18n::t("{actor} invited {subject}")),
        Joined => two(i18n::t("{subject} joined the room")),
        Left => two(i18n::t("{subject} left the room")),
        Banned => two(i18n::t("{actor} banned {subject}")),
        Unbanned => two(i18n::t("{actor} unbanned {subject}")),
        Kicked => two(i18n::t("{actor} removed {subject} from the room")),
        InvitationRejected => two(i18n::t("{subject} declined the invitation")),
        InvitationRevoked => two(i18n::t("{actor} revoked the invitation for {subject}")),
        RoomName(n) if n.is_empty() => two(i18n::t("{actor} removed the room name")),
        RoomName(n) => fill(i18n::t("{actor} changed the room name to «{name}»"), &[("actor", a), ("name", n)]),
        RoomTopic(t) if t.is_empty() => two(i18n::t("{actor} removed the room topic")),
        RoomTopic(t) => fill(i18n::t("{actor} changed the room topic to «{topic}»"), &[("actor", a), ("topic", t)]),
        RoomAvatar => two(i18n::t("{actor} changed the room avatar")),
        // Tuwunel keeps "" as display name: blank counts as absent (as Synapse does).
        DisplayName { old, new } => match (old.as_deref().map(str::trim).filter(|x| !x.is_empty()), new.as_deref().map(str::trim).filter(|x| !x.is_empty())) {
            (Some(old), Some(new)) => fill(i18n::t("{old} changed their name to {new}"), &[("old", old), ("new", new)]),
            (None, Some(new)) => fill(i18n::t("{subject} set their name to {new}"), &[("subject", s), ("new", new)]),
            (_, None) => two(i18n::t("{subject} removed their display name")),
        },
        MemberAvatar => two(i18n::t("{subject} changed their avatar")),
    };
    match row.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => fill(i18n::t("{text} (reason: {reason})"), &[("text", &text), ("reason", r)]),
        None => text,
    }
}

/// Status row text with the event time after it («Anna è entrato/a nella stanza · 01:20»);
/// one string, so a long row wraps and centres as a whole. `time` is the bubble stamp's.
pub fn status_with_time(text: &str, time: &str) -> String {
    let text = one_line(text);
    if time.is_empty() { text } else { format!("{text} · {time}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: Status, actor: &str, subject: &str, reason: Option<&str>) -> String {
        status_row_text(&StatusRow { status, actor: actor.into(), subject: subject.into(), reason: reason.map(Into::into) })
    }

    #[test]
    fn status_rows_in_english_and_italian() {
        let it = |f: &dyn Fn() -> String| { i18n::apply(i18n::Preference::Fixed("it"), None); let r = f(); i18n::apply(i18n::Preference::Fixed("en"), None); r };
        assert_eq!(row(Status::Joined, "Bob", "Bob", Some("  ")), "Bob joined the room");
        assert_eq!(it(&|| row(Status::Joined, "Bob", "Bob", Some("  "))), "Bob è entrato/a nella stanza");
        assert_eq!(row(Status::Kicked, "Ann", "Bob", Some("spam")), "Ann removed Bob from the room (reason: spam)");
        assert_eq!(it(&|| row(Status::Kicked, "Ann", "Bob", Some("spam"))), "Ann ha rimosso Bob dalla stanza (motivo: spam)");
        assert_eq!(it(&|| row(Status::Created, "meska", "", None)), "meska ha creato questa stanza.");
        assert_eq!(it(&|| row(Status::CreatedDirect, "meska", "", None)), "meska ha creato questo MD.");
        assert_eq!(it(&|| row(Status::RoomName("Casa".into()), "meska", "", None)), "meska ha cambiato il nome della stanza in «Casa»");
        assert_eq!(it(&|| row(Status::RoomTopic(String::new()), "meska", "", None)), "meska ha rimosso l'argomento della stanza");
        assert_eq!(it(&|| row(Status::DisplayName { old: Some("Mar".into()), new: Some("Marco".into()) }, "", "Marco", None)), "Mar ha cambiato il nome in Marco");
        assert_eq!(it(&|| row(Status::Unbanned, "A", "B", None)), "A ha revocato il ban a B");
        assert_eq!(it(&|| row(Status::InvitationRevoked, "A", "B", None)), "A ha revocato l'invito a B");
    }

    #[test]
    fn status_rows_never_leave_a_hole() {
        let id = "@bob:x.test";
        for s in [Status::Created, Status::CreatedDirect, Status::Invited, Status::Joined, Status::Left, Status::Banned, Status::Unbanned, Status::Kicked,
            Status::InvitationRejected, Status::InvitationRevoked, Status::RoomName(String::new()), Status::RoomTopic(String::new()),
            Status::RoomAvatar, Status::MemberAvatar, Status::DisplayName { old: None, new: Some("N".into()) },
            Status::DisplayName { old: Some("O".into()), new: None }, Status::DisplayName { old: None, new: None }] {
            let t = row(s.clone(), id, id, None);
            assert!(t.contains(id) && !t.contains("  ") && !t.contains("«»") && !t.contains('{') && t.trim() == t, "{s:?}: {t}");
        }
        assert_eq!(row(Status::Joined, "", "", None), "A user joined the room");
        // a name that looks like a placeholder is not filled twice
        assert_eq!(row(Status::Invited, "{subject}", "Bob", None), "{subject} invited Bob");
    }

    #[test]
    fn redacted_row_text_is_plain_one_line() {
        assert_eq!(redacted_name("Anna\nRossi"), "Anna Rossi:");
    }

    /// Lines the row paints: both texts are forced to one line; an empty one still reserves its line.
    fn content_height(name: &str, preview: &str) -> f32 {
        let lines = |t: &str| one_line(t).lines().count().max(1) as f32;
        let (n, p) = (lines(name), lines(preview));
        assert_eq!((n, p), (1., 1.));
        NAME_LINE_PX * n + TEXT_GAP_PX + PREVIEW_LINE_PX * p
    }

    #[test]
    fn content_never_taller_than_the_row() {
        let long = "x".repeat(300);
        let cases: [(&str, &str, &str); 6] = [
            (&long, "ciao", "3"),                                 // long name, no spaces
            ("Claudio Martini 😀", "ok 😀\n\ngrazie\r\n👍", "1"), // emoji and multiline preview
            ("", "ciao", ""),                                     // empty name
            ("Claudio", "", ""),                                  // empty preview
            ("Claudio", "ciao", "999"),                           // three-digit badge
            (
                "漢字漢字漢字漢字漢字漢字",
                "ＷＩＤＥ　ｗｉｄｅ　漢字\u{2028}ok",
                "12",
            ), // wide chars, selected + unread
        ];
        for (name, preview, _badge) in cases {
            assert!(
                content_height(name, preview) <= ROW_HEIGHT_PX - 2. * ROW_PAD_Y,
                "{name:?}/{preview:?}"
            );
            assert!(!one_line(preview).contains(['\n', '\r', '\u{2028}']));
        }
    }

    const _: () = assert!(AVATAR_PX + 2. * ROW_PAD_Y <= ROW_HEIGHT_PX);

    #[test]
    fn status_row_shows_the_event_time() {
        assert_eq!(status_with_time("Anna è entrato/a\nnella stanza", "01:20"), "Anna è entrato/a nella stanza · 01:20");
        assert_eq!(status_with_time("Anna", ""), "Anna");
    }

    #[test]
    fn one_line_collapses_breaks() {
        assert_eq!(one_line("a\n\n b\r\nc "), "a b c");
        assert_eq!(one_line(""), "");
    }
}
