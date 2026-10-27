//! Timeline probe (P281 spike): matrix-sdk-ui Timeline on a disposable homeserver. Read side
//! (messages, edit, reaction, reply, redaction, backwards pagination, UTD-free) and the action
//! side (`toggle_reaction`, `edit`, `send_reply`, `redact`). Driven by the timeline_probe script.
//! Two synthetic accounts only; no secrets here (fixture config via env).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_engine::{
    sdk::{
        Client,
        room::edit::EditedContent,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::room::message::{RoomMessageEventContent, RoomMessageEventContentWithoutRelation},
            serde::Raw,
            OwnedEventId,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{
        sync_service::SyncService,
        timeline::{EventTimelineItem, RoomExt, Timeline, TimelineEventItemId},
    },
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn item(t: &Timeline, id: &OwnedEventId) -> Option<EventTimelineItem> {
    t.items()
        .await
        .iter()
        .filter_map(|i| i.as_event())
        .find(|e| e.event_id() == Some(id))
        .cloned()
}

async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn body(e: &EventTimelineItem) -> Option<String> {
    e.content().as_message().map(|m| m.body().to_owned())
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn timeline_reads_and_acts_on_disposable_server() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let kind = config["kind"].as_str().unwrap().to_owned();
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
                .login_username(
                    config[format!("{who}_mxid")].as_str().unwrap(),
                    config[format!("{who}_password")].as_str().unwrap(),
                )
                .send()
                .await
                .unwrap();
        }
        let bob = b.user_id().unwrap().to_owned();
        let mut creation = CreationContent::new();
        creation.federate = false;
        let created = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap();
        let room_id = created.room_id().to_owned();
        b.join_room_by_id(&room_id).await.unwrap();
        let (sa, sb) = (
            SyncService::builder(a.clone()).build().await.unwrap(),
            SyncService::builder(b.clone()).build().await.unwrap(),
        );
        sa.start().await;
        sb.start().await;
        let ra = until("alice room", async || a.get_room(&room_id)).await;
        let rb = until("bob room", async || b.get_room(&room_id)).await;
        let ta = ra.timeline().await.unwrap();
        let tb = rb.timeline().await.unwrap();
        println!("[probe] {kind}: both timelines built");

        // Alice: 30 plain messages, to be paged later by Bob.
        for i in 0..30 {
            ra.send(RoomMessageEventContent::text_plain(format!("old {i:02}"))).await.unwrap();
        }
        // Live: Alice sends m1, Bob sees it on his Timeline.
        let m1 = ra.send(RoomMessageEventContent::text_plain("m1 hello")).await.unwrap().response.event_id;
        until("bob sees m1", async || item(&tb, &m1).await).await;
        println!("[probe] live message arrives on the other Timeline");

        // Edit by Alice through Timeline::edit; Bob sees the edited body in the same item.
        let id1 = TimelineEventItemId::EventId(m1.clone());
        until("alice m1 editable", async || item(&ta, &m1).await.filter(|e| e.is_editable())).await;
        ta.edit(&id1, EditedContent::RoomMessage(RoomMessageEventContentWithoutRelation::text_plain("m1 edited")))
            .await
            .unwrap();
        let e = until("bob sees edit", async || {
            item(&tb, &m1).await.filter(|e| body(e).as_deref() == Some("m1 edited"))
        })
        .await;
        assert!(e.content().as_message().unwrap().is_edited());
        println!("[probe] edit aggregated into the original item (is_edited)");

        // Reaction by Bob through toggle_reaction; Alice sees one reaction key with Bob.
        assert!(tb.toggle_reaction(&id1, "👍").await.unwrap());
        let e = until("alice sees reaction", async || {
            item(&ta, &m1).await.filter(|e| e.content().reactions().is_some_and(|r| r.contains_key("👍")))
        })
        .await;
        assert!(e.content().reactions().unwrap()["👍"].contains_key(&bob));
        // Toggling again removes it (redaction of the reaction).
        assert!(!tb.toggle_reaction(&id1, "👍").await.unwrap());
        until("alice sees reaction gone", async || {
            item(&ta, &m1).await.filter(|e| e.content().reactions().is_none_or(|r| !r.contains_key("👍")))
        })
        .await;
        println!("[probe] reaction aggregated and removed by toggle");

        // Reply by Bob; Alice sees in_reply_to the event id.
        tb.send_reply(RoomMessageEventContentWithoutRelation::text_plain("re: m1"), m1.clone()).await.unwrap();
        let reply = until("alice sees reply", async || {
            ta.items().await.iter().filter_map(|i| i.as_event()).find(|e| body(e).is_some_and(|b| b.contains("re: m1"))).cloned()
        })
        .await;
        assert_eq!(reply.content().in_reply_to().map(|r| r.event_id.clone()), Some(m1.clone()));
        println!("[probe] reply carries in_reply_to");

        // Redaction by Alice; Bob sees the message gone from the text.
        ta.redact(&id1, None).await.unwrap();
        until("bob sees redaction", async || item(&tb, &m1).await.filter(|e| e.content().as_message().is_none())).await;
        println!("[probe] redaction visible as non-message content");

        // Backwards pagination on a fresh Timeline of Bob's: 30 old messages + the rest.
        let fresh = rb.timeline().await.unwrap();
        let mut total = 0;
        for _ in 0..6 {
            let start = fresh.paginate_backwards(10).await.unwrap();
            total = fresh.items().await.iter().filter(|i| i.as_event().is_some()).count();
            println!("[probe] paginate_backwards(10) reached_start={start} events={total}");
            if start { break; }
        }
        assert!(total >= 30, "expected 30 old messages, got {total}");
        let _ = (sa.stop().await, sb.stop().await);
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
