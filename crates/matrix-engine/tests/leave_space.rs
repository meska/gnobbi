//! «Esci dallo spazio» (`operations::leave_space`) against an in-process loopback homeserver (plain /sync, no
//! Docker), same style as leave_forbidden.rs. The server logs every request, so the tests pin order and number of
//! the calls: only the space itself is left (never the sub-space or the plain room inside it), the membership
//! recheck follows the leave, and `Room::forget` (POST /forget) happens only after a leave that succeeded or was
//! confirmed by the recheck, and only when «Dimentica» is on.
//! Choices the contract leaves open, kept as `leave_room` does (see room_forgotten_after_confirmed_leave below):
//!  - 200 on /leave is still rechecked with the server (a lying 200 with membership `join` is `Forbidden`);
//!  - a 403 confirmed «already out» is `Ok`, but the vendored SDK keeps the room Joined locally, so
//!    `Room::forget` refuses (WrongRoomState) before any request: result `Ok(false)`, the leave is not undone;
//!  - any other /leave status (400, 404) is not rechecked: 404 = `Unavailable`, the rest `Transport`.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    operations::{OperationError, leave_space},
    sdk::{
        Client, RoomState, config::SyncSettings, ruma::RoomId as SdkRoomId,
        sliding_sync::VersionBuilder,
    },
};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const SPACE: &str = "!space:fake.test";
const SUB: &str = "!sub:fake.test";
const ROOM: &str = "!room:fake.test";
const ME: &str = "@a:fake.test";

#[derive(Clone, Copy)]
enum Leave {
    Status(u16),
    Hang,
}

