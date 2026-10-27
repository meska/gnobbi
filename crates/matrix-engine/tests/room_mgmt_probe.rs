//! Room-management probe against a disposable homeserver (read-only
//! verification). Driven by the room_mgmt_probe script inside a private
//! namespace. Exercises the official matrix-sdk 0.19.1 / matrix-sdk-ui APIs
//! for: DM creation, room creation (public, with alias), invites (list,
//! accept, decline), join by alias, public directory search, leave, forget,
//! member list and user profile. Two synthetic accounts only; no secrets here
//! (the fixture config path comes from the environment).
#![cfg(feature = "sdk")]
use matrix_engine::{
    sdk::{
        Client, RoomMemberships, RoomState,
        reqwest::Certificate,
        room_directory_search::RoomDirectorySearch,
        ruma::{
            OwnedRoomAliasId, RoomOrAliasId,
            api::client::{
                profile::DisplayName,
                room::Visibility,
                room::create_room::v3::{
                    CreationContent, Request as CreateRoomRequest, RoomPreset,
                },
            },
            assign,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn room_management_covers_dm_invite_join_directory_leave_members_profile() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let build = |_user: &str| {
            let (url, ca) = (url.clone(), ca.clone());
            async move {
                Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap()
            }
        };
        let a = build(config["alice_mxid"].as_str().unwrap()).await;
        let b = build(config["bob_mxid"].as_str().unwrap()).await;
        a.matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        b.matrix_auth()
            .login_username(
                config["bob_mxid"].as_str().unwrap(),
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let alice = a.user_id().unwrap().to_owned();
        let bob = b.user_id().unwrap().to_owned();
        let server = alice.server_name();
        a.account()
            .set_display_name(Some("Alice Probe"))
            .await
            .unwrap();
        b.account()
            .set_display_name(Some("Bob Probe"))
            .await
            .unwrap();
        println!("[probe] logged in {alice} and {bob}, display names set");

        // Both accounts sync with the official SyncService (MSC4186 baseline).
        let service_a = SyncService::builder(a.clone()).build().await.unwrap();
        service_a.start().await;
        let service = SyncService::builder(b.clone()).build().await.unwrap();
        service.start().await;

        // DM: SDK create_dm writes m.direct via account().mark_as_dm.
        let dm = a.create_dm(&bob).await.unwrap();
        let dm_id = dm.room_id().to_owned();
        until("m.direct account data lists the DM", || {
            a.get_dm_room(&bob).map(|r| r.room_id().to_owned()) == Some(dm_id.clone())
        })
        .await;
        println!("[probe] dm created {dm_id} and returned by get_dm_room");

        // Second invited room to decline later.
        let decline = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                is_direct: false,
            }))
            .await
            .unwrap();
        println!("[probe] invite-only room created {}", decline.room_id());

        // Public room with an alias (m.federate:false, as the test rules ask).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let localpart = format!("gnobbi-test-probe-{nanos}");
        let alias: OwnedRoomAliasId = format!("#{localpart}:{server}").parse().unwrap();
        let mut creation_content = CreationContent::new();
        creation_content.federate = false;
        let public = a
            .create_room(assign!(CreateRoomRequest::new(), {
                name: Some("Gnobbi probe".to_owned()),
                visibility: Visibility::Public,
                room_alias_name: Some(localpart),
                creation_content: Some(Raw::new(&creation_content).unwrap()),
            }))
            .await
            .unwrap();
        println!(
            "[probe] public room created {} alias {alias}",
            public.room_id()
        );

        // Alias resolution through the room directory.
        let resolved = a.resolve_room_alias(&alias).await.unwrap();
        assert_eq!(resolved.room_id, public.room_id());
        println!("[probe] alias resolves to {}", resolved.room_id);

        // B sees both invites once sync is running.
        let decline_id = decline.room_id().to_owned();
        let (dm_id, decline_id) = (&dm_id, &decline_id);
        until("bob sees both invites", || {
            let rooms = b.invited_rooms();
            rooms.iter().any(|r| r.room_id() == dm_id)
                && rooms.iter().any(|r| r.room_id() == decline_id)
        })
        .await;
        println!("[probe] bob invited to {dm_id} and {decline_id}");

        // Accept the DM, decline the other invitation (leave).
        b.get_room(dm_id).unwrap().join().await.unwrap();
        b.get_room(decline_id).unwrap().leave().await.unwrap();
        until("invites leave the invited list", || {
            let rooms = b.invited_rooms();
            rooms
                .iter()
                .all(|r| r.room_id() != dm_id && r.room_id() != decline_id)
        })
        .await;
        // Declining via leave() on an invited room also forgets it (SDK
        // room/mod.rs: Room::leave "If any room was in Invited state,
        // it'll also be forgotten").
        assert!(b.get_room(decline_id).is_none());
        println!("[probe] dm joined, invitation declined (room forgotten)");

        // Join the public room by alias.
        let target = RoomOrAliasId::parse(alias.as_str()).unwrap();
        let joined = b.join_room_by_id_or_alias(&target, &[]).await.unwrap();
        assert_eq!(joined.room_id(), public.room_id());
        println!("[probe] joined by alias into {}", joined.room_id());

        // Public directory search finds it.
        let mut search = RoomDirectorySearch::new(b.clone());
        search.search(None, 100, None).await.unwrap();
        let (results, _stream) = search.results();
        let hit = results.iter().find(|d| d.room_id == public.room_id());
        assert!(hit.is_some(), "public room not in directory");
        assert_eq!(hit.unwrap().alias.as_ref(), Some(&alias));
        println!("[probe] directory search lists {}", public.room_id());

        // Member list and room-scoped member profile.
        let members = joined.members(RoomMemberships::JOIN).await.unwrap();
        let ids: Vec<_> = members.iter().map(|m| m.user_id().to_owned()).collect();
        assert!(
            ids.contains(&alice) && ids.contains(&bob),
            "members: {ids:?}"
        );
        let member = joined
            .get_member(&alice)
            .await
            .unwrap()
            .expect("alice member");
        assert_eq!(member.display_name(), Some("Alice Probe"));
        println!("[probe] members {ids:?}, alice display name ok");

        // Global profile of the other user.
        let profile = a.account().fetch_user_profile_of(&bob).await.unwrap();
        let name = profile.get_static::<DisplayName>().unwrap().unwrap();
        assert_eq!(name, "Bob Probe");
        println!("[probe] profile of bob: {name}");

        // Leave and forget.
        joined.leave().await.unwrap();
        until("public room becomes left", || {
            b.get_room(public.room_id()).map(|r| r.state()) == Some(RoomState::Left)
        })
        .await;
        b.get_room(public.room_id())
            .unwrap()
            .forget()
            .await
            .unwrap();
        println!("[probe] left and forgot {}", public.room_id());

        service_a.stop().await;
        service.stop().await;
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
