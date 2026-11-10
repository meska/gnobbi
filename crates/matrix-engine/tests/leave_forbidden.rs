//! A leave refused by the server (403) must leave the room Joined in the local store: the room list
//! projection only emits Remove for non-Joined rooms, so the row stays and the sidebar can show the error.
//! In-process loopback homeserver (plain /sync, no Docker). If the server then says we are not in the room
//! (state read answers 404 / non-join), the 403 counts as a completed leave.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    operations::{OperationError, leave_room},
    sdk::{
        Client, RoomState,
        config::SyncSettings,
        ruma::RoomId as SdkRoomId,
        sliding_sync::VersionBuilder,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const ROOM: &str = "!r:fake.test";

/// `member`: body of our own m.room.member state (None = 404).
async fn serve(member: Option<&'static str>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("").to_owned();
                let (status, body) = if path.contains("/login") {
                    (200, r#"{"user_id":"@a:fake.test","access_token":"t","device_id":"D"}"#.to_owned())
                } else if path.contains("/versions") {
                    (200, r#"{"versions":["v1.11"],"unstable_features":{}}"#.to_owned())
                } else if path.contains("/sync") {
                    (200, format!(r#"{{"next_batch":"1","rooms":{{"join":{{"{ROOM}":{{}}}}}}}}"#))
                } else if path.contains("/leave") {
                    (403, r#"{"errcode":"M_FORBIDDEN","error":"no"}"#.to_owned())
                } else if path.contains("m.room.member") {
                    match member {
                        Some(b) => (200, b.to_owned()),
                        None => (404, r#"{"errcode":"M_NOT_FOUND","error":"x"}"#.to_owned()),
                    }
                } else {
                    (404, r#"{"errcode":"M_UNRECOGNIZED","error":"x"}"#.to_owned())
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
    url
}

async fn joined_client(url: &str) -> Client {
    let client = Client::builder()
        .homeserver_url(url)
        .sliding_sync_version_builder(VersionBuilder::None)
        .build()
        .await
        .unwrap();
    client.matrix_auth().login_username("@a:fake.test", "x").send().await.unwrap();
    client.sync_once(SyncSettings::default()).await.unwrap();
    let id = SdkRoomId::parse(ROOM).unwrap();
    assert_eq!(client.get_room(&id).unwrap().state(), RoomState::Joined);
    client
}

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(ROOM) }
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_leave_keeps_the_room_joined_locally() {
    let url = serve(Some(r#"{"membership":"join"}"#)).await;
    let client = joined_client(&url).await;
    let r = leave_room(&client, AccountId::new(1), &scope()).await;
    assert_eq!(r, Err(OperationError::Forbidden));
    let id = SdkRoomId::parse(ROOM).unwrap();
    assert_eq!(client.get_room(&id).unwrap().state(), RoomState::Joined);
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_leave_when_the_server_says_we_are_out_is_a_completed_leave() {
    let url = serve(Some(r#"{"membership":"leave"}"#)).await;
    let client = joined_client(&url).await;
    assert_eq!(leave_room(&client, AccountId::new(1), &scope()).await, Ok(()));
}
