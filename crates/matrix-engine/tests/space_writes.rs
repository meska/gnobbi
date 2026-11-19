//! Space writes against a disposable homeserver (driven by the room_mgmt_probe script with PROBE_TEST=space_writes):
//! private/public space with and without avatar, add/remove a room (`m.space.child`), and a member without the
//! power level (no edit right, `Forbidden`). The result is read back from the server state, not from the SDK store.
//! PROBE_TEST=space_move: a space moved under another, reordered with `order`, taken out, and refused without the right.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomScope};
use matrix_engine::{
    operations::{OperationError, SpaceAvatar, add_to_space, can_edit_space, create_room, create_space, place_in_space, remove_from_space},
    sdk::{
        Client,
        config::SyncSettings,
        reqwest::Certificate,
        ruma::{
            OwnedRoomId,
            api::client::{room::Visibility, state::get_state_event_for_key},
            events::StateEventType,
        },
    },
};

const ACCOUNT: AccountId = AccountId::new(1);

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .build()
        .await
        .unwrap();
    let user = config[format!("{who}_mxid")].as_str().unwrap();
    let password = config[format!("{who}_password")].as_str().unwrap();
    client.matrix_auth().login_username(user, password).send().await.unwrap();
    client
}

async fn sync(client: &Client) {
    client.sync_once(SyncSettings::default().timeout(std::time::Duration::from_millis(500))).await.unwrap();
}

fn id(scope: &RoomScope) -> OwnedRoomId {
    OwnedRoomId::try_from(scope.room_id.as_str()).unwrap()
}

