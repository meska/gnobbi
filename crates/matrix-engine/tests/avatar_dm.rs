//! DM avatar in the room list (a-f) and avatar cache reuse (g), on a disposable Synapse/Tuwunel.
//! Driven by the avatar_dm_probe script; the fixture config comes from the environment.
//! alice watches through RoomSync (the list's source); bob and carol are the peers.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListDiff, RoomListEvent, RoomSummary, bounded,
    avatar_cache::AvatarCache,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::OwnedUserId,
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

const WAIT: Duration = Duration::from_secs(60);

fn step(s: &str) {
    println!("STEP {s}");
}

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::None)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(
            config[format!("{who}_mxid")].as_str().unwrap(),
            config[format!("{who}_password")].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    client
}

/// A distinct, valid PNG per seed (distinct bytes = distinct mxc).
fn png(seed: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([seed, 200 - seed, 90, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// PUT with an empty avatar string: the Some("") case Tuwunel hands to the SDK.
async fn raw_put(config: &serde_json::Value, who: &Client, path: &str, body: &str) {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let http = matrix_engine::sdk::reqwest::Client::builder()
        .add_root_certificate(Certificate::from_pem(&ca).unwrap())
        .build()
        .unwrap();
    let r = http
        .put(format!("{}_matrix/client/v3/{path}", config["url"].as_str().unwrap()))
        .bearer_auth(who.access_token().unwrap())
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "PUT {path}: {}", r.status());
}

async fn dm(from: &Client, to: &Client) -> (String, matrix_engine::sdk::Room) {
    let id: OwnedUserId = to.user_id().unwrap().to_owned();
    let room = from.create_dm(&id).await.unwrap();
    to.join_room_by_id(room.room_id()).await.unwrap();
    (room.room_id().to_string(), room)
}

#[test]
#[ignore = "requires the avatar_dm_probe script"]
fn dm_avatar_rules_hot_update_and_cache_reuse() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap())
            .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (alice, bob, carol) = (
            login(&config, "alice").await,
            login(&config, "bob").await,
            login(&config, "carol").await,
        );
        step("fixtures");
        // bob has an avatar, carol none.
        let mxc_a = bob.account().upload_avatar(&"image/png".parse().unwrap(), png(1)).await.unwrap().to_string();
        let (dm_a, _) = dm(&alice, &bob).await;
        let (dm_b, _) = dm(&alice, &carol).await;
        // (d) a DM that carries its own m.room.avatar, set by alice.
        let (dm_own, own_room) = dm(&alice, &bob).await;
        let mxc_own = alice
            .media()
            .upload(&"image/png".parse().unwrap(), png(2), None)
            .await
            .unwrap()
            .content_uri;
        own_room.set_avatar_url(&mxc_own, None).await.unwrap();
        // (e) a DM whose m.room.avatar is the empty string.
        let (dm_empty, empty_room) = dm(&alice, &carol).await;
        empty_room
            .send_state_event_raw("m.room.avatar", "", serde_json::json!({"url": ""}))
            .await
            .unwrap();
        // (f) a group of three, not a DM.
        let group = alice
            .create_room(matrix_engine::sdk::ruma::api::client::room::create_room::v3::Request::new())
            .await
            .unwrap();
        for who in [&bob, &carol] {
            alice
                .get_room(group.room_id())
                .unwrap()
                .invite_user_by_id(who.user_id().unwrap())
                .await
                .unwrap();
            who.join_room_by_id(group.room_id()).await.unwrap();
        }
        let group = group.room_id().to_string();

        step("adapter");
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut adapter = RoomSync::from_client(AccountId::new(7), alice.clone());
        let run = sliding::run(&mut adapter, &mut tx, || {});
        tokio::pin!(run);
        let mut rooms: BTreeMap<String, RoomSummary> = BTreeMap::new();
        macro_rules! until {
            ($what:expr, $pred:expr) => {{
                let end = tokio::time::Instant::now() + WAIT;
                loop {
                    let ok = { let f: &dyn Fn(&BTreeMap<String, RoomSummary>) -> bool = &$pred; f(&rooms) };
                    if ok { break }
                    tokio::select! {
                        _ = &mut run => panic!("adapter ended"),
                        _ = tokio::time::sleep_until(end) => panic!("timeout: {} ; avatars={:?}", $what,
                            rooms.iter().map(|(k, r)| (k.clone(), r.info.avatar.clone(), r.info.joined_members)).collect::<Vec<_>>()),
                        event = rx.recv() => match event.unwrap() {
                            RoomListEvent::Diff(RoomListDiff::Upsert(r)) => { rooms.insert(r.scope.room_id.as_str().to_owned(), r); }
                            RoomListEvent::Diff(RoomListDiff::Remove(s)) => { rooms.remove(s.room_id.as_str()); }
                            _ => {}
                        },
                    }
                }
            }};
        }
        macro_rules! settle {
            ($secs:expr) => {{
                let end = tokio::time::Instant::now() + Duration::from_secs($secs);
                loop {
                    tokio::select! {
                        _ = &mut run => panic!("adapter ended"),
                        _ = tokio::time::sleep_until(end) => break,
                        event = rx.recv() => match event.unwrap() {
                            RoomListEvent::Diff(RoomListDiff::Upsert(r)) => { rooms.insert(r.scope.room_id.as_str().to_owned(), r); }
                            _ => {}
                        },
                    }
                }
            }};
        }
        let mut defects: Vec<String> = Vec::new();
        let av = |rooms: &BTreeMap<String, RoomSummary>, id: &str| rooms.get(id).map(|r| r.info.avatar.clone());
        let (a, b, o, e, g) = (dm_a.clone(), dm_b.clone(), dm_own.clone(), dm_empty.clone(), group.clone());

        step("(a)(b)(d)(e)(f) initial");
        until!("initial avatars", |r: &BTreeMap<String, RoomSummary>| {
            av(r, &a) == Some(Some(mxc_a.clone()))
                && av(r, &b).is_some() && av(r, &e).is_some() && av(r, &o).is_some()
                && r.get(&g).is_some_and(|x| x.info.joined_members == 3)
        });
        assert_eq!(av(&rooms, &a), Some(Some(mxc_a.clone())), "(a) DM shows the peer's avatar");
        assert_eq!(av(&rooms, &b), Some(None), "(b) DM with a peer without avatar: initials");
        until!("(d) own room avatar", |r: &BTreeMap<String, RoomSummary>| av(r, &o) == Some(Some(mxc_own.to_string())));
        assert_ne!(mxc_own.to_string(), mxc_a, "fixture: distinct pictures");
        assert_eq!(av(&rooms, &e), Some(None), "(e) empty m.room.avatar url is no avatar");
        if av(&rooms, &g) != Some(None) {
            let r = alice.get_room(<&matrix_engine::sdk::ruma::RoomId>::try_from(g.as_str()).unwrap()).unwrap();
            let state = r.get_state_event_static::<matrix_engine::sdk::ruma::events::room::avatar::RoomAvatarEventContent>().await;
            // The server's own view: does the group really carry an m.room.avatar event?
            let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
            let http = matrix_engine::sdk::reqwest::Client::builder().add_root_certificate(Certificate::from_pem(&ca).unwrap()).build().unwrap();
            let status = http.get(format!("{}_matrix/client/v3/rooms/{g}/state/m.room.avatar", config["url"].as_str().unwrap()))
                .bearer_auth(alice.access_token().unwrap()).send().await.unwrap().status();
            let msg = format!(
                "(f) group of 3 shows {:?}; sdk avatar_url={:?}, local m.room.avatar state present={}, server GET m.room.avatar={}, heroes={}, is_direct={:?}",
                av(&rooms, &g), r.avatar_url(), matches!(state, Ok(Some(_))), status, r.heroes().await.len(), r.is_direct().await
            );
            println!("DEFECT {msg}");
            defects.push(msg);
        }
        until!("(d) own room avatar", |r: &BTreeMap<String, RoomSummary>| av(r, &o) == Some(Some(mxc_own.to_string())));
        assert_ne!(mxc_own.to_string(), mxc_a, "fixture: distinct pictures");
        assert_eq!(av(&rooms, &e), Some(None), "(e) empty m.room.avatar url is no avatar");

        step("(c) hot update");
        let mxc_b = bob.account().upload_avatar(&"image/png".parse().unwrap(), png(3)).await.unwrap().to_string();
        assert_ne!(mxc_a, mxc_b);
        until!("(c) peer avatar changed", |r: &BTreeMap<String, RoomSummary>| av(r, &a) == Some(Some(mxc_b.clone())));
        settle!(3);
        assert_eq!(av(&rooms, &o), Some(Some(mxc_own.to_string())), "(d) own avatar wins over the peer's change");
        if av(&rooms, &g) != Some(None) { defects.push(format!("(f) after the member's avatar change the group shows {:?}", av(&rooms, &g))); }

        step("(g) cache reuse");
        let dir = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into())).join("avatar-dm-cache");
        let cache = AvatarCache::new(dir);
        let first = cache.get(&alice, &mxc_b).await.expect("download");
        let again = cache.get(&alice, &mxc_b).await.expect("cached");
        assert_eq!(first, again);
        assert!(image::load_from_memory(&first).is_ok(), "decodable picture");
        assert_eq!(cache.downloads(), 1, "second get of the list's mxc is served from the cache");
        let _ = cache.get(&alice, &mxc_a).await;
        assert_eq!(cache.downloads(), 2, "one download per distinct mxc");

        step("(e) hot: profile avatar set to the empty string");
        let user = |c: &Client| format!("profile/{}/avatar_url", c.user_id().unwrap());
        raw_put(&config, &carol, &user(&carol), r#"{"avatar_url":""}"#).await;
        settle!(5);
        assert_eq!(av(&rooms, &b), Some(None), "empty profile avatar of the peer is no avatar");
        raw_put(&config, &bob, &user(&bob), r#"{"avatar_url":""}"#).await;
        until!("peer avatar removed", |r: &BTreeMap<String, RoomSummary>| av(r, &a) == Some(None));
        assert_eq!(av(&rooms, &o), Some(Some(mxc_own.to_string())));

        step("cleanup");
        assert!(defects.is_empty(), "defects: {defects:#?}");
        for c in [alice, bob, carol] {
            let _ = c.matrix_auth().logout().await;
        }
    });
}
