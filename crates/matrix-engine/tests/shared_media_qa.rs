//! The «Media, file e link» tabs (`SharedMedia`) on a disposable Synapse or Tuwunel, one synthetic account, a plain and an
//! encrypted room. Each room holds an album of 3 pictures, a PDF and a message with a link, then 50 text messages on top:
//! the tabs must show 3 media, 1 file and 1 link, whatever the paging has to read to get there. Scenarios print
//! `QA <name> PASS|FAIL detail`.
//! Driver: PROBE_TARGET=shared_media_qa PROBE_TEST=shared_media_scenarios PROBE_SERVER=synapse|tuwunel
//! PROBE_FEATURES=sdk python3 the timeline_probe script
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    operations::{MoreHits, SharedKind, SharedState, SharedTab},
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            OwnedRoomId,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{InitialStateEvent, room::{encryption::RoomEncryptionEventContent, message::RoomMessageEventContent}},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Album, Attachment, SendRequest, fresh_transaction_id, send_text},
    shared_media::SharedMedia,
};
use std::time::{Duration, Instant};

const PASS: &str = "synthetic-passphrase-alice-1";

fn report(failed: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        failed.push(name.to_owned());
    }
}

async fn until<T>(what: &str, secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(secs), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 6, image::Rgba([200, 30, 30, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn send(client: &Client, room_id: &OwnedRoomId, body: &str, attachment: Option<Attachment>) {
    let req = SendRequest {
        scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(room_id.as_str()) },
        transaction_id: fresh_transaction_id(),
        body: body.to_owned(),
        attachment,
        poll: None,
        mentions: Vec::new(),
    };
    for _ in 0..5 {
        if send_text(client, AccountId::new(1), &req).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    panic!("send failed");
}

/// A text message; a loaded disposable server may time a send out, so it is tried a few times (a repeated text changes nothing here).
async fn text(room: &matrix_engine::sdk::Room, body: String) {
    for _ in 0..5 {
        if room.send(RoomMessageEventContent::text_plain(body.clone())).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    panic!("text send failed");
}

/// Everything the tab shows once the room's history has been read to its start: passes are asked for until nothing is left.
async fn whole_tab(client: &Client, room_id: &OwnedRoomId, tab: SharedTab) -> (SharedState, usize) {
    let mut shared = SharedMedia::open(client.clone(), room_id.clone(), tab);
    let mut rx = shared.subscribe();
    let mut passes = 1;
    loop {
        let state = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let s = rx.borrow_and_update().clone();
                if s.more != MoreHits::Loading {
                    return s;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("a pass never ended");
        if state.more == MoreHits::None || passes >= 20 {
            return (state, passes);
        }
        assert_eq!(state.more, MoreHits::Available, "a pass failed");
        shared.more();
        passes += 1;
    }
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn shared_media_scenarios() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let store = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores/alice-shared");
    std::fs::create_dir_all(&store).unwrap();
    let mut failed = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let ca = Certificate::from_pem(&std::fs::read(config["ca"].as_str().unwrap()).unwrap()).unwrap();
        let a = Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .add_root_certificates(vec![ca])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .sqlite_store(&store, Some(PASS))
            .build()
            .await
            .unwrap();
        a.event_cache().subscribe().unwrap();
        a.matrix_auth().login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap()).send().await.unwrap();
        // Both rooms exist before the sync starts, as in the other scenarios: a busy sync starves the server of time for createRoom.
        let mut ids = Vec::new();
        for encrypted in [false, true] {
            let mut creation = CreationContent::new();
            creation.federate = false;
            let mut req = assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                creation_content: Some(Raw::new(&creation).unwrap()),
            });
            if encrypted {
                req.initial_state = vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()];
            }
            ids.push(a.create_room(req).await.unwrap().room_id().to_owned());
        }
        let sa = SyncService::builder(a.clone()).build().await.unwrap();
        sa.start().await;
        for (encrypted, id) in [false, true].into_iter().zip(ids) {
            let kind = if encrypted { "e2ee" } else { "plain" };
            let room = until("room", 40, async || a.get_room(&id)).await;
            for index in 0..3u8 {
                let album = Some(Album { id: "qa-album".into(), index, count: 3 });
                send(&a, &id, "", Some(Attachment { filename: format!("foto{index}.png"), data: png(), album, ..Default::default() })).await;
            }
            send(&a, &id, "", Some(Attachment { filename: "contratto.pdf".into(), data: b"%PDF-1.4 synthetic".to_vec(), ..Default::default() })).await;
            text(&room, "guarda https://example.org/pagina?x=1 per i dettagli".into()).await;
            for i in 0..50 {
                text(&room, format!("solo testo {i}")).await;
            }
            // The newest event must be on the server before the history is read.
            tokio::time::sleep(Duration::from_secs(2)).await;

            let (media, passes) = whole_tab(&a, &id, SharedTab::Media).await;
            let kinds: Vec<_> = media.items.iter().map(|i| i.kind).collect();
            report(&mut failed, &format!("{kind}.media-tab-has-the-album"), kinds == [SharedKind::Image; 3] && media.items.iter().all(|i| !i.sender_name.is_empty()), format!("n={} passes={passes}", media.items.len()));
            let names: Vec<_> = media.items.iter().map(|i| i.name.as_str()).collect();
            report(&mut failed, &format!("{kind}.media-newest-first"), names == ["foto2.png", "foto1.png", "foto0.png"], format!("{names:?}"));
            let (files, _) = whole_tab(&a, &id, SharedTab::Files).await;
            report(&mut failed, &format!("{kind}.files-tab-has-the-pdf"), files.items.len() == 1 && files.items[0].name == "contratto.pdf" && files.items[0].size.is_some(), format!("n={}", files.items.len()));
            let (links, _) = whole_tab(&a, &id, SharedTab::Links).await;
            report(&mut failed, &format!("{kind}.links-tab-has-the-link"), links.items.len() == 1 && links.items[0].url.as_deref() == Some("https://example.org/pagina?x=1"), format!("n={}", links.items.len()));
            // The pictures of the tab are the real events: the media pipeline reads them by ID.
            let mut real = !media.items.is_empty();
            for i in &media.items {
                let event = room.event(&matrix_engine::sdk::ruma::EventId::parse(&i.event_id).unwrap(), None).await.unwrap();
                real &= event.raw().get_field::<String>("type").unwrap().as_deref() == Some("m.room.message");
            }
            report(&mut failed, &format!("{kind}.items-are-the-events"), real, String::new());
        }
        let _ = sa.stop().await;
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}