/// Content of one state event read from the server; `None` when the server has none.
async fn state(client: &Client, room: &RoomScope, kind: StateEventType, key: &str) -> Option<serde_json::Value> {
    let request = get_state_event_for_key::v3::Request::new(id(room), kind, key.to_owned());
    let response = client.send(request).await.ok()?;
    Some(serde_json::from_str(response.event_or_content.get()).unwrap())
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn spaces_are_created_and_rooms_added_and_removed_as_the_server_sees_them() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (login(&config, "alice").await, login(&config, "bob").await);
        let png = include_bytes!("fixtures/checker.png").to_vec();
        let avatar = SpaceAvatar { data: png, mime: "image/png".into() };

        let private = create_space(&a, ACCOUNT, "Privato", None, false, None).await.unwrap();
        assert!(private.notices.is_empty(), "{:?}", private.notices);
        let public = create_space(&a, ACCOUNT, "Pubblico", Some("tema"), true, Some(&avatar)).await.unwrap();
        assert!(public.notices.is_empty(), "{:?}", public.notices);
        println!("[probe] spaces created");

        for (space, name, listed, has_avatar) in [(&private.scope, "Privato", false, false), (&public.scope, "Pubblico", true, true)] {
            let create = state(&a, space, StateEventType::RoomCreate, "").await.unwrap();
            assert_eq!(create["type"], "m.space", "{name}: not a space");
            assert_eq!(state(&a, space, StateEventType::RoomName, "").await.unwrap()["name"], name);
            assert!(state(&a, space, StateEventType::RoomEncryption, "").await.is_none(), "{name}: spaces are not encrypted");
            let visibility = a.get_room(&id(space)).unwrap().privacy_settings().get_room_visibility().await.unwrap();
            assert_eq!(visibility == Visibility::Public, listed, "{name}: directory listing");
            let url = state(&a, space, StateEventType::RoomAvatar, "").await.and_then(|c| c["url"].as_str().map(str::to_owned));
            assert_eq!(url.is_some(), has_avatar, "{name}: avatar {url:?}");
        }
        assert_eq!(state(&a, &public.scope, StateEventType::RoomTopic, "").await.unwrap()["topic"], "tema");
        println!("[probe] state of both spaces as asked (type, name, listing, avatar, no encryption)");

        let room = create_room(&a, ACCOUNT, "Stanza", None, false).await.unwrap();
        sync(&a).await;
        add_to_space(&a, ACCOUNT, &room, &private.scope).await.unwrap();
        let child = state(&a, &private.scope, StateEventType::SpaceChild, id(&room).as_str()).await.unwrap();
        assert!(child["via"].as_array().is_some_and(|v| !v.is_empty()), "m.space.child without via: {child}");
        println!("[probe] added: m.space.child {child}");
        add_to_space(&a, ACCOUNT, &room, &private.scope).await.unwrap();
        sync(&a).await; // the SDK removes only what its synced state lists as a child; the app syncs all the time
        remove_from_space(&a, ACCOUNT, &room, &private.scope).await.unwrap();
        let child = state(&a, &private.scope, StateEventType::SpaceChild, id(&room).as_str()).await.unwrap();
        assert!(child.get("via").is_none(), "child still listed: {child}");
        println!("[probe] removed: m.space.child {child}");

        // Bob is a plain member of the private space: no edit right, the server refuses his write.
        a.get_room(&id(&private.scope)).unwrap().invite_user_by_id(b.user_id().unwrap()).await.unwrap();
        sync(&b).await;
        b.get_room(&id(&private.scope)).unwrap().join().await.unwrap();
        sync(&b).await;
        sync(&a).await;
        assert!(can_edit_space(&a, ACCOUNT, &private.scope).await.unwrap(), "alice must be able to edit");
        assert!(!can_edit_space(&b, ACCOUNT, &private.scope).await.unwrap(), "bob must not be able to edit");
        let bobs = create_room(&b, ACCOUNT, "Di Bob", None, false).await.unwrap();
        sync(&b).await;
        let refused = add_to_space(&b, ACCOUNT, &bobs, &private.scope).await;
        assert_eq!(refused, Err(OperationError::Forbidden));
        assert!(state(&a, &private.scope, StateEventType::SpaceChild, id(&bobs).as_str()).await.is_none(), "bob's write got through");
        println!("[probe] bob: can_edit false, add refused with Forbidden, nothing written");
    });
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn a_space_moves_under_another_with_an_order_as_the_server_sees_it() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (login(&config, "alice").await, login(&config, "bob").await);
        let mut spaces = Vec::new();
        for name in ["Vecchio", "Nuovo", "X", "Y"] {
            spaces.push(create_space(&a, ACCOUNT, name, None, false, None).await.unwrap().scope);
        }
        let [old, new, x, y] = &spaces[..] else { unreachable!() };
        sync(&a).await;
        add_to_space(&a, ACCOUNT, x, old).await.unwrap();
        sync(&a).await;
        async fn link(client: &Client, space: &RoomScope, room: &RoomScope) -> Option<serde_json::Value> {
            state(client, space, StateEventType::SpaceChild, id(room).as_str()).await
        }
        let child = |space, room| link(&a, space, room);
        let listed = |c: &Option<serde_json::Value>| c.as_ref().is_some_and(|c| c["via"].as_array().is_some_and(|v| !v.is_empty()));
        let order = |c: &Option<serde_json::Value>| c.as_ref().and_then(|c| c["order"].as_str().map(str::to_owned)).expect("order");

        // Vecchio > X becomes Nuovo > X.
        place_in_space(&a, ACCOUNT, x, Some(old), Some(new), None).await.unwrap();
        let moved = child(new, x).await;
        assert!(listed(&moved), "X not under the new parent: {moved:?}");
        assert!(!listed(&child(old, x).await), "X still under the old parent");
        let parent = state(&a, x, StateEventType::SpaceParent, id(new).as_str()).await.expect("m.space.parent");
        assert!(parent["canonical"] == true && parent["via"].as_array().is_some_and(|v| !v.is_empty()), "{parent}");
        assert!(!listed(&state(&a, x, StateEventType::SpaceParent, id(old).as_str()).await), "old m.space.parent left");
        println!("[probe] moved: child {} parent {parent}", moved.clone().unwrap());
        sync(&a).await;

        // Y lands before X, then X is put back before Y: the keys sort that way and the links stay.
        place_in_space(&a, ACCOUNT, y, None, Some(new), Some(x)).await.unwrap();
        let (kx, ky) = (order(&child(new, x).await), order(&child(new, y).await));
        assert!(ky < kx, "Y {ky:?} must sort before X {kx:?}");
        sync(&a).await;
        place_in_space(&a, ACCOUNT, x, Some(new), Some(new), Some(y)).await.unwrap();
        let (cx, cy) = (child(new, x).await, child(new, y).await);
        assert!(listed(&cx) && listed(&cy) && order(&cx) < order(&cy), "X {cx:?} must sort before Y {cy:?}");
        println!("[probe] ordered: Y before X ({ky:?} < {kx:?}), then X before Y ({:?} < {:?})", order(&cx), order(&cy));
        sync(&a).await;

        // Out to the top level: only the old link goes.
        place_in_space(&a, ACCOUNT, x, Some(new), None, None).await.unwrap();
        assert!(!listed(&child(new, x).await), "X still under Nuovo");
        assert!(listed(&child(new, y).await), "Y must stay");
        println!("[probe] taken out to the top level");

        // Bob is a plain member of Nuovo: his space is refused there and nothing is written.
        a.get_room(&id(new)).unwrap().invite_user_by_id(b.user_id().unwrap()).await.unwrap();
        sync(&b).await;
        b.get_room(&id(new)).unwrap().join().await.unwrap();
        let bobs = create_space(&b, ACCOUNT, "Di Bob", None, false, None).await.unwrap().scope;
        sync(&b).await;
        assert_eq!(place_in_space(&b, ACCOUNT, &bobs, None, Some(new), None).await, Err(OperationError::Forbidden));
        assert!(child(new, &bobs).await.is_none(), "bob's write got through");
        println!("[probe] bob: move refused with Forbidden, nothing written");
    });
}
