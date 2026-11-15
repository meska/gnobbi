//! Italian text of the status rows (join, invite, room rename…). The SDK timeline decides
//! what happened; this only words it. Names are display names, falling back to the user ID.

/// What the SDK timeline item says happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Created,
    Invited,
    Joined,
    Left,
    Banned,
    Unbanned,
    Kicked,
    /// Creation of a direct chat.
    CreatedDirect,
    InvitationRejected,
    InvitationRevoked,
    /// Room name (empty = removed).
    RoomName(String),
    /// Room topic (empty = removed).
    RoomTopic(String),
    RoomAvatar,
    /// Previous display name of the member (None = had none), new one in `subject`.
    DisplayName { old: Option<String>, new: Option<String> },
    MemberAvatar,
}

/// Longest fallback taken from an ID before it is cut with an ellipsis.
const MAX_ID_LABEL: usize = 24;

/// Name shown in a status row: the display name; else the part of the mxid before the domain
/// (a bridge number, masked or not, shows as it is); else the whole ID. IDs are cut with «…».
/// A name equal to the ID, or the SDK's «Unknown user» placeholder, counts as no name.
pub fn display_label(name: Option<&str>, id: &str) -> String {
    let (id, name) = (id.trim(), name.map(str::trim).unwrap_or_default());
    if !name.is_empty() && !name.eq_ignore_ascii_case(id) && !name.eq_ignore_ascii_case("unknown user") {
        return name.to_owned();
    }
    let local = id.trim_start_matches('@').split(':').next().unwrap_or_default();
    let pick = if local.is_empty() { id } else { local };
    if pick.is_empty() {
        return "Un utente".to_owned();
    }
    if pick.chars().count() <= MAX_ID_LABEL {
        return pick.to_owned();
    }
    format!("{}…", pick.chars().take(MAX_ID_LABEL - 1).collect::<String>())
}

/// `actor` did it to `subject` (the same person for self-service changes).
pub fn status_text(status: &Status, actor: &str, subject: &str) -> String {
    match status {
        Status::Created => format!("{actor} ha creato questa stanza."),
        Status::CreatedDirect => format!("{actor} ha creato questo MD."),
        Status::Invited => format!("{actor} ha invitato {subject}"),
        Status::Joined => format!("{subject} è entrato/a nella stanza"),
        Status::Left => format!("{subject} è uscito/a dalla stanza"),
        Status::Banned => format!("{actor} ha bandito {subject}"),
        Status::Unbanned => format!("{actor} ha revocato il ban a {subject}"),
        Status::Kicked => format!("{actor} ha rimosso {subject} dalla stanza"),
        Status::InvitationRejected => format!("{subject} ha rifiutato l'invito"),
        Status::InvitationRevoked => format!("{actor} ha revocato l'invito a {subject}"),
        Status::RoomName(n) if n.is_empty() => format!("{actor} ha rimosso il nome della stanza"),
        Status::RoomName(n) => format!("{actor} ha cambiato il nome della stanza in «{n}»"),
        Status::RoomTopic(t) if t.is_empty() => format!("{actor} ha rimosso l'argomento della stanza"),
        Status::RoomTopic(t) => format!("{actor} ha cambiato l'argomento della stanza in «{t}»"),
        Status::RoomAvatar => format!("{actor} ha cambiato l'avatar della stanza"),
        // Tuwunel keeps "" as display name: treat blank like absent (as Synapse does).
        Status::DisplayName { old, new } => match (old.as_deref().map(str::trim).filter(|s| !s.is_empty()), new.as_deref().map(str::trim).filter(|s| !s.is_empty())) {
            (Some(old), Some(new)) => format!("{old} ha cambiato il nome in {new}"),
            (None, Some(new)) => format!("{subject} ha impostato il nome {new}"),
            (_, None) => format!("{subject} ha rimosso il nome visualizzato"),
        },
        Status::MemberAvatar => format!("{subject} ha cambiato l'avatar"),
    }
}

/// A leave the SDK could not classify (`MembershipChange::None`, e.g. a backfilled event without
/// `prev_content`): the sender leaving is «Left», anyone else removing them is «Kicked».
pub fn unclassified_leave(sender_is_subject: bool) -> Status {
    if sender_is_subject { Status::Left } else { Status::Kicked }
}

