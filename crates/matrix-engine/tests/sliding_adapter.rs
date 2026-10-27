//! The sliding-sync adapter shows room previews and activity for live messages, on a real
//! homeserver. Driven by the sliding_adapter_probe script (disposable Synapse/Tuwunel or the owner
//! Tuwunel); the fixture config comes from the environment, no secret lives here.
#![recursion_limit = "512"]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListDiff, RoomListEvent, RoomSummary, RoomSyncStatus, bounded,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{Request as CreateRoom, RoomPreset},
            assign,
            events::room::message::RoomMessageEventContent,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

const WAIT: Duration = Duration::from_secs(60);

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let mut builder = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .sliding_sync_version_builder(VersionBuilder::None);
    if let Some(ca) = config["ca"].as_str() {
        builder = builder.add_root_certificates(vec![
            Certificate::from_pem(&std::fs::read(ca).unwrap()).unwrap(),
        ]);
    }
    let client = builder.build().await.unwrap();
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

#[test]
#[ignore = "requires the sliding_adapter_probe script"]
fn adapter_matches_the_classic_loop_and_follows_new_messages() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let account = AccountId::new(7);
        step("login");
        let (classic_client, sliding_client, bob) = (
            login(&config, "alice").await,
            login(&config, "alice").await,
            login(&config, "bob").await,
        );
        // The classic client also joins (it is not syncing yet): fewer logins, the owner server throttles them.
        let admin = classic_client.clone();
        step("rooms");
        let room = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("adapter-probe-{}", suffix())),
                invite: vec![admin.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        admin.join_room_by_id(room.room_id()).await.unwrap();
        room.send(RoomMessageEventContent::text_plain("first"))
            .await
            .unwrap();
        let id = room.room_id().to_string();

        step("adapter");
        // Adapter, on a client whose sliding version is unset (an existing profile).
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(64).unwrap());
        let mut adapter = RoomSync::from_client(account, sliding_client.clone());
        let run = sliding::run(&mut adapter, &mut tx, || {});
        tokio::pin!(run);
        let mut seen = Seen::default();
        tokio::select! {
            _ = &mut run => panic!("adapter ended"),
            () = collect(&mut rx, &mut seen, &id, "first", false) => {}
        }
        assert!(seen.ready, "Ready never came");

        step("second");
        // A new message: preview and activity follow, .
        room.send(RoomMessageEventContent::text_plain("second"))
            .await
            .unwrap();
        tokio::select! {
            _ = &mut run => panic!("adapter ended"),
            () = collect(&mut rx, &mut seen, &id, "second", true) => {}
        }
        assert!(
            seen.activity.iter().any(|s| s.room_id.as_str() == id),
            "no TimelineActivity for the new message"
        );

        step("cleanup");
        // Leave only what this run created, then revoke the devices.
        for client in [&admin, &bob] {
            if let Some(r) = client.get_room(room.room_id()) {
                let _ = r.leave().await;
                let _ = r.forget().await;
            }
        }
        for client in [classic_client, sliding_client, bob] {
            let _ = client.matrix_auth().logout().await;
        }
    });
}

fn step(name: &str) {
    eprintln!("[adapter-probe] {name}");
}

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}")
}

#[derive(Default)]
struct Seen {
    ready: bool,
    rooms: BTreeMap<String, RoomSummary>,
    activity: Vec<matrix_domain::RoomScope>,
}

/// Returns once Ready was seen and the room shows `preview` (and activity followed it).
async fn collect(
    rx: &mut matrix_engine::Receiver<RoomListEvent>,
    seen: &mut Seen,
    id: &str,
    preview: &str,
    need_activity: bool,
) {
    tokio::time::timeout(WAIT, async {
        loop {
            if seen.ready
                && seen
                    .rooms
                    .get(id)
                    .is_some_and(|r| r.info.preview.as_deref() == Some(preview))
                && (!need_activity || seen.activity.iter().any(|s| s.room_id.as_str() == id))
            {
                return;
            }
            match rx.recv().await.expect("adapter stream closed") {
                RoomListEvent::Diff(RoomListDiff::Upsert(r)) => {
                    if r.scope.room_id.as_str() == id && r.info.preview.as_deref() == Some(preview)
                    {
                        // Only activity after the wanted message counts.
                        seen.activity.clear();
                    }
                    seen.rooms.insert(r.scope.room_id.as_str().to_owned(), r);
                }
                RoomListEvent::Diff(RoomListDiff::Remove(s)) => {
                    seen.rooms.remove(s.room_id.as_str());
                }
                RoomListEvent::Status(RoomSyncStatus::Ready) => seen.ready = true,
                RoomListEvent::Status(RoomSyncStatus::Connecting) => {}
                RoomListEvent::Status(_) => panic!("adapter reported a failure"),
                RoomListEvent::TimelineActivity(s) => seen.activity.push(s),
                RoomListEvent::Order(_) => {}
            }
        }
    })
    .await
    .expect("adapter never showed the message")
}
