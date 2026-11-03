//! Status rows (join, invite, rename…) from the SDK timeline on a disposable homeserver (Synapse or Tuwunel).
//! Alice owns the room and reads it; bob and carol (no display name set) are acted on. Unknown/hidden state
//! must stay out. Driver: PROBE_TARGET=status_rows PROBE_TEST=status_rows_cover_every_kind
//! PROBE_SERVER=synapse|tuwunel python3 the timeline_probe script (via the ct script).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    receipts::ReadVisibility,
    room_sync::RoomSync,
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            serde::Raw,
            OwnedUserId,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

fn carol_id() -> OwnedUserId {
    "@carol:matrix-desktop.test".try_into().unwrap()
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn build(config: &serde_json::Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    c.matrix_auth()
        .login_username(&format!("@{who}:matrix-desktop.test"), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
    c
}

/// Status rows alice sees, oldest first: (body, sender id).
async fn rows(sync: &RoomSync, views: &mut ViewRequests, scope: &RoomScope) -> Vec<(String, String)> {
    let hidden = ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
    let (page, _) = sync.open_history(views.start(scope.clone()).unwrap(), hidden).await.unwrap();
    page.events.iter().filter(|e| e.status).map(|e| (e.body.clone(), e.sender.id.clone())).collect()
}

/// Wait until at least `count` status rows contain `frag`; panics with all rows seen.
async fn expect(sync: &RoomSync, views: &mut ViewRequests, scope: &RoomScope, frag: &str, count: usize) -> Vec<(String, String)> {
    let started = Instant::now();
    loop {
        let r = rows(sync, views, scope).await;
        if r.iter().filter(|(b, _)| b.contains(frag)).count() >= count {
            return r;
        }
        assert!(started.elapsed() < Duration::from_secs(40), "missing {count}x «{frag}» in {r:#?}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn room_of(c: &Client, id: &matrix_engine::sdk::ruma::OwnedRoomId) -> Room {
    let started = Instant::now();
    loop {
        if let Some(r) = c.get_room(id) {
            return r;
        }
        assert!(started.elapsed() < Duration::from_secs(40), "room not synced");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn status_rows_cover_every_kind() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (build(&config, "alice").await, build(&config, "bob").await);
        let bob: OwnedUserId = b.user_id().unwrap().to_owned();
        let carol: OwnedUserId = carol_id();
        let carol_c = build(&config, "carol").await;
        let mut creation = CreationContent::new();
        creation.federate = false;
        let room_id = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap()
            .room_id()
            .to_owned();
        let (sa, sb, sc) = (
            SyncService::builder(a.clone()).build().await.unwrap(),
            SyncService::builder(b.clone()).build().await.unwrap(),
            SyncService::builder(carol_c.clone()).build().await.unwrap(),
        );
        for s in [&sa, &sb, &sc] {
            s.start().await;
        }
        let ra = room_of(&a, &room_id).await;
        let sync = RoomSync::from_client(AccountId::new(1), a.clone());
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(room_id.as_str()) };
        let mut views = ViewRequests::default();
        let alice = a.user_id().unwrap().to_string();

        // Room state: name, topic, avatar, topic removed, name removed.
        ra.set_name("Casa".into()).await.unwrap();
        ra.set_room_topic("Cena").await.unwrap();
        ra.send_state_event_raw("m.room.avatar", "", serde_json::json!({"url": "mxc://matrix-desktop.test/fake"})).await.unwrap();
        ra.set_room_topic("").await.unwrap();
        // Hidden/unknown state: must not become rows and must not break the timeline.
        ra.send_state_event(RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared)).await.unwrap();
        ra.send_state_event_raw("org.example.unknown", "k", serde_json::json!({"x": 1})).await.unwrap();
        ra.send_state_event_raw("m.room.guest_access", "", serde_json::json!({"guest_access": "forbidden"})).await.unwrap();

        // Membership: invite, reject, invite, revoke, invite, join, leave, rejoin, kick, ban, unban.
        ra.invite_user_by_id(&bob).await.unwrap();
        let rb = room_of(&b, &room_id).await;
        rb.leave().await.unwrap(); // invite rejected
        ra.invite_user_by_id(&bob).await.unwrap();
        ra.kick_user(&bob, None).await.unwrap(); // invitation revoked
        ra.invite_user_by_id(&bob).await.unwrap();
        b.join_room_by_id(&room_id).await.unwrap();
        let rb = room_of(&b, &room_id).await;
        rb.leave().await.unwrap();
        ra.invite_user_by_id(&bob).await.unwrap();
        b.join_room_by_id(&room_id).await.unwrap(); // second join of a series
        // Member profile: name set, changed, avatar, name removed (m.room.member updates in every joined room).
        let account = b.account();
        account.set_display_name(Some("Roberto")).await.unwrap();
        expect(&sync, &mut views, &scope, "ha cambiato il nome in Roberto", 1).await;
        account.set_display_name(Some("Bob")).await.unwrap();
        expect(&sync, &mut views, &scope, "Roberto ha cambiato il nome in Bob", 1).await;
        account.set_avatar_url(Some("mxc://matrix-desktop.test/fakeavatar".into())).await.unwrap();
        expect(&sync, &mut views, &scope, "ha cambiato l'avatar", 2).await; // room avatar + member avatar
        account.set_display_name(None).await.unwrap();
        expect(&sync, &mut views, &scope, "ha rimosso il nome visualizzato", 1).await;
        ra.kick_user(&bob, None).await.unwrap();
        ra.ban_user(&bob, None).await.unwrap();
        ra.unban_user(&bob, None).await.unwrap();
        // carol has no display name of her own: row falls back to her ID
        ra.invite_user_by_id(&carol).await.unwrap();
        carol_c.join_room_by_id(&room_id).await.unwrap();

        let all = expect(&sync, &mut views, &scope, "nella stanza", 4).await; // alice (creator), bob twice, carol
        for (b, s) in &all {
            eprintln!("STATUS {s} | {b}");
        }
        let text = |t: &str| all.iter().any(|(b, _)| b == t);
        let n = |frag: &str| all.iter().filter(|(b, _)| b.contains(frag)).count();
        let _ = alice;
        assert!(text("@alice:matrix-desktop.test ha creato questa stanza.") || n("ha creato questa stanza") == 1, "creation");
        assert!(n("ha cambiato il nome della stanza in «Casa»") == 1, "room name");
        assert!(n("ha cambiato l'argomento della stanza in «Cena»") == 1, "topic");
        assert!(n("ha rimosso l'argomento della stanza") == 1, "topic removed");
        assert!(n("ha cambiato l'avatar della stanza") >= 1, "room avatar");
        assert_eq!(n("ha invitato"), 5, "invites: bob x4, carol");
        assert!(n("ha rifiutato l'invito") == 1, "reject");
        assert!(n("ha revocato l'invito a") == 1, "revoke");
        // «entrato/a» is the wording today; only the verb-free tail is pinned here.
        assert_eq!(n("nella stanza"), 4, "joins: creator, bob x2 (a join/leave series), carol; none dropped or merged");
        assert_eq!(n("dalla stanza"), 2, "one leave, one kick");
        assert!(n("ha rimosso") >= 1 && n("dalla stanza") >= 1, "kick");
        assert_eq!(n("ha cambiato il nome in"), 2, "member rename x2");
        assert_eq!(n("ha cambiato l'avatar"), 2, "member avatar + room avatar");
        // no display name: the user ID stands in for the name, in this row and in the ones about them
        assert_eq!(n("@bob:matrix-desktop.test ha rimosso il nome visualizzato"), 1, "id fallback");
        assert_eq!(n("ha rimosso @bob:matrix-desktop.test dalla stanza"), 1, "kick names the id");
        assert!(n("ha bandito") == 1, "ban");
        assert!(n("ha revocato il ban a") == 1, "unban");
        // unknown/hidden state: nothing mentions it, no empty or raw-type text
        assert!(all.iter().all(|(b, _)| !b.trim().is_empty() && !b.contains("org.example") && !b.contains("m.room.")), "hidden state leaked");
        // fallback: a user with no display name shows as the ID, never empty
        for (b, _) in &all {
            assert!(!b.contains("  ") && !b.starts_with(' ') && !b.contains("«»"), "empty name in «{b}»");
        }
        // A plain message after all the state still comes through as a normal row, after the status rows.
        ra.send(matrix_engine::sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("ciao")).await.unwrap();
        let hidden = ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
        let started = Instant::now();
        loop {
            let (page, _) = sync.open_history(views.start(scope.clone()).unwrap(), hidden).await.unwrap();
            if let Some(last) = page.events.last().filter(|e| e.body == "ciao") {
                assert!(!last.status);
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(40), "message after state rows");
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let _ = (sa.stop().await, sb.stop().await, sc.stop().await);
    });
}
