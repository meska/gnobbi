//! Join by alias, join from the public directory and leave, through the engine operations and the
//! live room list, against a disposable homeserver (the room_mgmt_probe script with
//! PROBE_TEST=room_join). Two synthetic accounts: A publishes two rooms, B joins and leaves.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    RoomListDiff, RoomListEvent, bounded,
    operations::{OperationError, join_room, leave_room, search_directory},
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::{
                Visibility,
                create_room::v3::{CreationContent, Request as CreateRoomRequest},
            },
            assign,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{num::NonZeroUsize, time::Duration};

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn b_joins_by_alias_and_from_the_directory_then_leaves() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let login = |who: &'static str| {
            let (url, ca) = (config["url"].as_str().unwrap().to_owned(), ca.clone());
            let user = config[format!("{who}_mxid")].as_str().unwrap().to_owned();
            let pass = config[format!("{who}_password")].as_str().unwrap().to_owned();
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client.matrix_auth().login_username(&user, &pass).send().await.unwrap();
                client
            }
        };
        let (a, b) = (login("alice").await, login("bob").await);
        let server = b.user_id().unwrap().server_name().to_owned();
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        // Public rooms in the directory; the first has an alias, the second only an ID.
        let publish = |name: String, alias: Option<String>| {
            let a = a.clone();
            async move {
                let mut content = CreationContent::new();
                content.federate = false;
                a.create_room(assign!(CreateRoomRequest::new(), {
                    name: Some(name),
                    visibility: Visibility::Public,
                    room_alias_name: alias,
                    creation_content: Some(Raw::new(&content).unwrap()),
                }))
                .await
                .unwrap()
            }
        };
        let alias_name = format!("gnobbi-test-alias-{nanos}");
        let by_alias = publish(format!("Gnobbi alias {nanos}"), Some(alias_name.clone())).await;
        let listed_name = format!("Gnobbi directory {nanos}");
        let listed = publish(listed_name.clone(), None).await;
        let scope = |id: &str| RoomScope { account_id: AccountId::new(2), room_id: RoomId::new(id) };
        let (alias_scope, listed_scope) = (scope(by_alias.room_id().as_str()), scope(listed.room_id().as_str()));

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(AccountId::new(2), b.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let steps = async {
            let account = AccountId::new(2);
            // Bad input and a missing alias stop before any room is touched.
            assert_eq!(join_room(&b, account, "not an alias").await, Err(OperationError::InvalidInput));
            assert_eq!(join_room(&b, account, &format!("#gnobbi-test-missing-{nanos}:{server}")).await, Err(OperationError::Unavailable));

            // Join by alias: the room appears in the list as joined.
            assert_eq!(join_room(&b, account, &format!("#{alias_name}:{server}")).await.unwrap(), alias_scope);
            // Join from the directory: the hit has no alias, so the room ID is the target.
            let mut hit = None;
            for _ in 0..20 {
                hit = search_directory(&b, &listed_name).await.unwrap().into_iter().find(|d| d.room_id == listed_scope.room_id.as_str());
                if hit.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let hit = hit.expect("public room not in directory");
            assert_eq!((hit.name.as_str(), hit.alias.as_deref()), (listed_name.as_str(), None));
            assert_eq!(join_room(&b, account, hit.target()).await.unwrap(), listed_scope);

            let (mut joined_alias, mut joined_listed) = (false, false);
            while !(joined_alias && joined_listed) {
                if let Some(RoomListEvent::Diff(RoomListDiff::Upsert(s))) = rx.recv().await
                    && !s.info.invited
                {
                    joined_alias |= s.scope == alias_scope;
                    joined_listed |= s.scope == listed_scope;
                }
            }

            // Leave: the room goes from the list; the other one stays; leaving twice is refused.
            leave_room(&b, account, &alias_scope).await.expect("leave");
            loop {
                match rx.recv().await {
                    Some(RoomListEvent::Diff(RoomListDiff::Remove(r))) if r == alias_scope => break,
                    Some(RoomListEvent::Diff(RoomListDiff::Remove(r))) => assert_ne!(r, listed_scope, "the other room must stay"),
                    Some(_) => {}
                    None => panic!("engine stopped"),
                }
            }
            assert_eq!(leave_room(&b, account, &alias_scope).await, Err(OperationError::Unavailable));
            println!("[join] alias {} directory {} left ok", alias_scope.room_id.as_str(), listed_scope.room_id.as_str());
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped before the steps ended"),
            r = tokio::time::timeout(Duration::from_secs(120), steps) => r.expect("join/leave not reflected in 120 s"),
        }
    });
}
