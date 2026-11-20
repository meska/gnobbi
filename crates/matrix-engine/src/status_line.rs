//! Data of the status rows (join, invite, room rename…). The SDK timeline decides what happened;
//! the UI words it in the active language, when it draws. Names are display names, falling back
//! to the user ID.

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
/// No name and no ID gives an empty string. A name equal to the ID, or the SDK's «Unknown user» placeholder, counts as no name.
pub fn display_label(name: Option<&str>, id: &str) -> String {
    let (id, name) = (id.trim(), name.map(str::trim).unwrap_or_default());
    if !name.is_empty() && !name.eq_ignore_ascii_case(id) && !name.eq_ignore_ascii_case("unknown user") {
        return name.to_owned();
    }
    let local = id.trim_start_matches('@').split(':').next().unwrap_or_default();
    let pick = if local.is_empty() { id } else { local };
    if pick.is_empty() {
        return String::new(); // the UI words «someone»
    }
    if pick.chars().count() <= MAX_ID_LABEL {
        return pick.to_owned();
    }
    format!("{}…", pick.chars().take(MAX_ID_LABEL - 1).collect::<String>())
}

/// What a status row says, without any wording: `actor` did it to `subject` (the same person for
/// self-service changes), `reason` is the one given on the membership event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub status: Status,
    /// Name of the actor; empty when there is none to show.
    pub actor: String,
    pub subject: String,
    pub reason: Option<String>,
}

/// A leave the SDK could not classify (`MembershipChange::None`, e.g. a backfilled event without
/// `prev_content`): the sender leaving is «Left», anyone else removing them is «Kicked».
pub fn unclassified_leave(sender_is_subject: bool) -> Status {
    if sender_is_subject { Status::Left } else { Status::Kicked }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(display_label(None, ""), "");
        assert_eq!(display_label(None, "@:x.test"), "@:x.test");
        let long = display_label(None, "@whatsapp_lid-12345678901234567890:example.com");
        assert_eq!(long.chars().count(), MAX_ID_LABEL);
        assert!(long.ends_with('…') && !long.contains("example.com"));
    }
}
