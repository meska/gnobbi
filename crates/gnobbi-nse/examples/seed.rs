//! Test fixture (host only): two synthetic accounts on a TEST homeserver, three events for account A's store (tag `1:1:accounts/1`).
//! Room 1 (encrypted): message A can already decrypt, plus an edit of it. Room 2 (encrypted): message sent AFTER A's last sync
//! (the key is not in A's store). Room 3 (not encrypted): plain message.
//! usage: seed <out_dir> <homeserver> <userA> <passA> <userB> <passB> <store_pass>   -> prints fixture JSON (room/event ids, texts)
use matrix_sdk::{
    Client,
    config::SyncSettings,
    ruma::{
        OwnedRoomId,
        api::client::room::create_room::v3::Request as Create,
        events::{
            InitialStateEvent,
            room::{encryption::RoomEncryptionEventContent, message::RoomMessageEventContent},
        },
        serde::Raw,
    },
};
use matrix_sdk_store_encryption::StoreCipher;
use serde_json::json;
use std::{path::PathBuf, time::Duration};

async fn login(dir: &PathBuf, hs: &str, user: &str, pw: &str, store_pass: &str) -> Client {
    std::fs::create_dir_all(dir).unwrap();
    let c = Client::builder()
        .homeserver_url(hs)
        .sqlite_store(dir, Some(store_pass))
        .build()
        .await
        .unwrap();
    c.matrix_auth()
        .login_username(user, pw)
        .initial_device_display_name("gnobbi-nse-test")
        .await
        .unwrap();
    // Same record as the app's `session.sealed` (matrix-engine persistence.rs): version 1, cipher export + encrypted payload.
    let cipher = StoreCipher::new().unwrap();
    let payload = cipher.encrypt_value(&json!({"account": 1, "value": {"homeserver": hs, "session": c.matrix_auth().session().unwrap()}})).unwrap();
    let record =
        json!({"version": 1, "cipher": cipher.export(store_pass).unwrap(), "payload": payload});
    std::fs::write(
        dir.join("session.sealed"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    c
}

async fn sync(c: &Client) {
    c.sync_once(SyncSettings::new().timeout(Duration::from_millis(500)))
        .await
        .unwrap();
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(&a[1]);
    let ca = login(
        &out.join("accounts/1/account-1"),
        &a[2],
        &a[3],
        &a[4],
        &a[7],
    )
    .await;
    let cb = login(&out.join("b"), &a[2], &a[5], &a[6], &a[7]).await;
    sync(&ca).await;
    sync(&cb).await;
    let a_id = ca.user_id().unwrap().to_owned();
    let mut rooms: Vec<OwnedRoomId> = vec![];
    for (name, encrypted) in [
        ("NSE room one", true),
        ("NSE room two", true),
        ("NSE room three", false),
    ] {
        let mut req = Create::new();
        req.name = Some(name.into());
        req.invite = vec![a_id.clone()];
        if encrypted {
            req.initial_state = vec![
                Raw::new(&InitialStateEvent::with_empty_state_key(
                    RoomEncryptionEventContent::with_recommended_defaults(),
                ))
                .unwrap()
                .cast_unchecked(),
            ];
        }
        rooms.push(cb.create_room(req).await.unwrap().room_id().to_owned());
    }
    sync(&ca).await;
    for r in &rooms {
        ca.get_room(r).expect("invite").join().await.unwrap();
    }
    sync(&ca).await;
    sync(&cb).await;
    sync(&cb).await;
    let send = |i: usize, body: &'static str| {
        let (cb, room) = (cb.clone(), rooms[i].clone());
        async move {
            cb.get_room(&room)
                .unwrap()
                .send(RoomMessageEventContent::text_plain(body))
                .await
                .unwrap()
                .response
                .event_id
        }
    };
    let (e1, e3) = (
        send(0, "nse test message one").await,
        send(2, "nse test message three").await,
    );
    sync(&ca).await; // A now holds the key of room 1 and sees room 3
    let edit = cb
        .get_room(&rooms[0])
        .unwrap()
        .send_raw("m.room.message", json!({"msgtype": "m.text", "body": "* edited", "m.new_content": {"msgtype": "m.text", "body": "edited"}, "m.relates_to": {"rel_type": "m.replace", "event_id": e1}}))
        .await
        .unwrap()
        .response
        .event_id;
    sync(&ca).await;
    sync(&cb).await;
    let e2 = send(1, "nse test message two").await; // after A's last sync
    println!(
        "{}",
        json!({"room1": rooms[0], "event1": e1, "body1": "nse test message one", "edit1": edit,
               "room2": rooms[1], "event2": e2, "body2": "nse test message two",
               "room3": rooms[2], "event3": e3, "body3": "nse test message three"})
    );
}
