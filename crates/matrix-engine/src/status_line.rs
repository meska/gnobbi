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
        Status::DisplayName { old, new } => match (old, new) {
            (Some(old), Some(new)) => format!("{old} ha cambiato il nome in {new}"),
            (None, Some(new)) => format!("{subject} ha impostato il nome {new}"),
            (_, None) => format!("{subject} ha rimosso il nome visualizzato"),
        },
        Status::MemberAvatar => format!("{subject} ha cambiato l'avatar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(status_text(&Status::DisplayName { old: Some("Mar".into()), new: None }, id, id), format!("{id} ha rimosso il nome visualizzato"));
    }
}
