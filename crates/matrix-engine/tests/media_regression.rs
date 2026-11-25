//! Regression for broken and valid media against a disposable Synapse or Tuwunel, driven by
//! the media_regression script (plain and encrypted room, one synthetic account).
//! Valid m.video opens; empty body, missing mxc and wrong type are one typed error each;
//! a truncated file is delivered byte for byte (the player's exit then reports it);
//! an Ogg/Opus voice note keeps its `.ogg` suffix (MIME parameters) and decodes (~14 s).
#![cfg(all(feature = "sdk", unix))]
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::MediaError,
    sdk::{
        Client,
        attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseVideoInfo},
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::Request as CreateRoomRequest,
            events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    video_file::{download_audio, download_video},
};

const CAP: usize = 1024 * 1024;

#[test]
#[ignore = "requires the media_regression script"]
fn broken_and_valid_media_on_a_real_homeserver() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(var("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let read = |key: &str| std::fs::read(config[key].as_str().unwrap()).unwrap();
    let (video, ogg) = (read("video_mp4"), read("voice_ogg"));
    let truncated = video[..video.len() / 3].to_vec();
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
        let server_name = config["alice_mxid"].as_str().unwrap().split_once(':').unwrap().1;
        let account = AccountId::new(1);
        for encrypted in [false, true] {
            let mut request = CreateRoomRequest::new();
            request.name = Some(format!("gnobbi-test-media-{encrypted}"));
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
            let mut live = ViewRequests::new();
            let tag = live.start(scope).unwrap();
            let video_event = |data: Vec<u8>| {
                let room = room.clone();
                async move {
                    let info = AttachmentInfo::Video(BaseVideoInfo::default());
                    let sent = room
                        .send_attachment(
                            "v.mp4",
                            &"video/mp4".parse().unwrap(),
                            data,
                            AttachmentConfig::new().info(info),
                        )
                        .await;
                    sent.map(|sent| EventId::new(sent.event_id.as_str()))
                }
            };
            let mut dl = async |event: &EventId| {
                download_video(&client, account, &tag, event, &profile, CAP, |_| {}).await
            };

            // Valid control: arrives intact, mp4 suffix, 0600.
            let ok = video_event(video.clone()).await.unwrap();
            let file = dl(&ok).await.unwrap();
            assert_eq!(std::fs::read(file.path()).unwrap(), video, "enc={encrypted}");
            assert_eq!(file.path().extension().unwrap(), "mp4");
            drop(file);

            // Empty body: one typed failure, nothing left on disk.
            // Synapse refuses an empty upload (400 Content-Length), so the case exists on Tuwunel only.
            match video_event(Vec::new()).await {
                Ok(empty) => {
                    let result = dl(&empty).await;
                    assert!(matches!(result, Err(MediaError::Failed)), "enc={encrypted}: {result:?}");
                }
                Err(error) => println!("EMPTY_UPLOAD_REFUSED_BY_SERVER {error}"),
            }

            // Truncated: delivered as received; the broken file is the player's to report.
            let cut = video_event(truncated.clone()).await.unwrap();
            let file = dl(&cut).await.unwrap();
            assert_eq!(std::fs::read(file.path()).unwrap(), truncated);
            drop(file);

            // mxc that the server does not know: Failed, not a hang or a panic.
            let missing = room
                .send_raw(
                    "m.room.message",
                    serde_json::json!({"msgtype": "m.video", "body": "gone.mp4",
                        "url": format!("mxc://{server_name}/doesNotExist{encrypted}"),
                        "info": {"mimetype": "video/mp4", "size": 26000}}),
                )
                .await
                .unwrap();
            let result = dl(&EventId::new(missing.response.event_id.as_str())).await;
            assert!(matches!(result, Err(MediaError::Failed)), "enc={encrypted}: {result:?}");

            // Wrong type: m.text is not media.
            let text = room
                .send_raw(
                    "m.room.message",
                    serde_json::json!({"msgtype": "m.text", "body": "ciao"}),
                )
                .await
                .unwrap();
            let result = dl(&EventId::new(text.response.event_id.as_str())).await;
            assert!(matches!(result, Err(MediaError::UnsupportedType)), "{result:?}");

            // Voice note Ogg/Opus: MIME with parameters must still give `.ogg`.
            let audio_event = |data: Vec<u8>| {
                let room = room.clone();
                async move {
                    let info = AttachmentInfo::Audio(BaseAudioInfo::default());
                    let sent = room
                        .send_attachment(
                            "voice.ogg",
                            &"audio/ogg; codecs=opus".parse().unwrap(),
                            data,
                            AttachmentConfig::new().info(info),
                        )
                        .await;
                    sent.map(|sent| EventId::new(sent.event_id.as_str()))
                }
            };
            let voice = audio_event(ogg.clone()).await.unwrap();
            let file = dl(&voice).await.unwrap();
            assert_eq!(file.path().extension().unwrap(), "ogg", "enc={encrypted}");
            assert_eq!(std::fs::read(file.path()).unwrap(), ogg);
            drop(file);
            // AUDIO-DECODE-BEGIN
            let audio = download_audio(&client, account, &tag, &voice, CAP).await.unwrap();
            let seconds = audio.samples.len() as f32 / audio.sample_rate as f32;
            assert!((13.5..14.5).contains(&seconds), "enc={encrypted}: {seconds}s");
            assert_eq!(audio.waveform.len(), 100);
            let bad = audio_event(ogg[..100].to_vec()).await.unwrap();
            let result = download_audio(&client, account, &tag, &bad, CAP).await;
            assert!(result.is_err(), "truncated voice message must fail: {result:?}");
            if let Ok(none) = audio_event(Vec::new()).await {
                let result = download_audio(&client, account, &tag, &none, CAP).await;
                assert!(result.is_err(), "empty voice message must fail: {result:?}");
            }
            // AUDIO-DECODE-END
            assert_eq!(
                std::fs::read_dir(profile.join("video-tmp")).unwrap().count(),
                0,
                "no plaintext left"
            );
            println!("MEDIA_REGRESSION_OK enc={encrypted}");
        }
    });
}
