//! Per-room notification modes end to end on a disposable Synapse or Tuwunel: alice (A) is the client under
//! test, bob (B) writes. Counts the `RoomListEvent::Alert`s of the real adapter (what the desktop turns into
//! an OS notification), reads the push rules the server holds after each choice, restarts as a second device
//! and breaks the network during a mode change. Driver: PROBE_TARGET=notification_modes
//! PROBE_TEST=every_notification_mode_decides_alerts PROBE_SERVER=synapse|tuwunel python3 the timeline_probe script
//! (via the ct script). Takes ~3 min per server: every negative check waits a fixed window and is followed by a
//! positive control in another room, so silence cannot come from a dead channel.
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_domain::{AccountId, RoomId, RoomScope, notifications::RoomNotifyMode};
use matrix_engine::{
    Receiver, RoomListDiff, RoomListEvent, RoomSyncStatus, bounded,
    notify::set_notification_mode,
    operations::OperationError,
    room_sync::RoomSync,
    sdk::{
        Client, Room,
        notification_settings::RoomNotificationMode,
        reqwest::Certificate,
        ruma::{
            api::client::{
                push::get_pushrules_all,
                room::create_room::v3::{Request as CreateRoom, RoomPreset},
            },
            assign,
            events::{Mentions, room::message::RoomMessageEventContent},
            push::Action,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{collections::HashMap, num::NonZeroUsize, time::Duration};
use tokio::{net::TcpListener, task::JoinHandle, time::Instant};

const WAIT: Duration = Duration::from_secs(60);
/// How long a message that must stay silent is given to (wrongly) produce an alert.
const QUIET: Duration = Duration::from_secs(6);

async fn login(config: &serde_json::Value, who: &str, url: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(url)
        .sliding_sync_version_builder(VersionBuilder::None)
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .with_enable_automatic_back_pagination(true)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
    client
}

/// The real adapter of one client, running on a local task, and what it sent so far.
struct Probe {
    account: AccountId,
    rx: Receiver<RoomListEvent>,
    job: JoinHandle<()>,
    alerts: Vec<String>,
    modes: HashMap<String, RoomNotifyMode>,
    ready: bool,
}

impl Probe {
    fn start(account: AccountId, client: &Client) -> Probe {
        let (mut tx, rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut adapter = RoomSync::from_client(account, client.clone());
        let job = tokio::task::spawn_local(async move { sliding::run(&mut adapter, &mut tx, || {}).await });
        Probe { account, rx, job, alerts: vec![], modes: HashMap::new(), ready: false }
    }

    async fn pump(&mut self, window: Duration, until: impl Fn(&Probe) -> bool) {
        let end = Instant::now() + window;
        while !until(self) {
            let Ok(event) = tokio::time::timeout_at(end, self.rx.recv()).await else { return };
            match event.expect("adapter stream closed") {
                RoomListEvent::Status(RoomSyncStatus::Ready) => self.ready = true,
                RoomListEvent::Alert(s, _) => {
                    assert_eq!(s.account_id, self.account);
                    self.alerts.push(s.room_id.as_str().to_owned());
                }
                RoomListEvent::Diff(RoomListDiff::Upsert(r)) => {
                    self.modes.insert(r.scope.room_id.as_str().to_owned(), r.info.notification_mode);
                }
                RoomListEvent::Status(RoomSyncStatus::Connecting) => {}
                RoomListEvent::Status(s) => { let _ = s; panic!("adapter reported a failure") },
                _ => {}
            }
        }
    }

    fn count(&self, room: &str) -> usize {
        self.alerts.iter().filter(|r| *r == room).count()
    }

    /// `n` more alerts for `room` than `before`, within WAIT, then no further one during QUIET.
    async fn alerts_exactly(&mut self, room: &str, before: usize, n: usize, what: &str) {
        self.pump(WAIT, |p| p.count(room) >= before + n).await;
        self.pump(QUIET, |_| false).await;
        assert_eq!(self.count(room) - before, n, "{what}: expected {n} alert(s) for the room, all alerts: {:?}", self.alerts);
    }

    /// Nothing for `room` during QUIET; then a positive control in `control` must alert exactly once.
    async fn silent(&mut self, room: &str, control: &Room, what: &str) {
        let (before, control_id) = (self.count(room), control.room_id().as_str().to_owned());
        let control_before = self.count(&control_id);
        self.pump(QUIET, |_| false).await;
        assert_eq!(self.count(room), before, "{what}: the room must not alert; all alerts: {:?}", self.alerts);
        say(what, "silent");
        control.send(text("control")).await.unwrap();
        self.alerts_exactly(&control_id, control_before, 1, &format!("{what}: control")).await;
    }

    /// Writes the mode like the UI does and waits until both the SDK cache and the projected room show it.
    async fn set_mode(&mut self, client: &Client, room: &Room, mode: RoomNotifyMode) {
        let scope = RoomScope { account_id: self.account, room_id: RoomId::new(room.room_id().as_str()) };
        set_notification_mode(client, self.account, &scope, mode).await.unwrap();
        let id = room.room_id().as_str().to_owned();
        let want = sdk_mode(mode);
        let mine = client.get_room(room.room_id()).unwrap();
        self.pump(WAIT, |p| mine.cached_user_defined_notification_mode() == Some(want) && p.modes.get(&id) == Some(&mode)).await;
        assert_eq!(self.modes.get(&id), Some(&mode), "the mode never reached the room list");
    }
}

fn sdk_mode(mode: RoomNotifyMode) -> RoomNotificationMode {
    match mode {
        RoomNotifyMode::AllMessages => RoomNotificationMode::AllMessages,
        RoomNotifyMode::MentionsAndKeywordsOnly => RoomNotificationMode::MentionsAndKeywordsOnly,
        RoomNotifyMode::Mute => RoomNotificationMode::Mute,
    }
}

fn text(body: &str) -> RoomMessageEventContent {
    RoomMessageEventContent::text_plain(body)
}

fn mention(body: &str, who: &Client) -> RoomMessageEventContent {
    text(body).add_mentions(Mentions::with_user_ids([who.user_id().unwrap().to_owned()]))
}

fn say(step: &str, result: &str) {
    eprintln!("[notification-modes] {step}: {result}");
}

/// The push rules the server holds for the room, whatever the list: `room:notify`, `room:-`, `override:-`.
async fn shape(client: &Client, room: &str) -> Vec<String> {
    let global = client.send(get_pushrules_all::v3::Request::new()).await.unwrap().global;
    let acts = |a: &[Action]| if a.iter().any(|a| matches!(a, Action::Notify)) { "notify" } else { "-" };
    let mut found: Vec<String> = global.room.iter().filter(|r| r.rule_id == room).map(|r| format!("room:{}", acts(&r.actions))).collect();
    found.extend(global.override_.iter().filter(|r| r.rule_id == room).map(|r| format!("override:{}", acts(&r.actions))));
    found.extend(global.underride.iter().filter(|r| r.rule_id == room).map(|r| format!("underride:{}", acts(&r.actions))));
    found.sort();
    found
}

/// A TCP pass-through to the server: TLS stays end to end, so the certificate for `localhost` still holds.
struct Proxy {
    port: u16,
    accept: JoinHandle<()>,
    conns: std::sync::Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl Proxy {
    async fn start(target: u16) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let conns: std::sync::Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> = Default::default();
        let list = conns.clone();
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let conn = tokio::spawn(async move {
                    if let Ok(mut outbound) = tokio::net::TcpStream::connect(("127.0.0.1", target)).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                list.lock().unwrap().push(conn);
            }
        });
        Proxy { port, accept, conns }
    }

    /// The network goes away: no new connection, the open ones are cut.
    fn cut(&self) {
        self.accept.abort();
        self.conns.lock().unwrap().iter().for_each(JoinHandle::abort);
    }
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn every_notification_mode_decides_alerts() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        let url = config["url"].as_str().unwrap().to_owned();
        let account = AccountId::new(7);
        let (alice, bob) = (login(&config, "alice", &url).await, login(&config, "bob", &url).await);
        let alice_id = alice.user_id().unwrap().to_owned();
        let make = |name: &str| {
            let (bob, alice_id, name) = (bob.clone(), alice_id.clone(), format!("{name}-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            async move {
                bob.create_room(assign!(CreateRoom::new(), { preset: Some(RoomPreset::PrivateChat), name: Some(name), invite: vec![alice_id] })).await.unwrap()
            }
        };
        let (r, c, l) = (make("modes-r").await, make("modes-control").await, make("modes-low").await);
        for room in [&r, &c, &l] {
            alice.join_room_by_id(room.room_id()).await.unwrap();
        }
        let (rid, cid, lid) = (r.room_id().as_str().to_owned(), c.room_id().as_str().to_owned(), l.room_id().as_str().to_owned());
        // The cutoff of the adapter is its start; the server stamps whole milliseconds on its own clock.
        tokio::time::sleep(Duration::from_secs(2)).await;

        let mut p = Probe::start(account, &alice);
        p.pump(WAIT, |p| p.ready && [&rid, &cid, &lid].iter().all(|id| p.modes.contains_key(*id))).await;
        assert!(p.ready, "Ready never came");
        let mine = alice.get_room(r.room_id()).unwrap();
        assert!(p.alerts.is_empty(), "history replayed at start must not alert: {:?}", p.alerts);
        assert!(shape(&alice, &rid).await.is_empty(), "no rule before any choice");

        // 1. Low priority: a normal message does not notify.
        mine.set_is_low_priority(true, None).await.unwrap();
        p.pump(WAIT, |_| mine.is_low_priority()).await;
        assert!(mine.is_low_priority(), "the low priority tag never reached the client");
        r.send(text("normal while low priority")).await.unwrap();
        p.silent(&rid, &c, "1 low priority, normal message").await;

        // 2. «Silenziata»: nothing, not even a direct mention; the server keeps counting.
        p.set_mode(&alice, &r, RoomNotifyMode::Mute).await;
        assert_eq!(shape(&alice, &rid).await, ["override:-"], "Mute is one Override rule with no actions");
        r.send(text("normal while muted")).await.unwrap();
        r.send(mention("mention while muted", &alice)).await.unwrap();
        p.silent(&rid, &c, "2 mute, normal and mention").await;
        let counts = (mine.num_unread_messages(), mine.unread_notification_counts().notification_count, mine.unread_notification_counts().highlight_count);
        say("2 unread counters (messages, server notification, server highlight)", &format!("{counts:?}"));
        assert!(counts.0 >= 2, "the muted room must stay among the unread ones of the SDK, got {counts:?}");

        // 3. Low priority, not muted: a mention and an invite do notify; a normal message and a keyword do not.
        p.set_mode(&alice, &r, RoomNotifyMode::AllMessages).await;
        assert_eq!(shape(&alice, &rid).await, ["room:notify"], "Tutti i messaggi is one Room rule [notify], the Override is gone");
        assert!(mine.is_low_priority());
        let before = p.count(&rid);
        r.send(mention("mention in low priority", &alice)).await.unwrap();
        p.alerts_exactly(&rid, before, 1, "3 low priority, mention").await;
        alice.notification_settings().await.add_keyword("zebra123".into()).await.unwrap();
        p.pump(Duration::from_secs(4), |_| false).await;
        r.send(text("zebra123 passes by")).await.unwrap();
        p.silent(&rid, &c, "3 low priority, keyword (contract: silent)").await;
        let d = make("modes-invite").await;
        let (did, before) = (d.room_id().as_str().to_owned(), p.count(d.room_id().as_str()));
        p.alerts_exactly(&did, before, 1, "3 invite while low priority elsewhere").await;
        say("3 invite", "one alert, no repeat within the quiet window");

        // 4. «Solo menzioni» (no longer low priority, to isolate the mode): normal no, mention yes.
        mine.set_is_low_priority(false, None).await.unwrap();
        p.pump(WAIT, |_| !mine.is_low_priority()).await;
        p.set_mode(&alice, &r, RoomNotifyMode::MentionsAndKeywordsOnly).await;
        assert_eq!(shape(&alice, &rid).await, ["room:-"], "Solo menzioni is one Room rule with no actions");
        r.send(text("normal in mentions only")).await.unwrap();
        p.silent(&rid, &c, "4 mentions only, normal message").await;
        let before = p.count(&rid);
        r.send(mention("mention in mentions only", &alice)).await.unwrap();
        p.alerts_exactly(&rid, before, 1, "4 mentions only, mention").await;

        // 5. Silence off: both come back.
        p.set_mode(&alice, &r, RoomNotifyMode::AllMessages).await;
        let before = p.count(&rid);
        r.send(text("normal after unmute")).await.unwrap();
        p.alerts_exactly(&rid, before, 1, "5 all messages, normal").await;
        r.send(mention("mention after unmute", &alice)).await.unwrap();
        p.alerts_exactly(&rid, before + 1, 1, "5 all messages, mention").await;

        // 6. Restart as a second device with messages that arrived while offline.
        p.set_mode(&alice, &r, RoomNotifyMode::Mute).await;
        let low = alice.get_room(l.room_id()).unwrap();
        low.set_is_low_priority(true, None).await.unwrap();
        p.pump(WAIT, |_| low.is_low_priority()).await;
        assert!(low.is_low_priority());
        p.job.abort();
        drop(p);
        for (room, body) in [(&r, "offline normal"), (&l, "offline normal"), (&c, "offline normal")] {
            room.send(text(body)).await.unwrap();
        }
        r.send(mention("offline mention", &alice)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await; // the aborted run stops its SyncService asynchronously
        let names = |p: &Probe| p.alerts.iter().map(|id| match id.as_str() { i if i == rid => "R(muted)", i if i == cid => "C(control)", i if i == lid => "L(low priority)", i if i == did => "D(pending invite)", _ => "?" }).collect::<Vec<_>>();
        // 6a. Same device, same store: what a restart of the app does.
        let mut p = Probe::start(account, &alice);
        p.pump(WAIT, |p| p.ready && [&rid, &cid, &lid].iter().all(|id| p.modes.contains_key(*id))).await;
        p.pump(QUIET, |_| false).await;
        assert!(p.alerts.is_empty(), "no burst for what arrived offline (same device): {:?}", names(&p));
        assert_eq!(p.modes.get(&rid), Some(&RoomNotifyMode::Mute), "the mute survives the restart");
        r.send(text("live normal after restart")).await.unwrap();
        r.send(mention("live mention after restart", &alice)).await.unwrap();
        l.send(text("live normal in low priority")).await.unwrap();
        p.silent(&rid, &c, "6a after restart, muted + low priority rooms").await;
        assert_eq!(p.count(&lid), 0);
        assert_eq!(p.count(&did), 0, "the invite pending at the restart must not alert again");
        say("6a restart", "offline messages: 0 alerts; live: muted and low priority silent, control alerts; pending invite silent");
        p.job.abort();
        tokio::time::sleep(Duration::from_secs(1)).await;
        for (room, body) in [(&r, "offline 2 normal"), (&l, "offline 2 normal"), (&c, "offline 2 normal")] {
            room.send(text(body)).await.unwrap();
        }
        // 6b. Another device with an empty store: sees the mute and the low priority, no burst of messages.
        let alice2 = login(&config, "alice", &url).await;
        let mut p = Probe::start(account, &alice2);
        p.pump(WAIT, |p| p.ready && [&rid, &cid, &lid].iter().all(|id| p.modes.contains_key(*id))).await;
        p.pump(QUIET, |_| false).await;
        say("6b fresh device alerts", &format!("{:?}", names(&p)));
        assert!(p.alerts.iter().all(|id| *id == did), "no message burst on a fresh device: {:?}", names(&p));
        assert_eq!(p.modes.get(&rid), Some(&RoomNotifyMode::Mute), "the mute is visible to the other device");
        assert!(alice2.get_room(l.room_id()).unwrap().is_low_priority(), "low priority is visible to the other device");
        assert_eq!(shape(&alice2, &rid).await, ["override:-"]);
        p.job.abort();

        // 8. The network goes away during a mode change: an error, and the server rule is untouched.
        let target: u16 = url.trim_end_matches('/').rsplit(':').next().unwrap().parse().unwrap();
        let proxy = Proxy::start(target).await;
        // Log in directly, then restore that session on a client that only knows the proxy: a login response
        // can carry a well-known that moves the SDK client to the real server (Synapse does).
        let direct = login(&config, "alice", &url).await;
        let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
        let alice3 = Client::builder()
            .homeserver_url(format!("https://localhost:{}/", proxy.port))
            .sliding_sync_version_builder(VersionBuilder::None)
            .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        alice3.matrix_auth().restore_session(direct.matrix_auth().session().unwrap(), Default::default()).await.unwrap();
        alice3.sync_once(Default::default()).await.unwrap();
        proxy.cut();
        tokio::time::sleep(Duration::from_millis(500)).await; // aborts take effect at the next poll
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", proxy.port)).await.is_err(), "the proxy must be gone");
        let scope = RoomScope { account_id: account, room_id: RoomId::new(&rid) };
        say("8 before the cut write", &format!("alice3 sees mode {:?}, joined {}", alice3.get_room(r.room_id()).and_then(|x| x.cached_user_defined_notification_mode()), alice3.get_room(r.room_id()).is_some()));
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(120), set_notification_mode(&alice3, account, &scope, RoomNotifyMode::MentionsAndKeywordsOnly)).await;
        say("8 network cut", &format!("{result:?} after {:.1}s", started.elapsed().as_secs_f64()));
        say("8 server rule after the write", &format!("{:?}", shape(&alice2, &rid).await));
        assert_eq!(result.expect("the write hangs when the network is gone"), Err(OperationError::Transport));
        assert_eq!(shape(&alice2, &rid).await, ["override:-"], "a failed write must leave the server rule as it was");

        for client in [&alice, &alice2, &bob] {
            for room in [&rid, &cid, &lid, &did] {
                if let Some(room) = client.get_room(<&matrix_engine::sdk::ruma::RoomId>::try_from(room.as_str()).unwrap()) {
                    let _ = room.leave().await;
                    let _ = room.forget().await;
                }
            }
        }
        for client in [alice, alice2, alice3, direct, bob] {
            let _ = client.matrix_auth().logout().await;
        }
    });
}
