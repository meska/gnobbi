//! `video_file::download_video` against a disposable homeserver, driven by
//! the video_download script: a small synthetic MP4 arrives intact in a private
//! temp file, one over the cap is refused, in a plain and in an encrypted room.
//! One synthetic account; the fixture config path comes from the environment.
#![cfg(all(feature = "sdk", unix))]
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::MediaError,
    sdk::{
        Client,
        attachment::{AttachmentConfig, AttachmentInfo, BaseVideoInfo},
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::Request as CreateRoomRequest,
            events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    video_file::download_video,
};
use std::os::unix::fs::PermissionsExt;

const CAP: usize = 64 * 1024;

#[test]
#[ignore = "requires the video_download script"]
fn small_video_lands_in_private_file_and_oversized_is_refused() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(var("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let small = std::fs::read(config["small_mp4"].as_str().unwrap()).unwrap();
    let big = std::fs::read(config["big_mp4"].as_str().unwrap()).unwrap();
    assert!(small.len() < CAP && big.len() > CAP);
    let profile = std::path::PathBuf::from(var("HOME")).join("profile");
    std::fs::create_dir(&profile).unwrap();
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
        for encrypted in [false, true] {
            let mut request = CreateRoomRequest::new();
            request.name = Some(format!("gnobbi-test-video-{encrypted}"));
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
            let send = |data: Vec<u8>, size: Option<u64>| {
                let room = room.clone();
                async move {
                    let info = AttachmentInfo::Video(BaseVideoInfo {
                        size: size.map(|s| s.try_into().unwrap()),
                        ..Default::default()
                    });
                    let config = AttachmentConfig::new().info(info);
                    EventId::new(
                        room.send_attachment("v.mp4", &"video/mp4".parse().unwrap(), data, config)
                            .await
                            .unwrap()
                            .event_id
                            .as_str(),
                    )
                }
            };
            let mut live = ViewRequests::new(); // dropping it would cancel the tag
            let tag = live.start(scope.clone()).unwrap();

            let ok = send(small.clone(), None).await;
            let mut seen = Vec::new();
            let file = download_video(&client, account, &tag, &ok, &profile, CAP, |n| seen.push(n))
                .await
                .unwrap();
            assert_eq!(
                std::fs::read(file.path()).unwrap(),
                small,
                "encrypted={encrypted}"
            );
            let mode =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                (mode(file.path()), mode(file.path().parent().unwrap())),
                (0o600, 0o700)
            );
            assert_eq!(seen.last(), Some(&small.len()));
            let path = file.path().to_owned();
            drop(file);
            assert!(!path.exists());

            // No declared size: the receive-time cap stops it.
            let over = send(big.clone(), None).await;
            let result = download_video(&client, account, &tag, &over, &profile, CAP, |_| {}).await;
            assert!(
                matches!(result, Err(MediaError::TooLarge)),
                "encrypted={encrypted}: {result:?}"
            );
            // Declared size over the cap: refused before any media request.
            let declared = send(big.clone(), Some(big.len() as u64)).await;
            let result =
                download_video(&client, account, &tag, &declared, &profile, CAP, |_| {}).await;
            assert!(
                matches!(result, Err(MediaError::TooLarge)),
                "encrypted={encrypted}: {result:?}"
            );

            let mut views = ViewRequests::new();
            let tag = views.start(scope).unwrap();
            views.cancel();
            let result = download_video(&client, account, &tag, &ok, &profile, CAP, |_| {}).await;
            assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
            assert_eq!(
                std::fs::read_dir(profile.join("video-tmp"))
                    .unwrap()
                    .count(),
                0,
                "no plaintext left"
            );
        }
    });
}
