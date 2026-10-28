//! The app's own send path (`sending::send_text` with an attachment) against a
//! disposable homeserver, driven by the attachment_send script: user A sends an
//! image with caption and a file without caption, user B receives them, in a
//! plain and in an encrypted room. Two synthetic accounts only; the fixture
//! config path comes from the environment.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    sdk::{
        Client,
        event_cache::RoomEventCache,
        media::{MediaFormat, MediaRequestParameters},
        reqwest::Certificate,
        ruma::{
            api::client::room::{
                Visibility,
                create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            },
            assign,
            events::room::MediaSource,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Attachment, SendRequest, Voice, fresh_transaction_id, send_text},
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(90), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Raw JSON of a decrypted (or plain) room message sent by `sender` whose
/// msgtype is `msgtype`, polled from the accumulated room event cache.
async fn wait_message(events: &RoomEventCache, sender: &str, msgtype: &str) -> serde_json::Value {
    let start = Instant::now();
    loop {
        if let Ok(list) = events.events().await
            && let Some(found) = list.iter().find_map(|event| {
                let value: serde_json::Value =
                    serde_json::from_str(event.raw().json().get()).ok()?;
                let content = &value["content"];
                (value["sender"] == sender
                    && value["type"] == "m.room.message"
                    && content["msgtype"] == msgtype)
                    .then_some(value)
            })
        {
            return found;
        }
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "timeout: m.{msgtype} from {sender}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn send(
    client: &Client,
    room: &matrix_engine::sdk::Room,
    body: &str,
    name: &str,
    data: &[u8],
    voice: Option<Voice>,
) {
    let request = SendRequest {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new(room.room_id().as_str()),
        },
        transaction_id: fresh_transaction_id(),
        body: body.to_owned(),
        attachment: Some(Attachment {
            filename: name.to_owned(),
            data: data.to_vec(),
            voice,
        }),
    };
    // An encrypted room may reject the first attempts until the Megolm session reaches B.
    let start = Instant::now();
    while let Err(error) = send_text(client, AccountId::new(1), &request).await {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "send kept failing: {error:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[test]
#[ignore = "requires the attachment_send script"]
fn app_sends_image_file_and_voice_note_plain_and_encrypted() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let image = std::fs::read(config["image_png"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let login = |user: &'static str| {
            let (url, ca, config) = (url.clone(), ca.clone(), config.clone());
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client
                    .matrix_auth()
                    .login_username(
                        config[format!("{user}_mxid")].as_str().unwrap(),
                        config[format!("{user}_password")].as_str().unwrap(),
                    )
                    .send()
                    .await
                    .unwrap();
                client
            }
        };
        let a = login("alice").await;
        let b = login("bob").await;
        let alice = a.user_id().unwrap().to_owned();
        let bob = b.user_id().unwrap().to_owned();
        let service_a = SyncService::builder(a.clone()).build().await.unwrap();
        service_a.start().await;
        let service_b = SyncService::builder(b.clone()).build().await.unwrap();
        service_b.start().await;

        for encrypted in [false, true] {
            let mut creation = CreationContent::new();
            creation.federate = false;
            let room = a
                .create_room(assign!(CreateRoomRequest::new(), {
                    name: Some(format!("Gnobbi allegati {}", if encrypted { "cifrata" } else { "chiara" })),
                    visibility: Visibility::Private,
                    preset: Some(RoomPreset::PrivateChat),
                    invite: vec![bob.clone()],
                    creation_content: Some(Raw::new(&creation).unwrap()),
                }))
                .await
                .unwrap();
            let id = room.room_id().to_owned();
            until("bob sees the invite", || b.invited_rooms().iter().any(|r| r.room_id() == id)).await;
            b.get_room(&id).unwrap().join().await.unwrap();
            let (room_a, room_b) = (a.get_room(&id).unwrap(), b.get_room(&id).unwrap());
            if encrypted {
                room_a.enable_encryption().await.unwrap();
                let start = Instant::now();
                loop {
                    let (x, y) = (room_a.latest_encryption_state().await, room_b.latest_encryption_state().await);
                    if matches!((&x, &y), (Ok(x), Ok(y)) if x.is_encrypted() && y.is_encrypted()) {
                        break;
                    }
                    assert!(start.elapsed() < Duration::from_secs(90), "timeout: room encryption state");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
            let (events, _keep) = room_b.event_cache().await.unwrap();
            // Send only once B's sync has delivered the room: an event racing the first limited sync can be lost to the cache.
            let start = Instant::now();
            while events.events().await.map_or(true, |list| list.is_empty()) {
                assert!(start.elapsed() < Duration::from_secs(60), "timeout: room reaches bob");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }

            send(&a, &room_a, "didascalia immagine", "foto.png", &image, None).await;
            send(&a, &room_a, "", "nota.txt", b"contenuto del file\n", None).await;
            let wav = b"RIFF....WAVEfmt synthetic".to_vec();
            let voice = Voice { duration_ms: 2500, waveform: vec![0.0, 0.5, 1.0] };
            send(&a, &room_a, "", "nota-vocale.wav", &wav, Some(voice)).await;

            let img = wait_message(&events, alice.as_str(), "m.image").await["content"].clone();
            assert_eq!(img["body"], "didascalia immagine");
            assert_eq!(img["filename"], "foto.png");
            assert_eq!(img["info"]["mimetype"], "image/png");
            assert_eq!((img["info"]["w"].as_u64(), img["info"]["h"].as_u64()), (Some(32), Some(24)), "{img}");
            assert_eq!(img.get("file").is_some(), encrypted, "media is encrypted exactly in encrypted rooms");
            let file = wait_message(&events, alice.as_str(), "m.file").await["content"].clone();
            assert_eq!(file["body"], "nota.txt");
            assert_eq!(file["info"]["mimetype"], "text/plain");

            let note = wait_message(&events, alice.as_str(), "m.audio").await["content"].clone();
            assert!(note.get("org.matrix.msc3245.voice").is_some(), "voice marker: {note}");
            assert_eq!(note["org.matrix.msc1767.audio"]["duration"], 2500, "{note}");
            assert_eq!(note["org.matrix.msc1767.audio"]["waveform"].as_array().map(Vec::len), Some(3), "{note}");
            assert_eq!(note["info"]["duration"], 2500, "{note}");
            assert_eq!(note.get("file").is_some(), encrypted);
            let voice_source: MediaSource = serde_json::from_value(note).unwrap();
            let voice_bytes = b
                .media()
                .get_media_content(&MediaRequestParameters { source: voice_source, format: MediaFormat::File }, false)
                .await
                .unwrap();
            assert_eq!(voice_bytes, wav, "B downloads the exact voice note");

            let source: MediaSource = serde_json::from_value(img).unwrap();
            let bytes = b
                .media()
                .get_media_content(&MediaRequestParameters { source, format: MediaFormat::File }, false)
                .await
                .unwrap();
            assert_eq!(bytes, image, "B downloads (and decrypts) the exact image A sent");
            println!("[send] encrypted={encrypted}: B received image+caption, file and voice note");
        }
        service_a.stop().await;
        service_b.stop().await;
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
