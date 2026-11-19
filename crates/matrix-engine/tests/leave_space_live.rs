//! «Esci dallo spazio» (`operations::leave_space`) su un homeserver usa e getta (Synapse o Tuwunel).
//! Driver: `PROBE_TEST=leave_space_live PROBE_SERVER=synapse|tuwunel the room_mgmt_probe script` (sulla CT).
//! Ogni esito è letto dal server, non dallo stato locale: membership con una richiesta di stato, «dimenticato»
//! con un login nuovo senza store e /sync con `include_leave` (una stanza dimenticata non compare più, una solo
//! lasciata sì come `leave`). Il blocco della UI (`space::leave_block`) si decide sui fatti di `space_children`
//! (figli con `joined`, `last_owner`): qui si verificano quei fatti sul server reale.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomScope};
use matrix_engine::{
    operations::{add_to_space, create_space, leave_space, space_children},
    sdk::{
        Client, RoomState,
        config::SyncSettings,
        reqwest::Certificate,
        ruma::{
            RoomId, assign,
            api::client::{
                filter::{FilterDefinition, RoomFilter},
                room::create_room::v3::Request as CreateRoom,
                state::get_state_event_for_key::v3::Request as StateRequest,
                sync::sync_events::v3::Filter,
            },
            events::StateEventType,
        },
        sliding_sync::VersionBuilder,
    },
};
use std::time::{Duration, Instant};

const ACCOUNT: AccountId = AccountId::new(1);

async fn sync(c: &Client) {
    c.sync_once(SyncSettings::default().timeout(Duration::from_millis(500))).await.unwrap();
}

