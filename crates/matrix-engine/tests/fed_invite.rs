//! Federated invite seen through the live engine path (`sliding::run`), as the INVITED user on
//! server B. Driven by the fed_invite_probe script (FED_CONFIG: B's url/ca/user/password + room_id);
//! A (another homeserver, federated) has already invited B. Complements sync_migration_qa S6 (one server).
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListDiff, RoomListEvent, bounded,
    room_sync::RoomSync,
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    sliding,
};
use std::{num::NonZeroUsize, time::Duration};

#[test]
#[ignore = "requires the fed_invite_probe script"]
fn federated_invite_is_listed_and_can_be_accepted_or_rejected() {
    let c: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FED_CONFIG").unwrap()).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(c["url"].as_str().unwrap())
            .add_root_certificates(vec![Certificate::from_pem(&std::fs::read(c["ca"].as_str().unwrap()).unwrap()).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(c["user"].as_str().unwrap(), c["password"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        let (accept_id, reject_id) = (c["room_id"].as_str().unwrap(), c["reject_room_id"].as_str().unwrap());
        let account = AccountId::new(9);
        let scope = |id: &str| matrix_domain::RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id) };
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(account, client.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let steps = async {
            // Both invitations arrive as rows with their stripped-state name.
            let (mut seen_accept, mut seen_reject) = (false, false);
            while !(seen_accept && seen_reject) {
                if let Some(RoomListEvent::Diff(RoomListDiff::Upsert(s))) = rx.recv().await {
                    assert!(s.info.invited || (s.scope.room_id.as_str() != accept_id && s.scope.room_id.as_str() != reject_id));
                    seen_accept |= s.scope.room_id.as_str() == accept_id;
                    seen_reject |= s.scope.room_id.as_str() == reject_id;
                }
            }
            // Accept: the SDK joins through the inviting server; the room comes back as a joined row.
            matrix_engine::operations::answer_invite(&client, account, &scope(accept_id), true).await.expect("accept");
            // Reject: the SDK leaves; the room disappears from the list.
            matrix_engine::operations::answer_invite(&client, account, &scope(reject_id), false).await.expect("reject");
            let (mut joined, mut removed) = (false, false);
            while !(joined && removed) {
                match rx.recv().await {
                    Some(RoomListEvent::Diff(RoomListDiff::Upsert(s))) if s.scope.room_id.as_str() == accept_id && !s.info.invited => joined = true,
                    Some(RoomListEvent::Diff(RoomListDiff::Remove(r))) if r.room_id.as_str() == reject_id => removed = true,
                    Some(_) => {}
                    None => panic!("engine stopped"),
                }
            }
            // Answering again is not an open invitation any more.
            assert!(matrix_engine::operations::answer_invite(&client, account, &scope(accept_id), true).await.is_err());
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped before the answers"),
            r = tokio::time::timeout(Duration::from_secs(90), steps) => r.expect("accept/reject not reflected in 90 s"),
        }
    });
}
#[test]
#[ignore = "requires the fed_invite_probe script"]
fn federated_invite_is_listed_as_invited() {
    let c: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FED_CONFIG").unwrap()).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(c["url"].as_str().unwrap())
            .add_root_certificates(vec![Certificate::from_pem(&std::fs::read(c["ca"].as_str().unwrap()).unwrap()).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(c["user"].as_str().unwrap(), c["password"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        let want = c["room_id"].as_str().unwrap();
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(AccountId::new(9), client);
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let found = tokio::select! {
            _ = &mut run => None,
            r = tokio::time::timeout(Duration::from_secs(40), async {
                while let Some(ev) = rx.recv().await {
                    if let RoomListEvent::Diff(RoomListDiff::Upsert(s)) = ev {
                        eprintln!("ENGINE upsert id={} name={:?} invited={}", s.scope.room_id.as_str(), s.name, s.info.invited);
                        if s.scope.room_id.as_str() == want { return Some(s); }
                    }
                }
                None
            }) => r.ok().flatten(),
        };
        let s = found.expect("engine never emitted the federated invite");
        assert!(s.info.invited, "federated invite not marked invited");
        assert_eq!(s.name, "gnobbi-test-fed-invite", "invite name from stripped state");
    });
}
