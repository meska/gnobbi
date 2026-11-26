//! «Scarica» (↓) from a NEW session on a disposable homeserver, driven by the save_fresh_session script
//! (PROBE_SERVER=synapse|tuwunel). Alice sends through the real app path (`sending::send_text`), Bob
//! receives; then BOTH sessions are dropped and rebuilt from their sqlite store only (an app restart:
//! no sync, no media cache) and each saves the original through `video_file::save_attachment`, the
//! function behind the click. Plain and encrypted room; image, file, two files with the SAME name.
//! The driver compares SHA-256 of every saved file with the original it generated.
//! Also: name rule (the name offered is the file's own, one safe component), failed download (media that
//! does not exist, unwritable path), the size cap, and a destination that already exists.
#![cfg(all(feature = "sdk", unix))]
#![recursion_limit = "512"]
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, TransactionId};
use matrix_engine::{
    ViewRequests,
    history::proposed_name,
    media::{MAX_SAVE_BYTES, MediaError},
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{
                InitialStateEvent,
                room::{
                    encryption::RoomEncryptionEventContent,
                    message::{MessageType, RoomMessageEventContent},
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
        store::RoomLoadSettings,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Attachment, SendRequest, send_text},
    video_file::save_attachment,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const CAP: usize = 512 * 1024;
fn account() -> AccountId {
    AccountId::new(1)
}

async fn until<T>(secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed() > Duration::from_secs(secs) {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// A client on its own sqlite store: logged in, or (restart) restored from that store.
async fn open(config: &serde_json::Value, who: &str, dir: &Path, restore: Option<matrix_engine::sdk::authentication::matrix::MatrixSession>) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .sqlite_store(dir, None)
        .build()
        .await
        .unwrap();
    match restore {
        Some(session) => client.matrix_auth().restore_session(session, RoomLoadSettings::default()).await.unwrap(),
        None => {
            client
                .matrix_auth()
                .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
                .send()
                .await
                .unwrap();
        }
    }
    client
}

fn request(room: &Room, tx: &str, name: &str, data: &[u8]) -> SendRequest {
    SendRequest {
        scope: RoomScope { account_id: account(), room_id: RoomId::new(room.room_id().as_str()) },
        transaction_id: TransactionId::new(tx),
        body: String::new(),
        attachment: Some(Attachment { filename: name.into(), data: data.to_vec(), voice: None, ..Default::default() }),
        poll: None,
        mentions: Vec::new(),
    }
}

/// What the bubble would offer in the picker: the name in the server's copy of the event.
async fn offered_name(room: &Room, id: &EventId) -> Option<String> {
    let event = room.event(&matrix_engine::sdk::ruma::EventId::parse(id.as_str()).unwrap(), None).await.ok()?;
    let content = event.raw().get_field::<RoomMessageEventContent>("content").ok()??;
    match content.msgtype {
        MessageType::File(f) => Some(proposed_name(f.filename())),
        MessageType::Image(i) => Some(proposed_name(i.filename())),
        _ => None,
    }
}

struct Item {
    label: &'static str,
    file: &'static str,
    original: Vec<u8>,
    event: EventId,
}

#[test]
#[ignore = "requires the save_fresh_session script"]
fn save_from_a_new_session_writes_the_original_plain_and_encrypted_received_and_sent() {
    // Name rule, pure: one safe component, never the sender's path.
    assert_eq!(proposed_name("../../etc/rel.pdf"), "rel.pdf");
    assert_eq!(proposed_name("C:\\x\\rel.pdf"), "rel.pdf");
    assert_eq!(proposed_name("  ..nascosto.txt"), "nascosto.txt");
    assert_eq!(proposed_name(""), "allegato");
    assert_eq!(MAX_SAVE_BYTES, 256 * 1024 * 1024, "the product cap");

    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(var("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let read = |key: &str| std::fs::read(config[key].as_str().unwrap()).unwrap();
    let (png, pdf, pdf2, big) = (read("image_png"), read("doc_pdf"), read("doc2_pdf"), read("big_bin"));
    assert!(png.len() < CAP && pdf.len() < CAP && pdf2.len() < CAP && big.len() > CAP);
    assert_ne!(pdf, pdf2);
    let server = var("PROBE_SERVER");
    let home = PathBuf::from(var("HOME"));
    let out = home.join("saved");
    std::fs::create_dir(&out).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut failures: Vec<String> = Vec::new();
    rt.block_on(async {
        for encrypted in [false, true] {
            let enc = if encrypted { "cifrata" } else { "chiara" };
            let stores = home.join(format!("stores-{enc}"));
            let a = open(&config, "alice", &stores.join("a"), None).await;
            let b = open(&config, "bob", &stores.join("b"), None).await;
            let (sa, sb) = (a.matrix_auth().session().unwrap(), b.matrix_auth().session().unwrap());
            let (svc_a, svc_b) = (SyncService::builder(a.clone()).build().await.unwrap(), SyncService::builder(b.clone()).build().await.unwrap());
            svc_a.start().await;
            svc_b.start().await;
            let mut creation = CreationContent::new();
            creation.federate = false;
            let mut create = assign!(CreateRoomRequest::new(), {
                name: Some(format!("gnobbi-save-{enc}")),
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![b.user_id().unwrap().to_owned()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            });
            if encrypted {
                create.initial_state = vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()];
            }
            let room_a = a.create_room(create).await.unwrap();
            let id = room_a.room_id().to_owned();
            until(40, async || b.join_room_by_id(&id).await.ok()).await.expect("bob joins");
            let room_b = until(40, async || b.get_room(&id)).await.expect("room on bob");
            until(40, async || (room_a.joined_user_ids().await.map(|m| m.len()).unwrap_or(0) == 2).then_some(())).await.expect("2 members");
            if encrypted {
                tokio::time::sleep(Duration::from_secs(5)).await; // the sender learns Bob's device before sending
            }
            let mut items: Vec<Item> = Vec::new();
            for (label, file, data) in [("immagine", "foto.png", &png), ("file", "relazione finale è.pdf", &pdf), ("omonimo1", "rel.pdf", &pdf), ("omonimo2", "rel.pdf", &pdf2)] {
                let req = request(&room_a, &format!("{enc}-{label}"), file, data);
                let mut sent = None;
                for _ in 0..5 {
                    if let Ok(event) = send_text(&a, account(), &req).await {
                        sent = Some(event);
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                items.push(Item { label, file, original: data.clone(), event: sent.unwrap_or_else(|| panic!("send {label} {enc}")) });
            }
            // Over the cap: sent, saved never.
            let req = request(&room_a, &format!("{enc}-grande"), "grande.bin", &big);
            let over = send_text(&a, account(), &req).await.expect("send big");
            // A file whose media does not exist on the server.
            let content = serde_json::json!({"msgtype": "m.file", "body": "rotto.bin", "filename": "rotto.bin",
                "url": format!("mxc://{}/nonexistent", config["server_name"].as_str().unwrap()), "info": {"mimetype": "application/octet-stream", "size": 1234}});
            let broken = EventId::new(room_a.send_raw("m.room.message", content).await.expect("send broken").response.event_id.as_str());
            // Bob has the room keys for all of it before the restart (the event comes back decrypted).
            for event in items.iter().map(|i| &i.event).chain([&over, &broken]) {
                let parsed = matrix_engine::sdk::ruma::EventId::parse(event.as_str()).unwrap();
                until(60, async || {
                    let ev = room_b.event(&parsed, None).await.ok()?;
                    (ev.raw().get_field::<String>("type").ok()??.as_str() == "m.room.message").then_some(())
                })
                .await
                .unwrap_or_else(|| panic!("{enc}: Bob never got {event:?} in the clear"));
            }
            svc_a.stop().await;
            svc_b.stop().await;
            drop((svc_a, svc_b, room_a, room_b, a, b));
            tokio::time::sleep(Duration::from_secs(1)).await;

            // The new sessions: the stores only. No sync, no media cache.
            let who = [("ricevuta", open(&config, "bob", &stores.join("b"), Some(sb)).await), ("inviata", open(&config, "alice", &stores.join("a"), Some(sa)).await)];
            let scope = RoomScope { account_id: account(), room_id: RoomId::new(id.as_str()) };
            let mut views = ViewRequests::new();
            let tag = views.start(scope).unwrap();
            for (origin, client) in &who {
                let room = client.get_room(&id).expect("room restored from the store");
                let mut check = |ok: bool, what: String| {
                    println!("SAVEQA {server} {enc} {origin} {} {what}", if ok { "OK" } else { "FAIL" });
                    if !ok {
                        failures.push(format!("{server} {enc} {origin}: {what}"));
                    }
                };
                for item in &items {
                    let name = offered_name(&room, &item.event).await;
                    check(name.as_deref() == Some(item.file), format!("{} name offered {name:?} == {:?}", item.label, item.file));
                    let dest = out.join(format!("{enc}-{origin}-{}", item.label));
                    // A destination that already exists and is longer than the file: replaced, not appended to.
                    if item.label == "omonimo1" {
                        std::fs::write(&dest, vec![0xAAu8; item.original.len() + 777]).unwrap();
                    }
                    let result = save_attachment(client, account(), &tag, &item.event, &dest, CAP).await;
                    let same = std::fs::read(&dest).is_ok_and(|got| got == item.original);
                    check(result.is_ok() && same, format!("{} saved {result:?} identical={same} file={}", item.label, dest.file_name().unwrap().to_string_lossy()));
                    check(!out.join(format!("{enc}-{origin}-{}.part", item.label)).exists(), format!("{} no .part left", item.label));
                }
                let dest = out.join(format!("{enc}-{origin}-grande"));
                let result = save_attachment(client, account(), &tag, &over, &dest, CAP).await;
                check(matches!(result, Err(MediaError::TooLarge)) && !dest.exists(), format!("over the cap: {result:?} written={}", dest.exists()));
                let dest = out.join(format!("{enc}-{origin}-rotto"));
                let result = save_attachment(client, account(), &tag, &broken, &dest, CAP).await;
                check(matches!(result, Err(MediaError::Failed)) && !dest.exists(), format!("download that fails: {result:?} written={}", dest.exists()));
                let dest = out.join("manca/dir/x.pdf");
                let result = save_attachment(client, account(), &tag, &items[1].event, &dest, CAP).await;
                check(matches!(result, Err(MediaError::Failed)), format!("unwritable path: {result:?}"));
                // After the failures the same click works again.
                let dest = out.join(format!("{enc}-{origin}-ritento"));
                let result = save_attachment(client, account(), &tag, &items[1].event, &dest, CAP).await;
                check(result.is_ok() && std::fs::read(&dest).is_ok_and(|got| got == items[1].original), format!("retry after failures {result:?}"));
            }
        }
    });
    assert!(failures.is_empty(), "{failures:#?}");
}
