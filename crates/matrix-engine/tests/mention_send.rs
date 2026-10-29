//! The app's own send path with a picked mention against a disposable homeserver, driven by
//! the mention_send script: the member list offered to A, the sent event (m.mentions, matrix.to link,
//! plain body) and B's notification. Two synthetic accounts; the fixture config path comes from
//! the environment.
// The queue futures nest deeper than the default 128.
#![recursion_limit = "256"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    operations::members,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::{
                push::get_notifications::v3::Request as NotificationsRequest,
                room::{
                    create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
                    get_room_event::v3::Request as GetEventRequest,
                },
            },
            assign,
            serde::Raw,
        },
    },
    sending::{Mention, SendRequest, fresh_transaction_id, send_text},
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

#[test]
#[ignore = "requires the mention_send script"]
fn app_mention_reaches_bob_with_metadata_and_notification() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let login = |who: &'static str| {
            let (url, ca, config) = (url.clone(), ca.clone(), config.clone());
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
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
        };
        let (a, b) = (login("alice").await, login("bob").await);
        let bob = b.user_id().unwrap().to_owned();
        let mut creation = CreationContent::new();
        creation.federate = false;
        let created = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap();
        let room_id = created.room_id().to_owned();
        b.join_room_by_id(&room_id).await.unwrap();
        let scope = RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new(room_id.as_str()),
        };

        // The completion list: both joined members, with their user ids.
        let list = members(&a, AccountId::new(1), &scope).await.unwrap();
        let listed = list
            .iter()
            .find(|m| m.user_id == bob.as_str())
            .expect("bob not offered");
        assert!(
            list.iter()
                .any(|m| m.user_id == a.user_id().unwrap().as_str())
        );
        let label = format!("@{}", listed.name);

        // The app send path with the picked mention.
        let body = format!("ciao {label} <ok> & via");
        let event_id = send_text(
            &a,
            AccountId::new(1),
            &SendRequest {
                scope: scope.clone(),
                transaction_id: fresh_transaction_id(),
                body: body.clone(),
                attachment: None,
                mentions: vec![Mention {
                    user_id: bob.to_string(),
                    label: label.clone(),
                }],
            },
        )
        .await
        .unwrap();
        let raw = serde_json::to_value(
            &b.send(GetEventRequest::new(
                room_id.clone(),
                event_id.as_str().try_into().unwrap(),
            ))
            .await
            .unwrap()
            .event,
        )
        .unwrap();
        let content = &raw["content"];
        assert_eq!(content["body"], body);
        assert_eq!(
            content["m.mentions"]["user_ids"],
            serde_json::json!([bob.as_str()])
        );
        assert_eq!(content["format"], "org.matrix.custom.html");
        assert_eq!(
            content["formatted_body"],
            format!("ciao <a href=\"https://matrix.to/#/{bob}\">{label}</a> &lt;ok&gt; &amp; via")
        );
        println!("[mention] stored: {content}");

        // B is notified: the server lists the event with a notify action.
        let start = Instant::now();
        loop {
            let notifications = b.send(NotificationsRequest::new()).await.unwrap();
            let hit = notifications.notifications.iter().any(|n| {
                let v = serde_json::to_value(n).unwrap();
                v["event"]["event_id"] == event_id.as_str()
                    && v["actions"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == "notify"))
            });
            if hit {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "bob was not notified"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        println!("[mention] bob notified");
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
