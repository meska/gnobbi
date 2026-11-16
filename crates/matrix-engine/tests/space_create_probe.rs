//! e2e di `operations::create_space` contro un homeserver usa e getta (Synapse o Tuwunel).
//! Stato letto dal server con richieste dell'SDK (nessun HTTP proprio). Driver:
//! `PROBE_TARGET=space_create_probe PROBE_TEST=space_create_covers_private_public_avatar_and_second_session the timeline_probe script`.
//! `SPACE_EXPECT_PUBLISH=allow|deny` (opzionale) fissa l'esito atteso della pubblicazione nella directory;
//! senza, sono accettati entrambi e l'esito viene stampato (`OUTCOME ...`).
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{create_space, SpaceAvatar, SpaceNotice},
    sdk::{
        reqwest::Certificate,
        ruma::{
            api::client::{room::Visibility, state::get_state_event_for_key},
            events::StateEventType,
            RoomId,
        },
        sliding_sync::VersionBuilder,
        Client, RoomState,
    },
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

// PNG 1x1 valido.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
    0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0, 5, 0xfe, 2, 0xfe,
    0xa7, 0x35, 0x81, 0x84, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn login(config: &serde_json::Value) -> Client {
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
        .login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    SyncService::builder(client.clone()).build().await.unwrap().start().await;
    client
}

/// Contenuto di uno stato letto dal server (None se assente).
async fn state(c: &Client, room: &RoomId, ty: StateEventType) -> Option<serde_json::Value> {
    let req = get_state_event_for_key::v3::Request::new(room.to_owned(), ty, String::new());
    c.send(req).await.ok().map(|r| serde_json::from_str(r.event_or_content.get()).unwrap())
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn space_create_covers_private_public_avatar_and_second_session() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let expect = std::env::var("SPACE_EXPECT_PUBLISH").unwrap_or_default();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = login(&config).await;
        let acc = AccountId::new(1);
        let id = |s: &matrix_domain::RoomScope| RoomId::parse(s.room_id.as_str()).unwrap();
        let tag = std::process::id();

        // 1. Privato: invite, private, nessuna cifratura, create.type = m.space.
        let r = create_space(&a, acc, &format!("Privato {tag}"), Some("t"), false, None).await.unwrap();
        assert!(r.notices.is_empty(), "{:?}", r.notices);
        let (rid, room) = (id(&r.scope), a.get_room(&id(&r.scope)).unwrap());
        assert_eq!(state(&a, &rid, StateEventType::RoomCreate).await.unwrap()["type"], "m.space");
        assert_eq!(state(&a, &rid, StateEventType::RoomJoinRules).await.unwrap()["join_rule"], "invite");
        assert!(state(&a, &rid, StateEventType::RoomEncryption).await.is_none(), "space must not be encrypted");
        assert_eq!(room.privacy_settings().get_room_visibility().await.unwrap(), Visibility::Private);
        println!("CASE private: PASS");

        // 2. Pubblico: sempre creato come m.space; la directory può rifiutare.
        let r = create_space(&a, acc, &format!("Pubblico {tag}"), None, true, None).await.unwrap();
        let (rid, room) = (id(&r.scope), a.get_room(&id(&r.scope)).unwrap());
        assert_eq!(state(&a, &rid, StateEventType::RoomCreate).await.unwrap()["type"], "m.space");
        let vis = room.privacy_settings().get_room_visibility().await.unwrap();
        let denied = r.notices == [SpaceNotice::NotPublished];
        assert!(denied || r.notices.is_empty(), "{:?}", r.notices);
        assert_eq!(vis == Visibility::Public, !denied, "visibility {vis:?} vs notices {:?}", r.notices);
        match expect.as_str() {
            "allow" => assert!(!denied, "expected publication allowed"),
            "deny" => assert!(denied, "expected publication denied"),
            _ => {}
        }
        println!("OUTCOME public: {}", if denied { "creato, non pubblicato nella directory" } else { "creato e pubblicato" });
        println!("CASE public: PASS");

        // 3a. Avatar valido.
        let png = SpaceAvatar { data: PNG.to_vec(), mime: "image/png".into() };
        let r = create_space(&a, acc, &format!("Avatar {tag}"), None, false, Some(&png)).await.unwrap();
        assert!(r.notices.is_empty(), "{:?}", r.notices);
        let avatar = state(&a, &id(&r.scope), StateEventType::RoomAvatar).await.unwrap();
        assert!(avatar["url"].as_str().unwrap().starts_with("mxc://"));
        println!("CASE avatar_ok: PASS");

        // 3b. Upload rifiutato (60 MB > limite di entrambi): spazio creato, un avviso, nessun m.room.avatar.
        let big = SpaceAvatar { data: vec![0; 60 << 20], mime: "image/png".into() };
        let r = create_space(&a, acc, &format!("Avatar KO {tag}"), None, false, Some(&big)).await.unwrap();
        assert!(matches!(r.notices[..], [SpaceNotice::AvatarFailed(_)]), "{:?}", r.notices);
        println!("OUTCOME avatar_fail: {:?} — «{}»", r.notices[0], r.notices[0].line());
        let rid = id(&r.scope);
        assert_eq!(state(&a, &rid, StateEventType::RoomCreate).await.unwrap()["type"], "m.space");
        assert!(state(&a, &rid, StateEventType::RoomAvatar).await.is_none());
        println!("CASE avatar_fail: PASS");

        // 4. Seconda sessione dello stesso account (altro device) vede lo spazio dopo il sync.
        let b = login(&config).await;
        assert_ne!(a.device_id(), b.device_id());
        until("second session sees the space", || {
            b.get_room(&rid).is_some_and(|r| r.state() == RoomState::Joined && r.is_space())
        })
        .await;
        println!("CASE second_session: PASS");
    });
}
