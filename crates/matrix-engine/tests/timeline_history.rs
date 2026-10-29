//! History rows from the SDK Timeline on a disposable homeserver (Synapse or Tuwunel):
//! edit, redaction and reaction folded in by the SDK, and the cursor chain down to the room start.
//! Driven by the timeline_probe script (`PROBE_TARGET=timeline_history PROBE_TEST=history_rows_come_from_the_timeline`).
//! Two synthetic accounts only; no secrets here (fixture config via env).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    receipts::ReadVisibility,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            OwnedEventId,
            api::client::room::create_room::v3::{
                CreationContent, Request as CreateRoomRequest, RoomPreset,
            },
            assign,
            events::{
                reaction::ReactionEventContent,
                relation::Annotation,
                room::message::{ReplacementMetadata, RoomMessageEventContent},
            },
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

#[test]
#[ignore = "requires the timeline_probe script"]
fn history_rows_come_from_the_timeline() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
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
                .login_username(
                    config[format!("{who}_mxid")].as_str().unwrap(),
                    config[format!("{who}_password")].as_str().unwrap(),
                )
                .send()
                .await
                .unwrap();
        }
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
        b.join_room_by_id(&room_id).await.unwrap();
        let (sa, sb) = (
            SyncService::builder(a.clone()).build().await.unwrap(),
            SyncService::builder(b.clone()).build().await.unwrap(),
        );
        sa.start().await;
        sb.start().await;
        let started = Instant::now();
        let ra = loop {
            if let Some(r) = a.get_room(&room_id) {
                break r;
            }
            assert!(started.elapsed() < Duration::from_secs(40), "alice room");
            tokio::time::sleep(Duration::from_millis(300)).await;
        };

        // 30 plain messages, then one that is edited, one redacted, one reacted to, and the last.
        let mut sent: Vec<OwnedEventId> = Vec::new();
        for text in (0..30).map(|i| format!("old {i:02}")).chain([
            "m1".into(),
            "m2".into(),
            "m3".into(),
            "last".into(),
        ]) {
            sent.push(
                ra.send(RoomMessageEventContent::text_plain(text))
                    .await
                    .unwrap()
                    .response
                    .event_id,
            );
        }
        let (m1, m2, m3) = (sent[30].clone(), sent[31].clone(), sent[32].clone());
        ra.send(
            RoomMessageEventContent::text_plain("m1 edited")
                .make_replacement(ReplacementMetadata::new(m1.clone(), None)),
        )
        .await
        .unwrap();
        ra.redact(&m2, None, None).await.unwrap();
        ra.send(ReactionEventContent::new(Annotation::new(
            m3.clone(),
            "\u{1f44d}".into(),
        )))
        .await
        .unwrap();

        let sync = RoomSync::from_client(AccountId::new(2), b.clone());
        let scope = RoomScope {
            account_id: AccountId::new(2),
            room_id: RoomId::new(room_id.as_str()),
        };
        let mut views = ViewRequests::default();
        let hidden = ReadVisibility {
            window_focused: false,
            room_foreground: false,
            bottom_gap_px: 0.0,
            public_receipts: true,
        };
        // Bob's sync is asynchronous: open until the edit, redaction and reaction have arrived.
        let started = Instant::now();
        let (opened, reactions) = loop {
            let (page, reactions) = sync
                .open_history(views.start(scope.clone()).unwrap(), hidden)
                .await
                .unwrap();
            let bodies: Vec<_> = page.events.iter().map(|e| e.body.as_str()).collect();
            if bodies.contains(&"m1 edited")
                && bodies.contains(&"[Messaggio eliminato]")
                && !reactions.is_empty()
            {
                break (page, reactions);
            }
            assert!(
                started.elapsed() < Duration::from_secs(40),
                "bob rows: {bodies:?}"
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        assert_eq!(opened.events.last().unwrap().body, "last");
        assert!(
            !opened
                .events
                .iter()
                .any(|e| e.body == "m1" || e.body == "m2")
        );
        assert_eq!(reactions.len(), 1);
        assert_eq!(
            (reactions[0].key.as_str(), reactions[0].count),
            ("\u{1f44d}", 1)
        );
        assert_eq!(reactions[0].target.as_str(), m3.as_str());
        assert!(
            opened
                .events
                .iter()
                .all(|e| !e.body.contains("m1 edited") || e.event_id.as_str() == m1.as_str()),
            "the edit is not a row of its own"
        );

        // Scroll up to the start of the room by the cursor chain, a page at a time.
        let mut older = Vec::new();
        let mut cursor = opened.next.clone();
        let mut guard = 0;
        while let Some(before) = cursor {
            let page = sync
                .history_page(views.start(scope.clone()).unwrap(), Some(before))
                .await
                .unwrap();
            assert!(page.events.len() <= 25);
            older.splice(0..0, page.events);
            cursor = page.next;
            guard += 1;
            assert!(guard < 10, "the cursor chain must end");
        }
        let all: Vec<_> = older.iter().chain(opened.events.iter()).collect();
        let ids: std::collections::BTreeSet<_> = all.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids.len(), all.len(), "no duplicates across pages");
        let expect: Vec<&str> = sent.iter().map(|e| e.as_str()).collect();
        assert_eq!(
            all.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(),
            expect,
            "every message once, oldest first"
        );
        println!(
            "[probe] {} rows in {} pages down to the start",
            all.len(),
            guard + 1
        );
        let _ = (sa.stop().await, sb.stop().await);
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
