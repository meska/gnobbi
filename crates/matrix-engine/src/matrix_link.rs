//! `matrix:` and `https://matrix.to/#/` links from the system: untrusted input, parsed by ruma
//! (no syntax parser of ours) behind an allowlist. The result only names a room or a user;
//! `action=join|chat` is accepted and ignored, nothing here has an effect.
use matrix_domain::has_unsafe_url_char;
use matrix_sdk::ruma::{
    MatrixToUri, MatrixUri,
    matrix_uri::{MatrixId, UriAction},
};

const MAX_LEN: usize = 2048;
const MAX_VIA: usize = 5;
const MATRIX_TO: &str = "https://matrix.to/#/";

// The types live in matrix-domain so the desktop shell can hold a link without the SDK.
pub use matrix_domain::{LinkError, MatrixLink};

pub fn parse_matrix_link(raw: &str) -> Result<MatrixLink, LinkError> {
    if raw.len() > MAX_LEN || has_unsafe_url_char(raw) {
        return Err(LinkError::Invalid);
    }
    let uri = if raw.get(..MATRIX_TO.len()) == Some(MATRIX_TO) {
        MatrixToUri::parse(raw).map(MatrixUri::from)
    } else if raw.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("matrix:")) {
        MatrixUri::parse(raw)
    } else {
        return Err(LinkError::Invalid);
    }
    .map_err(|_| LinkError::Invalid)?;
    if !matches!(uri.action(), None | Some(UriAction::Join | UriAction::Chat)) {
        return Err(LinkError::Invalid);
    }
    let mut via: Vec<String> = Vec::new();
    for server in uri.via() {
        if !via.iter().any(|v| v == server.as_str()) {
            via.push(server.to_string());
        }
    }
    if uri.via().len() > MAX_VIA {
        return Err(LinkError::Invalid);
    }
    let link = match uri.id() {
        MatrixId::Room(id) => room(id.to_string(), via, None),
        MatrixId::RoomAlias(alias) => room(alias.to_string(), via, None),
        MatrixId::Event(room_id, event) => room(room_id.to_string(), via, Some(event.to_string())),
        MatrixId::User(user) => MatrixLink::User {
            user_id: user.to_string(),
        },
        _ => return Err(LinkError::Invalid),
    };
    // ruma percent-decodes: look again at what it produced.
    let texts: Vec<&str> = match &link {
        MatrixLink::Room { target, via, event } => std::iter::once(target.as_str())
            .chain(via.iter().map(String::as_str))
            .chain(event.as_deref())
            .collect(),
        MatrixLink::User { user_id } => vec![user_id],
    };
    // Matrix ids are at most 255 bytes by spec; ruma does not enforce it for every kind.
    if texts.into_iter().any(|t| t.len() > 255 || has_unsafe_url_char(t)) {
        return Err(LinkError::Invalid);
    }
    Ok(link)
}

fn room(target: String, via: Vec<String>, event: Option<String>) -> MatrixLink {
    MatrixLink::Room { target, via, event }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room_link(target: &str, via: &[&str], event: Option<&str>) -> MatrixLink {
        MatrixLink::Room {
            target: target.into(),
            via: via.iter().map(|v| v.to_string()).collect(),
            event: event.map(Into::into),
        }
    }

    #[test]
    fn accepted_forms() {
        let user = MatrixLink::User { user_id: "@u:s.org".into() };
        for (raw, want) in [
            ("matrix:r/alias:s.org", room_link("#alias:s.org", &[], None)),
            ("MATRIX:r/alias:s.org", room_link("#alias:s.org", &[], None)),
            ("matrix:r/alias:s.org?action=join", room_link("#alias:s.org", &[], None)),
            ("matrix:roomid/id:s.org?via=a.org&via=b.org", room_link("!id:s.org", &["a.org", "b.org"], None)),
            ("matrix:roomid/id:s.org/e/ev?via=a.org", room_link("!id:s.org", &["a.org"], Some("$ev"))),
            ("matrix:roomid/id:s.org?via=a.org&via=a.org", room_link("!id:s.org", &["a.org"], None)),
            ("matrix:roomid/id:s.org?via=a&via=b&via=c&via=d&via=e", room_link("!id:s.org", &["a", "b", "c", "d", "e"], None)),
            ("matrix:u/u:s.org", user.clone()),
            ("matrix:u/u:s.org?action=chat", user.clone()),
            ("https://matrix.to/#/@u:s.org", user.clone()),
            ("https://matrix.to/#/#alias:s.org", room_link("#alias:s.org", &[], None)),
            ("https://matrix.to/#/!id:s.org?via=a.org", room_link("!id:s.org", &["a.org"], None)),
            ("https://matrix.to/#/!id:s.org/$ev?via=a.org", room_link("!id:s.org", &["a.org"], Some("$ev"))),
        ] {
            assert_eq!(parse_matrix_link(raw), Ok(want), "{raw}");
        }
        // The percent-encoded id and the matrix.to form name the same room.
        assert_eq!(
            parse_matrix_link("matrix:r/alias:s.org"),
            parse_matrix_link("https://matrix.to/#/%23alias:s.org")
        );
    }

    #[test]
    fn rejected_forms() {
        let long_id = format!("matrix:u/{}:b", "a".repeat(256));
        let too_long = format!("matrix:r/{}:b", "a".repeat(2049));
        for raw in [
            "", "matrix:", "matrix:r/", "matrix:x/a:b", "matrix://r/a:b", "matrix:r/a",
            "matrix:u/a:b?action=evil", "matrix:u/a:b?action=join&action=chat", "matrix:r/a:b?foo=1",
            "matrix:r/a:b?via=", "matrix:r/a:b?via=bad%20host",
            "matrix:r/a:b?via=a&via=b&via=c&via=d&via=e&via=f",
            "http://matrix.to/#/@a:b", "https://www.matrix.to/#/@a:b", "https://matrix.to.evil/#/@a:b",
            "https://evil.invalid/#/@a:b", "https://matrix.to/#/@a:b?via=x?y",
            "https://app.element.io/#/room/#a:b", "matrix:u/a:b\n", " matrix:u/a:b",
            "matrix:u/a%E2%80%AEb:c", "matrix:u/a%0Ab:c", "matrix:u/a%00b:c", "matrix:r/a\u{200b}:b",
            "matrix:r/a:b/../../etc/passwd", "matrix:r/$(touch NEVER):b", "javascript:alert(1)",
            "file:///etc/passwd", "--signin", "-matrix:u/a:b", &long_id, &too_long,
        ] {
            assert_eq!(parse_matrix_link(raw), Err(LinkError::Invalid), "{raw:?}");
        }
    }
}