/// Syncs `c` until `cond` holds (classic /sync, driven by the test: nothing runs in the background).
async fn until<F: std::future::Future<Output = bool>>(c: &Client, what: &str, mut cond: impl FnMut() -> F) {
    let start = Instant::now();
    loop {
        sync(c).await;
        if cond().await {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
    }
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
        .login_username(config[&format!("{who}_mxid")].as_str().unwrap(), config[&format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap_or_else(|e| panic!("login {who}: {e:?}"));
    client
}

/// `m.room.member` membership of `who` as the server holds it, read by `reader` (a member who stays in: a user
/// who left cannot read the room state any more).
async fn membership(reader: &Client, room: &RoomId, who: &Client) -> String {
    let who = who.user_id().unwrap().to_string();
    let r = reader.send(StateRequest::new(room.to_owned(), StateEventType::RoomMember, who)).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(r.event_or_content.get()).unwrap();
    v["membership"].as_str().unwrap_or("?").to_owned()
}

/// What a brand-new session of `who` (one login: the server rate-limits logins) is told about `rooms`: Joined / Left, None = the server no longer lists it (forgotten).
async fn fresh_view(config: &serde_json::Value, who: &str, rooms: [&RoomId; 2]) -> [Option<RoomState>; 2] {
    let c = login(config, who).await;
    let filter = assign!(FilterDefinition::default(), { room: assign!(RoomFilter::default(), { include_leave: true }) });
    c.sync_once(SyncSettings::default().filter(Filter::FilterDefinition(filter))).await.unwrap();
    let state = rooms.map(|room| c.get_room(room).map(|r| r.state()));
    let _ = c.matrix_auth().logout().await;
    state
}

fn scope_of(room: &RoomId) -> RoomScope {
    RoomScope { account_id: ACCOUNT, room_id: matrix_domain::RoomId::new(room.as_str()) }
}

fn rid(s: &RoomScope) -> matrix_sdk::ruma::OwnedRoomId {
    RoomId::parse(s.room_id.as_str()).unwrap()
}

/// alice creates an empty private space, bob joins it (plain member, power level 0).
async fn space_with_bob(alice: &Client, bob: &Client, name: &str) -> matrix_sdk::ruma::OwnedRoomId {
    let id = rid(&create_space(alice, ACCOUNT, name, None, false, None).await.unwrap().scope);
    alice.get_room(&id).unwrap().invite_user_by_id(bob.user_id().unwrap()).await.unwrap();
    bob.join_room_by_id(&id).await.unwrap();
    until(bob, "bob sees the space joined", async || bob.get_room(&id).is_some_and(|r| r.state() == RoomState::Joined && r.is_space())).await;
    id
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn leaving_a_space_with_and_without_forget_as_the_server_sees_it() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
      tokio::time::timeout(Duration::from_secs(150), async {
        let (alice, bob) = (login(&config, "alice").await, login(&config, "bob").await);
        let tag = std::process::id();

        println!("step 1. own room");
        // 1. Own room in the space: the facts block the leave (joined child); alice is also the last owner.
        let own = rid(&create_space(&alice, ACCOUNT, &format!("Con stanze {tag}"), None, false, None).await.unwrap().scope);
        let room = alice.create_room(CreateRoom::new()).await.unwrap();
        until(&alice, "alice's sync knows both rooms", async || alice.get_room(&own).is_some_and(|r| r.is_space()) && alice.get_room(room.room_id()).is_some()).await;
        add_to_space(&alice, ACCOUNT, &scope_of(room.room_id()), &scope_of(&own)).await.unwrap();
        until(&alice, "alice's space lists her joined room", async || {
            space_children(&alice, ACCOUNT, &scope_of(&own)).await.is_ok_and(|c| c.children.iter().any(|x| x.joined))
        })
        .await;
        let facts = space_children(&alice, ACCOUNT, &scope_of(&own)).await.unwrap();
        assert!(facts.children.iter().any(|c| c.joined && c.scope.room_id.as_str() == room.room_id().as_str()), "{facts:?}");
        assert!(facts.last_owner, "creator alone in the space is the last owner: {facts:?}");
        // The UI never offers the leave here; nothing has been left on the server.
        assert_eq!(membership(&alice, &own, &alice).await, "join");

        println!("step 2. last admin");
        // 2. Last admin of an empty space: no child, last_owner → blocked for the reason «ultimo admin».
        let solo = rid(&create_space(&alice, ACCOUNT, &format!("Ultimo admin {tag}"), None, false, None).await.unwrap().scope);
        sync(&alice).await;
        let facts = space_children(&alice, ACCOUNT, &scope_of(&solo)).await.unwrap();
        assert!(facts.children.iter().all(|c| !c.joined) && facts.last_owner, "{facts:?}");
        assert_eq!(membership(&alice, &solo, &alice).await, "join");

        println!("step 3. no forget");
        // 3. Empty space, plain member, «Dimentica» off: out on the server, still listed as left by a new session.
        let keep = space_with_bob(&alice, &bob, &format!("Senza dimenticare {tag}")).await;
        sync(&bob).await;
        let facts = space_children(&bob, ACCOUNT, &scope_of(&keep)).await.unwrap();
        assert!(facts.children.is_empty() && !facts.last_owner, "bob may leave: {facts:?}");
        let (_, r) = leave_space(&bob, ACCOUNT, &scope_of(&keep), false).await;
        assert_eq!(r, Ok(false));
        assert_eq!(membership(&alice, &keep, &bob).await, "leave");

        println!("step 4. forget");
        // 4. Same, «Dimentica» on: out, and the server no longer lists the space to a new session.
        let gone = space_with_bob(&alice, &bob, &format!("Dimenticato {tag}")).await;
        let (_, r) = leave_space(&bob, ACCOUNT, &scope_of(&gone), true).await;
        assert_eq!(r, Ok(true), "forget ran");
        assert_eq!(membership(&alice, &gone, &bob).await, "leave");
        // A new session: the space left without «Dimentica» is still listed as left, the forgotten one is gone.
        assert_eq!(fresh_view(&config, "bob", [&keep, &gone]).await, [Some(RoomState::Left), None], "left vs forgotten on the server");

        println!("step 5. already left");
        // 5. Already left from another session while this one still believes it is in: no error.
        let elsewhere = space_with_bob(&alice, &bob, &format!("Già uscito {tag}")).await;
        // bob's first session does not sync from here on: it keeps its stale local state
        let bob2 = login(&config, "bob").await;
        sync(&bob2).await;
        bob2.get_room(&elsewhere).expect("bob2 knows the space").leave().await.unwrap();
        assert_eq!(membership(&alice, &elsewhere, &bob2).await, "leave");
        assert_eq!(bob.get_room(&elsewhere).unwrap().state(), RoomState::Joined, "stale session");
        let (_, r) = leave_space(&bob, ACCOUNT, &scope_of(&elsewhere), true).await;
        assert!(r.is_ok(), "already left must not fail: {r:?}");
        assert_eq!(membership(&alice, &elsewhere, &bob).await, "leave");

        // Teardown of the sessions (the driver removes the homeserver and its data).
        let _ = bob2.matrix_auth().logout().await;
        for c in [alice, bob] {
            let _ = c.matrix_auth().logout().await;
        }
    })
    .await
    .expect("test body exceeded 150 s (last step printed above)");
    });
}