#[derive(Clone, Copy)]
struct Cfg {
    leave: Leave,
    /// Answer to the read of our own m.room.member: (status, body).
    member: (u16, &'static str),
    /// A second admin in the space: we are then not the last owner.
    other_admin: bool,
}

const JOIN: (u16, &str) = (200, r#"{"membership":"join"}"#);
const OUT: (u16, &str) = (200, r#"{"membership":"leave"}"#);
const NO_MEMBER: (u16, &str) = (404, r#"{"errcode":"M_NOT_FOUND","error":"x"}"#);
const DENIED: (u16, &str) = (403, r#"{"errcode":"M_FORBIDDEN","error":"x"}"#);

fn ev(kind: &str, key: &str, content: String) -> String {
    format!(
        r#"{{"type":"{kind}","state_key":"{key}","sender":"{ME}","event_id":"$e{}","origin_server_ts":1,"content":{content}}}"#,
        kind.len() + key.len()
    )
}

/// SPACE (we are admin), SUB = joined sub-space of SPACE, ROOM = plain room that SPACE lists as a child.
fn sync_body(other_admin: bool) -> String {
    let create = ev(
        "m.room.create",
        "",
        format!(r#"{{"creator":"{ME}","room_version":"10","type":"m.space"}}"#),
    );
    let me = ev("m.room.member", ME, r#"{"membership":"join"}"#.into());
    let (users, extra) = if other_admin {
        (
            format!(r#"{{"{ME}":100,"@b:fake.test":100}}"#),
            ev(
                "m.room.member",
                "@b:fake.test",
                r#"{"membership":"join"}"#.into(),
            ),
        )
    } else {
        (format!(r#"{{"{ME}":100}}"#), String::new())
    };
    let pl = ev("m.room.power_levels", "", format!(r#"{{"users":{users}}}"#));
    let space_state = [
        create.clone(),
        me.clone(),
        pl.clone(),
        ev("m.space.child", SUB, r#"{"via":["fake.test"]}"#.into()),
        ev("m.space.child", ROOM, r#"{"via":["fake.test"]}"#.into()),
        extra,
    ]
    .into_iter()
    .filter(|e| !e.is_empty())
    .collect::<Vec<_>>()
    .join(",");
    let sub_state = [
        create,
        me.clone(),
        pl,
        ev(
            "m.space.parent",
            SPACE,
            r#"{"via":["fake.test"],"canonical":true}"#.into(),
        ),
    ]
    .join(",");
    let room_state = ev("m.room.member", ME, r#"{"membership":"join"}"#.into());
    format!(
        r#"{{"next_batch":"1","rooms":{{"join":{{"{SPACE}":{{"state":{{"events":[{space_state}]}}}},"{SUB}":{{"state":{{"events":[{sub_state}]}}}},"{ROOM}":{{"state":{{"events":[{room_state}]}}}}}}}}}}"#
    )
}

type Log = Arc<Mutex<Vec<String>>>;

async fn serve(cfg: Cfg) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let seen = log.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let mut parts = head.split(' ');
                let (method, path) = (
                    parts.next().unwrap_or("").to_owned(),
                    parts.next().unwrap_or("").to_owned(),
                );
                let (status, body) = if path.contains("/login") {
                    (
                        200,
                        format!(r#"{{"user_id":"{ME}","access_token":"t","device_id":"D"}}"#),
                    )
                } else if path.contains("/versions") {
                    (
                        200,
                        r#"{"versions":["v1.11"],"unstable_features":{}}"#.to_owned(),
                    )
                } else if path.contains("/sync") {
                    (200, sync_body(cfg.other_admin))
                } else if path.ends_with("/leave") {
                    seen.lock().unwrap().push(format!(
                        "{method} leave {}",
                        path.split('/').nth(5).unwrap_or("?")
                    ));
                    match cfg.leave {
                        Leave::Hang => std::future::pending().await,
                        Leave::Status(200) => (200, "{}".to_owned()),
                        Leave::Status(403) => (403, DENIED.1.to_owned()),
                        Leave::Status(404) => (404, NO_MEMBER.1.to_owned()),
                        Leave::Status(c) => {
                            (c, r#"{"errcode":"M_UNKNOWN","error":"x"}"#.to_owned())
                        }
                    }
                } else if path.ends_with("/forget") {
                    seen.lock().unwrap().push(format!(
                        "{method} forget {}",
                        path.split('/').nth(5).unwrap_or("?")
                    ));
                    (200, "{}".to_owned())
                } else if path.contains("m.room.member") {
                    seen.lock().unwrap().push(format!(
                        "{method} member {}",
                        path.split('/').nth(5).unwrap_or("?")
                    ));
                    (cfg.member.0, cfg.member.1.to_owned())
                } else {
                    (
                        404,
                        r#"{"errcode":"M_UNRECOGNIZED","error":"x"}"#.to_owned(),
                    )
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (url, log)
}

async fn joined_client(url: &str) -> Client {
    let client = Client::builder()
        .homeserver_url(url)
        .sliding_sync_version_builder(VersionBuilder::None)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(ME, "x")
        .send()
        .await
        .unwrap();
    client.sync_once(SyncSettings::default()).await.unwrap();
    for id in [SPACE, SUB, ROOM] {
        assert_eq!(
            client
                .get_room(&SdkRoomId::parse(id).unwrap())
                .unwrap()
                .state(),
            RoomState::Joined
        );
    }
    client
}

fn scope(room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new(room),
    }
}

/// None = the SDK no longer has the room (forgotten).
fn state(client: &Client, room: &str) -> Option<RoomState> {
    client
        .get_room(&SdkRoomId::parse(room).unwrap())
        .map(|r| r.state())
}

/// Request log with short room names: `leave`, `member`, `forget` and the room they address.
fn calls(log: &Log) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .map(|c| {
            c.replace(SPACE, "SPACE")
                .replace(SUB, "SUB")
                .replace(ROOM, "ROOM")
        })
        .collect()
}

async fn run(
    cfg: Cfg,
    forget: bool,
) -> (Client, Log, (Option<bool>, Result<bool, OperationError>)) {
    let (url, log) = serve(cfg).await;
    let client = joined_client(&url).await;
    let result = leave_space(&client, AccountId::new(1), &scope(SPACE), forget).await;
    (client, log, result)
}

const OK200: Leave = Leave::Status(200);

#[tokio::test(flavor = "current_thread")]
async fn a_leave_then_forget_touches_only_the_space_in_order() {
    let (client, log, r) = run(
        Cfg {
            leave: OK200,
            member: OUT,
            other_admin: false,
        },
        true,
    )
    .await;
    assert_eq!(r, (Some(true), Ok(true)));
    assert_eq!(
        calls(&log),
        ["POST leave SPACE", "GET member SPACE", "POST forget SPACE"]
    );
    assert_eq!(state(&client, SPACE), None);
    // Sub-space and the plain room inside it stay joined.
    assert_eq!(
        (state(&client, SUB), state(&client, ROOM)),
        (Some(RoomState::Joined), Some(RoomState::Joined))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn forget_off_leaves_without_forgetting() {
    let (client, log, r) = run(
        Cfg {
            leave: OK200,
            member: OUT,
            other_admin: false,
        },
        false,
    )
    .await;
    assert_eq!(r, (Some(true), Ok(false)));
    assert_eq!(calls(&log), ["POST leave SPACE", "GET member SPACE"]);
    assert_eq!(state(&client, SPACE), Some(RoomState::Left));
}

#[tokio::test(flavor = "current_thread")]
async fn is_last_owner_is_false_when_another_admin_is_joined() {
    let (_, _, r) = run(
        Cfg {
            leave: OK200,
            member: OUT,
            other_admin: true,
        },
        false,
    )
    .await;
    assert_eq!(r, (Some(false), Ok(false)));
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_leave_with_membership_still_join_is_not_a_success_and_never_forgets() {
    let (client, log, r) = run(
        Cfg {
            leave: Leave::Status(403),
            member: JOIN,
            other_admin: false,
        },
        true,
    )
    .await;
    assert_eq!(r, (Some(true), Err(OperationError::Forbidden)));
    assert_eq!(calls(&log), ["POST leave SPACE", "GET member SPACE"]);
    assert_eq!(state(&client, SPACE), Some(RoomState::Joined));
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_leave_with_an_unreadable_membership_stays_forbidden() {
    let (client, log, r) = run(
        Cfg {
            leave: Leave::Status(403),
            member: DENIED,
            other_admin: false,
        },
        true,
    )
    .await;
    assert_eq!(r.1, Err(OperationError::Forbidden));
    assert_eq!(calls(&log), ["POST leave SPACE", "GET member SPACE"]);
    assert_eq!(state(&client, SPACE), Some(RoomState::Joined));
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_leave_already_out_elsewhere_is_ok_and_forget_is_refused_by_the_sdk() {
    for member in [OUT, NO_MEMBER] {
        let (client, log, r) = run(
            Cfg {
                leave: Leave::Status(403),
                member,
                other_admin: false,
            },
            true,
        )
        .await;
        // Local state stays Joined (vendored SDK), so Room::forget fails before any request: Ok(false), no POST forget.
        assert_eq!(r, (Some(true), Ok(false)), "member answer {member:?}");
        assert_eq!(calls(&log), ["POST leave SPACE", "GET member SPACE"]);
        assert_eq!(state(&client, SPACE), Some(RoomState::Joined));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_200_leave_that_the_server_still_shows_as_join_is_forbidden_and_not_forgotten() {
    let (_, log, r) = run(
        Cfg {
            leave: OK200,
            member: JOIN,
            other_admin: false,
        },
        true,
    )
    .await;
    assert_eq!(r.1, Err(OperationError::Forbidden));
    assert_eq!(calls(&log), ["POST leave SPACE", "GET member SPACE"]);
}

#[tokio::test(flavor = "current_thread")]
async fn other_leave_outcomes_are_classified_without_recheck_or_forget() {
    for (status, want) in [
        (404, OperationError::Unavailable),
        (400, OperationError::Transport),
    ] {
        let (client, log, r) = run(
            Cfg {
                leave: Leave::Status(status),
                member: OUT,
                other_admin: false,
            },
            true,
        )
        .await;
        assert_eq!(r.1, Err(want), "status {status}");
        assert_eq!(calls(&log), ["POST leave SPACE"], "status {status}");
        assert_eq!(state(&client, SPACE), Some(RoomState::Joined));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_hanging_leave_times_out_as_transport_and_nothing_follows() {
    let (url, log) = serve(Cfg {
        leave: Leave::Hang,
        member: OUT,
        other_admin: false,
    })
    .await;
    let client = joined_client(&url).await;
    let space = scope(SPACE);
    let fut = leave_space(&client, AccountId::new(1), &space, true);
    tokio::pin!(fut);
    // Real clock until the server has the /leave (a paused clock would time out the earlier calls on idle I/O),
    // then the 30 s limit elapses on the paused clock.
    while !calls(&log).iter().any(|c| c.contains("leave")) {
        tokio::select! { _ = &mut fut => panic!("finished before /leave"), _ = tokio::task::yield_now() => {} }
    }
    tokio::time::pause();
    let r = fut.await;
    assert_eq!(r, (Some(true), Err(OperationError::Transport)));
    assert_eq!(calls(&log), ["POST leave SPACE"]);
}

#[tokio::test(flavor = "current_thread")]
async fn an_unknown_space_or_foreign_account_makes_no_leave_call() {
    let (url, log) = serve(Cfg {
        leave: OK200,
        member: OUT,
        other_admin: false,
    })
    .await;
    let client = joined_client(&url).await;
    // A plain room is not a node of the space graph.
    assert_eq!(
        leave_space(&client, AccountId::new(1), &scope(ROOM), true).await,
        (None, Err(OperationError::Unavailable))
    );
    assert_eq!(
        leave_space(&client, AccountId::new(1), &scope("!nope:fake.test"), true).await,
        (None, Err(OperationError::Unavailable))
    );
    assert_eq!(
        leave_space(&client, AccountId::new(2), &scope(SPACE), true).await,
        (None, Err(OperationError::WrongScope))
    );
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));
}
