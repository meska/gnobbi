use super::*;
use crate::search::SearchError::*;
use matrix_sdk::ruma::{
    OwnedRoomOrAliasId, ServerName,
    api::client::directory::get_public_rooms_filtered::v3::Response,
    directory::PublicRoomsChunkInit,
    owned_room_id,
};

#[test]
fn server_names_follow_the_matrix_grammar() {
    for ok in ["matrix.org", " matrix.org ", "example.org:8448", "198.51.100.5", "198.51.100.5:8448", "[::1]", "[2001:db8::1]:8448"] {
        assert!(parse_server(ok).is_ok(), "{ok}");
    }
    for bad in ["", "   ", "a b", "@bob:matrix.org", "#a:matrix.org", "http://matrix.org", "matrix.org/x", "matrix.org:", "matrix.org:99999", "::1", "[::1"] {
        assert_eq!(parse_server(bad), Err(InvalidServer), "{bad}");
    }
}

#[test]
fn request_carries_term_limit_since_and_server() {
    let r = directory_request(" rust ", Some("remote.org:8448"), None, Some("tok".into()), 20, None).unwrap();
    assert_eq!(r.server.as_deref().map(|s| s.as_str()), Some("remote.org:8448"));
    assert_eq!(r.filter.generic_search_term.as_deref(), Some("rust"));
    assert_eq!((r.limit.map(u64::from), r.since.as_deref()), (Some(20u64), Some("tok")));
}

#[test]
fn local_search_sends_no_server_and_no_empty_term() {
    let local = ServerName::parse("home.org").unwrap();
    for server in [None, Some("home.org")] {
        let r = directory_request("  ", server, Some(&local), None, 20, None).unwrap();
        assert!(r.server.is_none() && r.filter.generic_search_term.is_none() && r.since.is_none());
    }
    assert_eq!(directory_request("a", Some("not a server"), None, None, 20, None).err(), Some(InvalidServer));
}

fn chunk(id: &str) -> matrix_sdk::ruma::directory::PublicRoomsChunk {
    PublicRoomsChunkInit { num_joined_members: 7u32.into(), room_id: matrix_sdk::ruma::RoomId::parse(id).unwrap(), world_readable: true, guest_can_join: false }.into()
}

#[test]
fn page_maps_every_field_and_keeps_next_batch() {
    let mut full = chunk("!a:remote.org");
    full.name = Some("Rust".into());
    full.canonical_alias = Some(matrix_sdk::ruma::owned_room_alias_id!("#rust:remote.org"));
    full.topic = Some("Cose".into());
    full.avatar_url = Some("mxc://remote.org/abc".into());
    let mut bare = chunk("!b:remote.org");
    bare.topic = Some(" ".into());
    let mut response = Response::new();
    response.chunk = vec![full, bare];
    response.next_batch = Some("p2".into());
    let page = directory_page(response, "remote.org");
    assert_eq!(page.next_batch.as_deref(), Some("p2"));
    let (a, b) = (&page.hits[0], &page.hits[1]);
    assert_eq!((a.name.as_str(), a.alias.as_deref(), a.topic.as_deref(), a.members), ("Rust", Some("#rust:remote.org"), Some("Cose"), 7));
    assert_eq!((a.avatar_url.as_deref(), a.world_readable, a.guest_can_join), (Some("mxc://remote.org/abc"), true, false));
    assert_eq!((a.join_rule.as_str(), a.origin.as_str(), a.target()), ("public", "remote.org", "#rust:remote.org"));
    // No name, no alias: the room ID is the name; blank topic is no topic.
    assert_eq!((b.name.as_str(), b.topic.as_deref(), b.target()), ("!b:remote.org", None, "!b:remote.org"));
}

#[test]
fn last_page_has_no_next_batch() {
    let mut response = Response::new();
    response.next_batch = Some(String::new());
    assert_eq!(directory_page(response, "x.org").next_batch, None);
    assert_eq!(directory_page(Response::new(), "x.org").next_batch, None);
}

#[test]
fn errors_are_told_apart() {
    assert_eq!(directory_error(None, None, false), Network);
    assert_eq!(directory_error(Some(400), Some(&ErrorKind::InvalidParam), false), InvalidServer);
    assert_eq!(directory_error(Some(403), Some(&ErrorKind::Forbidden), false), Refused);
    assert_eq!(directory_error(Some(404), None, false), Refused);
    assert_eq!(directory_error(Some(404), Some(&ErrorKind::Unrecognized), false), Refused);
    assert_eq!(directory_error(Some(502), None, false), Unreachable);
    assert_eq!(directory_error(Some(500), Some(&ErrorKind::Unknown), false), Unreachable);
    // Our server answered but the body was unreadable (2xx not decodable, non-Matrix error body): not «Connessione non riuscita».
    assert_eq!(directory_error(None, None, true), Unreachable);
    assert_eq!(directory_error(Some(429), Some(&ErrorKind::Unknown), true), Unreachable);
}

#[test]
fn via_is_the_given_servers_else_the_room_id_server() {
    let id = OwnedRoomOrAliasId::from(owned_room_id!("!r:remote.org"));
    let alias = OwnedRoomOrAliasId::try_from("#a:remote.org").unwrap();
    let names = |v: Vec<matrix_sdk::ruma::OwnedServerName>| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(names(join_via(&id, &["a.org".into(), "b.org:8448".into(), "a.org".into()]).unwrap()), ["a.org", "b.org:8448"]);
    assert_eq!(names(join_via(&id, &[]).unwrap()), ["remote.org"]);
    assert!(join_via(&alias, &[]).unwrap().is_empty());
    assert_eq!(names(join_via(&alias, &["via.org".into()]).unwrap()), ["via.org"]);
    assert_eq!(join_via(&id, &["bad server".into()]), Err(OperationError::InvalidInput));
}

#[test]
fn kind_sets_the_room_types_filter_and_is_checked_on_each_chunk() {
    use matrix_sdk::ruma::{directory::RoomTypeFilter, room::RoomType};
    let filter = |k| directory_request("", None, None, None, 20, k).unwrap().filter.room_types;
    assert!(filter(None).is_empty());
    assert_eq!(filter(Some(DirectoryKind::Rooms)), vec![RoomTypeFilter::Default]);
    assert_eq!(filter(Some(DirectoryKind::Spaces)), vec![RoomTypeFilter::Space]);
    // A server that ignores the filter still gets its answer cut to the kind asked.
    let (space, other) = (RoomType::Space, RoomType::from("org.example.x"));
    assert!(kind_matches(None, Some(&space)) && kind_matches(None, None));
    assert!(kind_matches(Some(DirectoryKind::Spaces), Some(&space)));
    assert!(!kind_matches(Some(DirectoryKind::Spaces), None) && !kind_matches(Some(DirectoryKind::Spaces), Some(&other)));
    assert!(kind_matches(Some(DirectoryKind::Rooms), None));
    assert!(!kind_matches(Some(DirectoryKind::Rooms), Some(&space)) && !kind_matches(Some(DirectoryKind::Rooms), Some(&other)));
}
