//! `video_file::save_attachment` against a disposable homeserver, driven by
//! the save_attachment script (PROBE_SERVER=synapse|tuwunel): the ORIGINAL of an image, a video, an
//! audio and a file lands byte-identical at the chosen path (never the thumbnail), over the cap is
//! refused, a failed write can be retried, in a plain and in an encrypted room.
//! One synthetic account; the fixture config path comes from the environment.
#![cfg(all(feature = "sdk", unix))]
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::MediaError,
    sdk::{
        Client,
        attachment::{AttachmentConfig, AttachmentInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo, Thumbnail},
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::Request as CreateRoomRequest,
            events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    video_file::save_attachment,
};

const CAP: usize = 64 * 1024;

#[test]
#[ignore = "requires the save_attachment script"]
fn save_writes_the_original_and_refuses_over_the_cap_plain_and_encrypted() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(var("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let small = std::fs::read(config["small_mp4"].as_str().unwrap()).unwrap();
    let big = std::fs::read(config["big_mp4"].as_str().unwrap()).unwrap();
    assert!(small.len() < CAP && big.len() > CAP);
    let out = std::path::PathBuf::from(var("HOME")).join("saved");
    std::fs::create_dir(&out).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
        let client = Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let account = AccountId::new(1);
        let thumb = vec![7u8; 40]; // must never be what gets saved
        for encrypted in [false, true] {
            let mut request = CreateRoomRequest::new();
            request.name = Some(format!("gnobbi-test-save-{encrypted}"));
            if encrypted {
                let state = InitialStateEvent::with_empty_state_key(
                    RoomEncryptionEventContent::with_recommended_defaults(),
                );
                request.initial_state = vec![Raw::new(&state).unwrap().cast_unchecked()];
            }
            let room = client.create_room(request).await.unwrap();
            let scope = RoomScope {
                account_id: account,
                room_id: RoomId::new(room.room_id().as_str()),
            };
            let mut live = ViewRequests::new(); // dropping it would cancel the tag
            let tag = live.start(scope.clone()).unwrap();
            let send = |name: &str, mime: &str, data: Vec<u8>, info: AttachmentInfo| {
                let (room, thumb, name, mime) = (room.clone(), thumb.clone(), name.to_owned(), mime.to_owned());
                async move {
                    let mut config = AttachmentConfig::new().info(info);
                    if name.ends_with(".png") || name.ends_with(".mp4") {
                        config = config.thumbnail(Some(Thumbnail {
                            data: thumb,
                            content_type: "image/png".parse().unwrap(),
                            height: 1u32.into(),
                            width: 1u32.into(),
                            size: 40u32.into(),
                        }));
                    }
                    EventId::new(
                        room.send_attachment(&name, &mime.parse().unwrap(), data, config)
                            .await
                            .unwrap()
                            .event_id
                            .as_str(),
                    )
                }
            };
            let png = [1u8, 2, 3, 4, 5, 6, 7, 8].repeat(500);
            let doc = b"%PDF-1.4 originale".repeat(50);
            let audio = vec![3u8; 2500];
            let cases = [
                ("foto.png", "image/png", png.clone(), AttachmentInfo::Image(BaseImageInfo::default())),
                ("clip.mp4", "video/mp4", small.clone(), AttachmentInfo::Video(BaseVideoInfo::default())),
                ("nota.ogg", "audio/ogg", audio.clone(), AttachmentInfo::Audio(Default::default())),
                ("rel.pdf", "application/pdf", doc.clone(), AttachmentInfo::File(BaseFileInfo::default())),
            ];
            for (name, mime, data, info) in cases {
                let event = send(name, mime, data.clone(), info).await;
                let dest = out.join(format!("{encrypted}-{name}"));
                save_attachment(&client, account, &tag, &event, &dest, CAP).await.unwrap();
                let got = std::fs::read(&dest).unwrap();
                assert_eq!(got, data, "{name} encrypted={encrypted}: bytes differ from the original");
                assert_ne!(got, thumb, "{name}: saved the thumbnail");
                assert!(!out.join(format!("{encrypted}-{name}.part")).exists());
            }
            // No declared size: the receive-time cap stops it; nothing is written.
            let over = send("grande.bin", "application/octet-stream", big.clone(), AttachmentInfo::File(BaseFileInfo::default())).await;
            let dest = out.join(format!("{encrypted}-grande.bin"));
            let result = save_attachment(&client, account, &tag, &over, &dest, CAP).await;
            assert!(matches!(result, Err(MediaError::TooLarge)), "encrypted={encrypted}: {result:?}");
            assert!(!dest.exists() && !out.join(format!("{encrypted}-grande.bin.part")).exists());
            // Declared size over the cap: refused before any media request.
            let declared = send("dich.bin", "application/octet-stream", big.clone(), AttachmentInfo::File(BaseFileInfo { size: Some((big.len() as u32).into()) })).await;
            let result = save_attachment(&client, account, &tag, &declared, &out.join("dich.bin"), CAP).await;
            assert!(matches!(result, Err(MediaError::TooLarge)), "{result:?}");
            // Failure (missing directory), then the same event retried on a good path succeeds.
            let doc_event = send("retry.pdf", "application/pdf", doc.clone(), AttachmentInfo::File(BaseFileInfo::default())).await;
            let bad = out.join("non/esiste/retry.pdf");
            let result = save_attachment(&client, account, &tag, &doc_event, &bad, CAP).await;
            assert!(matches!(result, Err(MediaError::Failed)), "{result:?}");
            let good = out.join(format!("{encrypted}-retry.pdf"));
            save_attachment(&client, account, &tag, &doc_event, &good, CAP).await.unwrap();
            assert_eq!(std::fs::read(&good).unwrap(), doc);
            // A text event is not an attachment.
            let text = room.send(matrix_engine::sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("ciao")).await.unwrap();
            let result = save_attachment(&client, account, &tag, &EventId::new(text.response.event_id.as_str()), &out.join("t"), CAP).await;
            assert!(matches!(result, Err(MediaError::UnsupportedType)), "{result:?}");
            // Cancelled: nothing written.
            let mut views = ViewRequests::new();
            let ctag = views.start(scope).unwrap();
            views.cancel();
            let dest = out.join(format!("{encrypted}-annullato.pdf"));
            let result = save_attachment(&client, account, &ctag, &doc_event, &dest, CAP).await;
            assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
            assert!(!dest.exists());
        }
    });
}
