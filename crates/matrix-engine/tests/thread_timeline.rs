//! Thread adapter on a disposable homeserver, two synthetic accounts only (the timeline_probe script
//! with PROBE_TARGET=thread_timeline PROBE_FEATURES=threads).
#![cfg(feature = "threads")]
#![recursion_limit = "256"]
use matrix_engine::{
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::room::message::{Relation, RoomMessageEventContent},
            events::relation::Thread,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    thread_timeline,
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn thread_timeline_lists_root_and_replies() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let build = || async {
            Client::builder()
                .homeserver_url(url.clone())
                .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                .build()
                .await
                .unwrap()
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
        let until = async |f: &dyn Fn() -> Option<matrix_engine::sdk::Room>| {
            let start = Instant::now();
            loop {
                if let Some(r) = f() {
                    return r;
                }
                assert!(start.elapsed() < Duration::from_secs(40), "timeout: room");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        };
        let ra = until(&|| a.get_room(&room_id)).await;
        let rb = until(&|| b.get_room(&room_id)).await;

        let root = ra.send(RoomMessageEventContent::text_plain("root")).await.unwrap().response.event_id;
        let mut reply = RoomMessageEventContent::text_plain("in thread");
        reply.relates_to = Some(Relation::Thread(Thread::plain(root.clone(), root.clone())));
        rb.send(reply).await.unwrap();
        // A message outside the thread must not appear in it.
        ra.send(RoomMessageEventContent::text_plain("elsewhere")).await.unwrap();

        let timeline = thread_timeline::open(&ra, root.clone()).await.unwrap();
        let start = Instant::now();
        let _bodies = loop {
            // Pagination is the portable read path: a server may not push thread events by sync.
            thread_timeline::load_older(&timeline, 20).await.unwrap();
            let bodies: Vec<_> =
                thread_timeline::events(&timeline).await.into_iter().filter_map(|e| e.body).collect();
            if bodies.contains(&"in thread".to_owned()) {
                break bodies;
            }
            assert!(start.elapsed() < Duration::from_secs(40), "timeout: thread reply, got {}", bodies.len());
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        // The thread opens on its latest events; paging back reaches the root.
        assert!(thread_timeline::load_older(&timeline, 20).await.unwrap(), "root not reached");
        let bodies: Vec<_> =
            thread_timeline::events(&timeline).await.into_iter().filter_map(|e| e.body).collect();
        assert!(bodies.contains(&"root".to_owned()) && !bodies.contains(&"elsewhere".to_owned()), "{}", bodies.len());
        println!("[probe] thread timeline: root + reply, main-timeline message excluded");
        let _ = (sa.stop().await, sb.stop().await);
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
