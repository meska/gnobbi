//! Reactions of a message as the history projects them (aggregate, who, ours, order) and the operations that add and take back
//! ours, on a disposable homeserver (Synapse or Tuwunel). Driven by the timeline_probe script
//! (`PROBE_TARGET=reactions_projection PROBE_TEST=reactions_are_projected_and_taken_back`). Two synthetic accounts only.
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, TransactionId};
use matrix_engine::{
    ViewRequests,
    history::{HistoryEvent, Reaction},
    operations::{Operation, OperationError, OperationRequest},
    receipts::ReadVisibility,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{reaction::ReactionEventContent, relation::Annotation, room::message::RoomMessageEventContent},
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

fn summary(rs: &[Reaction]) -> Vec<(String, usize, bool)> {
    rs.iter().map(|r| (r.key.clone(), r.count, r.mine)).collect()
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn reactions_are_projected_and_taken_back() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let build = || {
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
        let (a, b) = (build().await, build().await);
        for (c, who) in [(&a, "alice"), (&b, "bob")] {
            c.matrix_auth()
                .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
                .send()
                .await
                .unwrap();
        }
        b.account().set_display_name(Some("Bobby")).await.unwrap();
        let mut creation = CreationContent::new();
        creation.federate = false;
        let room_id = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![b.user_id().unwrap().to_owned()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap()
            .room_id()
            .to_owned();
        let rb = b.join_room_by_id(&room_id).await.unwrap();
        let sa = SyncService::builder(a.clone()).build().await.unwrap();
        sa.start().await;
        let started = Instant::now();
        let ra = loop {
            if let Some(r) = a.get_room(&room_id) {
                break r;
            }
            assert!(started.elapsed() < Duration::from_secs(40), "alice room");
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let message = ra.send(RoomMessageEventContent::text_plain("hello")).await.unwrap().response.event_id;
        let react = |key: &str| ReactionEventContent::new(Annotation::new(message.clone(), key.into()));
        let thumbs_by_bob = rb.send(react("\u{1f44d}")).await.unwrap().response.event_id;
        let heart_by_bob = rb.send(react("\u{2764}\u{fe0f}")).await.unwrap().response.event_id;
        let _ = thumbs_by_bob;

        let sync = RoomSync::from_client(AccountId::new(1), a.clone());
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(room_id.as_str()) };
        let mut views = ViewRequests::default();
        let hidden = ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
        // Alice's projection of the message, polled until `ready` holds (the sync is asynchronous).
        macro_rules! shown {
            ($ready:expr, $what:expr) => {{
                let ready: &dyn Fn(&[Reaction]) -> bool = &$ready;
                let started = Instant::now();
                loop {
                    let (page, _) = sync.open_history(views.start(scope.clone()).unwrap(), hidden).await.unwrap();
                    let row: Option<&HistoryEvent> = page.events.iter().find(|e| e.event_id.as_str() == message.as_str());
                    if let Some(row) = row
                        && ready(&row.reactions)
                    {
                        break row.reactions.clone();
                    }
                    assert!(started.elapsed() < Duration::from_secs(40), "{}: {:?}", $what, row.map(|r| summary(&r.reactions)));
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            }};
        }
        let op = |tx: &str, operation| OperationRequest {
            scope: scope.clone(),
            transaction_id: TransactionId::new(tx),
            target: EventId::new(message.as_str()),
            operation,
        };
        let (thumbs, heart) = ("\u{1f44d}".to_string(), "\u{2764}\u{fe0f}".to_string());

        // Bob's two reactions: first appearance order, names, nothing ours.
        let first = shown!(|r| r.len() == 2, "bob's reactions");
        assert_eq!(summary(&first), [(thumbs.clone(), 1, false), (heart.clone(), 1, false)]);
        assert_eq!(first[0].senders.len(), 1);
        assert_eq!(first[0].senders[0].user_id, b.user_id().unwrap().as_str());
        assert_eq!(first[0].senders[0].name.as_deref(), Some("Bobby"), "the display name comes with the sender");
        assert!(first.iter().all(|r| !r.mine));

        // Ours joins Bob's: counted, flagged, and the order is kept.
        sync.operate(&op("rx-add", Operation::React { key: thumbs.clone() })).await.expect("react");
        let both = shown!(|r| r.first().is_some_and(|r| r.count == 2 && r.mine && !r.pending), "alice joins");
        assert_eq!(summary(&both), [(thumbs.clone(), 2, true), (heart.clone(), 1, false)]);
        // Choosing it again with the row showing ours is refused by React; the way back is Unreact.
        assert_eq!(sync.operate(&op("rx-dup", Operation::React { key: thumbs.clone() })).await, Err(OperationError::InvalidInput));

        // Taking it back works from the projection alone: no journal of this session is involved.
        sync.operate(&op("rx-del", Operation::Unreact { key: thumbs.clone() })).await.expect("unreact");
        let back = shown!(|r| r.first().is_some_and(|r| r.count == 1 && !r.mine), "alice leaves");
        assert_eq!(summary(&back), [(thumbs.clone(), 1, false), (heart.clone(), 1, false)]);

        // A reaction made outside this session's journal (another device): taken back the same way.
        ra.send(react("\u{1f602}")).await.unwrap();
        let other_device = shown!(|r| r.iter().any(|r| r.mine), "reaction from another device");
        assert_eq!(other_device.last().map(|r| (r.key.as_str(), r.mine)), Some(("\u{1f602}", true)));
        sync.operate(&op("rx-del2", Operation::Unreact { key: "\u{1f602}".into() })).await.expect("unreact other device");
        let gone = shown!(|r| r.iter().all(|r| !r.mine), "other device reaction removed");
        assert_eq!(summary(&gone), [(thumbs.clone(), 1, false), (heart.clone(), 1, false)]);

        // One reaction per user: ours 😂 then 😮 leaves only 😮 (the engine redacts the old one); Bob's pills stay.
        sync.operate(&op("rx-a", Operation::React { key: "\u{1f602}".into() })).await.expect("react 1");
        shown!(|r| r.iter().any(|r| r.key == "\u{1f602}" && r.mine), "first of ours");
        sync.operate(&op("rx-b", Operation::React { key: "\u{1f62e}".into() })).await.expect("react 2");
        let swapped = shown!(|r| r.iter().any(|r| r.key == "\u{1f62e}" && r.mine) && r.iter().all(|r| r.key != "\u{1f602}"), "replaced");
        assert_eq!(swapped.iter().filter(|r| r.mine).count(), 1);
        // Bob's own view of Alice's reactions: only 😮.
        let page = rb
            .relations(message.clone(), matrix_engine::sdk::room::RelationsOptions {
                include_relations: matrix_engine::sdk::room::IncludeRelations::RelationsOfType(matrix_engine::sdk::ruma::events::relation::RelationType::Annotation),
                ..Default::default()
            })
            .await
            .unwrap();
        let alices: Vec<String> = page.chunk.iter().filter_map(|e| {
            let v: serde_json::Value = serde_json::from_str(e.raw().json().get()).ok()?;
            (v["sender"].as_str() == Some(a.user_id().unwrap().as_str())).then(|| v["content"]["m.relates_to"]["key"].as_str().unwrap_or_default().to_owned())
        }).collect();
        assert_eq!(alices, ["\u{1f62e}"], "Bob sees only the new reaction of Alice");
        sync.operate(&op("rx-c", Operation::Unreact { key: "\u{1f62e}".into() })).await.expect("unreact 2");
        shown!(|r| r.iter().all(|r| !r.mine), "ours gone");

        // Bob takes his heart back: the pill goes with it.
        rb.redact(&heart_by_bob, None, None).await.unwrap();
        let last = shown!(|r| r.len() == 1, "bob removes his heart");
        assert_eq!(summary(&last), [(thumbs.clone(), 1, false)]);
        let _ = sa.stop().await;
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