/// Appends the reason given on the membership event, when there is one.
pub fn with_reason(text: String, reason: Option<&str>) -> String {
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => format!("{text} (motivo: {r})"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_rows_with_and_without_reason() {
        let row = |st: Status, actor: &str, subject: &str, reason: Option<&str>| with_reason(status_text(&st, actor, subject), reason);
        // sender == state_key
        assert_eq!(row(unclassified_leave(true), "Bob", "Bob", None), "Bob è uscito/a dalla stanza");
        assert_eq!(row(unclassified_leave(true), "Bob", "Bob", Some("ciao")), "Bob è uscito/a dalla stanza (motivo: ciao)");
        assert_eq!(row(Status::InvitationRejected, "Bob", "Bob", Some("no")), "Bob ha rifiutato l'invito (motivo: no)");
        // sender != state_key
        assert_eq!(row(unclassified_leave(false), "Ann", "Bob", None), "Ann ha rimosso Bob dalla stanza");
        assert_eq!(row(unclassified_leave(false), "Ann", "Bob", Some("spam")), "Ann ha rimosso Bob dalla stanza (motivo: spam)");
        assert_eq!(row(Status::Banned, "Ann", "Bob", Some("spam")), "Ann ha bandito Bob (motivo: spam)");
        assert_eq!(row(Status::Invited, "Ann", "Bob", None), "Ann ha invitato Bob");
        // blank reason counts as none
        assert_eq!(row(Status::Joined, "Bob", "Bob", Some("  ")), "Bob è entrato/a nella stanza");
    }

    #[test]
    fn label_fallbacks() {
        let id = "@whatsapp_lid-123:example.com";
        assert_eq!(display_label(Some("Marco"), id), "Marco");
        assert_eq!(display_label(None, id), "whatsapp_lid-123");
        assert_eq!(display_label(Some(""), id), "whatsapp_lid-123");
        assert_eq!(display_label(Some(id), id), "whatsapp_lid-123");
        assert_eq!(display_label(Some("Unknown user"), id), "whatsapp_lid-123");
        assert_eq!(display_label(Some("+39••••••56"), id), "+39••••••56");
        assert_eq!(display_label(None, "@+39••••••56:example.com"), "+39••••••56");
        assert_eq!(display_label(None, ""), "Un utente");
        assert_eq!(display_label(None, "@:x.test"), "@:x.test");
        let long = display_label(None, "@whatsapp_lid-12345678901234567890:example.com");
        assert_eq!(long.chars().count(), MAX_ID_LABEL);
        assert!(long.ends_with('…') && !long.contains("example.com"));
    }

    #[test]
    fn wording_follows_element() {
        let t = |s: Status| status_text(&s, "meska", "Marco");
        assert_eq!(t(Status::Created), "meska ha creato questa stanza.");
        assert_eq!(t(Status::CreatedDirect), "meska ha creato questo MD.");
        assert_eq!(t(Status::Invited), "meska ha invitato Marco");
        assert_eq!(t(Status::Joined), "Marco è entrato/a nella stanza");
        assert_eq!(t(Status::Left), "Marco è uscito/a dalla stanza");
        assert_eq!(t(Status::InvitationRejected), "Marco ha rifiutato l'invito");
        assert_eq!(t(Status::Kicked), "meska ha rimosso Marco dalla stanza");
        assert_eq!(t(Status::Banned), "meska ha bandito Marco");
        assert_eq!(t(Status::RoomName("Casa".into())), "meska ha cambiato il nome della stanza in «Casa»");
        assert_eq!(t(Status::RoomTopic(String::new())), "meska ha rimosso l'argomento della stanza");
        assert_eq!(t(Status::DisplayName { old: Some("Mar".into()), new: Some("Marco".into()) }), "Mar ha cambiato il nome in Marco");
        assert_eq!(t(Status::MemberAvatar), "Marco ha cambiato l'avatar");
    }

    #[test]
    fn wording_never_leaves_a_hole() {
        // Every variant, with an ID as name (no display name) and with empty room name/topic: no empty slots.
        let id = "@bob:x.test";
        for s in [Status::Created, Status::CreatedDirect, Status::Invited, Status::Joined, Status::Left, Status::Banned, Status::Unbanned, Status::Kicked,
            Status::InvitationRejected, Status::InvitationRevoked, Status::RoomName(String::new()), Status::RoomTopic(String::new()),
            Status::RoomAvatar, Status::MemberAvatar, Status::DisplayName { old: None, new: Some("N".into()) },
            Status::DisplayName { old: Some("O".into()), new: None }, Status::DisplayName { old: None, new: None }] {
            let t = status_text(&s, id, id);
            assert!(t.contains(id) && !t.contains("  ") && !t.contains("«»") && t.trim() == t, "{s:?}: {t}");
        }
        let dn = |o: &str, n: &str| status_text(&Status::DisplayName { old: Some(o.into()), new: Some(n.into()) }, "erin", "erin");
        assert_eq!(dn("", "Erin"), "erin ha impostato il nome Erin");
        assert_eq!(dn("  ", ""), "erin ha rimosso il nome visualizzato");
        assert_eq!(dn("Erin", ""), "erin ha rimosso il nome visualizzato");
        assert_eq!(status_text(&Status::DisplayName { old: Some("Mar".into()), new: None }, id, id), format!("{id} ha rimosso il nome visualizzato"));
    }
}
