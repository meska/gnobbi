//! Mentions, formatted text and emoji probe against a disposable homeserver
//! (read-only verification). Driven by the mentions_format_probe script inside
//! a private namespace. Uses only official matrix-sdk 0.19.1 APIs: `Room::send`
//! for an m.room.message with `formatted_body` (org.matrix.custom.html) and
//! intentional `m.mentions`, for an @room mention and for an m.reaction with a
//! unicode emoji key; raw read-back via `get_room_event` (servers must store
//! the content verbatim), receive-side sanitization with the ruma sanitizer,
//! default push rules via `get_pushrules_all` and `/notifications` to check
//! that an intentional user mention triggers the server push rules. Two
//! synthetic accounts only; no secrets here (fixture config via env).
#![cfg(feature = "sdk")]
use matrix_engine::sdk::{
    Client,
    reqwest::Certificate,
    ruma::{
        OwnedEventId, OwnedRoomId,
        api::client::{
            push::{
                get_notifications::v3::Request as NotificationsRequest,
                get_pushrules_all::v3::Request as PushRulesRequest,
            },
            room::{
                create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
                get_room_event::v3::Request as GetEventRequest,
            },
        },
        assign,
        events::{
            Mentions,
            reaction::ReactionEventContent,
            relation::Annotation,
            room::message::{MessageType, RoomMessageEventContent},
        },
        html::{HtmlSanitizerMode, RemoveReplyFallback},
        serde::Raw,
    },
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn raw_event(
    client: &Client,
    room: &OwnedRoomId,
    event_id: &OwnedEventId,
) -> serde_json::Value {
    let response = client
        .send(GetEventRequest::new(room.clone(), event_id.clone()))
        .await
        .unwrap();
    serde_json::to_value(&response.event).unwrap()
}

#[test]
#[ignore = "requires the mentions_format_probe script"]
fn mentions_formatting_emoji_round_trip_on_disposable_server() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let kind = config["kind"].as_str().unwrap().to_owned();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let build = || {
            let (url, ca) = (url.clone(), ca.clone());
            async move {
                Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .build()
                    .await
                    .unwrap()
            }
        };
        let a = build().await;
        let b = build().await;
        a.matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        b.matrix_auth()
            .login_username(
                config["bob_mxid"].as_str().unwrap(),
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let bob = b.user_id().unwrap().to_owned();
        println!("[probe] {kind}: logged in alice and {bob}");

        let mut creation = CreationContent::new();
        creation.federate = false;
        let created = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                is_direct: false,
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap();
        let room_id = created.room_id().to_owned();
        b.join_room_by_id(&room_id).await.unwrap();
        let room = a.get_room(&room_id).unwrap();
        println!("[probe] room {room_id} created, bob joined");

        // 1. Formatted mention: HTML body with a matrix.to link, allowed tags,
        //    a disallowed <script> tag and an emoji, plus intentional
        //    m.mentions for bob.
        let formatted = format!(
            "<strong>ping</strong> <a href=\"https://matrix.to/#/{bob}\">{bob}</a> \
             😀 <del>old</del><script>alert(1)</script>"
        );
        let plain_body = format!("ping {bob} 😀 old");
        let content = RoomMessageEventContent::text_html(&plain_body, formatted)
            .add_mentions(Mentions::with_user_ids([bob.clone()]));
        let sent = room
            .send(content)
            .with_transaction_id("p254-1".into())
            .await
            .unwrap();
        let mention_event: OwnedEventId = sent.response.event_id.clone();
        let raw = raw_event(&b, &room_id, &mention_event).await;
        assert_eq!(raw["type"], "m.room.message");
        let stored = &raw["content"];
        assert_eq!(stored["format"], "org.matrix.custom.html");
        let stored_html = stored["formatted_body"].as_str().unwrap();
        assert!(
            stored_html.contains("matrix.to"),
            "no matrix.to link: {stored_html}"
        );
        assert!(
            stored_html.contains("<script>alert(1)</script>"),
            "server altered the formatted_body: {stored_html}"
        );
        let mentioned: Vec<&str> = stored["m.mentions"]["user_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(
            mentioned.contains(&bob.as_str()),
            "m.mentions: {mentioned:?}"
        );
        println!("[probe] formatted mention stored verbatim, m.mentions={mentioned:?}");

        // 2. SDK receive path: deserialize and sanitize (what matrix-sdk-ui
        //    does with DEFAULT_SANITIZER_MODE = Compat).
        let mut received: RoomMessageEventContent = serde_json::from_value(stored.clone()).unwrap();
        assert!(
            received
                .mentions
                .as_ref()
                .is_some_and(|m| m.user_ids.contains(&bob)),
            "ruma did not deserialize m.mentions"
        );
        received.sanitize(HtmlSanitizerMode::Compat, RemoveReplyFallback::No);
        let MessageType::Text(text) = &received.msgtype else {
            panic!("not a text message");
        };
        let sanitized = text.formatted.as_ref().unwrap().body.clone();
        assert!(
            !sanitized.contains("<script"),
            "script survived sanitize: {sanitized}"
        );
        assert!(
            sanitized.contains("<strong>"),
            "allowed tag stripped: {sanitized}"
        );
        assert!(
            sanitized.contains("matrix.to"),
            "mention link stripped: {sanitized}"
        );
        println!("[probe] sanitized html keeps allowed tags and drops <script>");

        // 3. Emoji in a plain body round-trips.
        let emoji_body = "😀🙌 hello";
        let sent = room
            .send(RoomMessageEventContent::text_plain(emoji_body))
            .with_transaction_id("p254-2".into())
            .await
            .unwrap();
        let raw = raw_event(&b, &room_id, &sent.response.event_id).await;
        assert_eq!(raw["content"]["body"], emoji_body);
        println!("[probe] unicode emoji body round-trips");

        // 4. @room intentional mention.
        let content = RoomMessageEventContent::text_html("@room check", "<b>@room</b> check")
            .add_mentions(Mentions::with_room_mention());
        let sent = room
            .send(content)
            .with_transaction_id("p254-3".into())
            .await
            .unwrap();
        let raw = raw_event(&b, &room_id, &sent.response.event_id).await;
        assert_eq!(
            raw["content"]["m.mentions"]["room"], true,
            "@room mention lost"
        );
        println!("[probe] @room mention (m.mentions.room=true) stored");

        // 5. Reaction with a unicode emoji key via the SDK event content.
        let reaction =
            ReactionEventContent::new(Annotation::new(mention_event.clone(), "👍".to_owned()));
        let sent = room
            .send(reaction)
            .with_transaction_id("p254-4".into())
            .await
            .unwrap();
        let raw = raw_event(&b, &room_id, &sent.response.event_id).await;
        assert_eq!(raw["type"], "m.reaction");
        assert_eq!(raw["content"]["m.relates_to"]["rel_type"], "m.annotation");
        assert_eq!(raw["content"]["m.relates_to"]["key"], "👍");
        assert_eq!(
            raw["content"]["m.relates_to"]["event_id"],
            mention_event.as_str()
        );
        println!("[probe] emoji reaction (m.reaction, key 👍) round-trips");

        // 6. Default push rules mention coverage.
        let rules = b.send(PushRulesRequest::new()).await.unwrap();
        let mut ids: Vec<String> = Vec::new();
        let g = &rules.global;
        for r in g.override_.iter() {
            ids.push(r.rule_id.clone());
        }
        for r in g.content.iter() {
            ids.push(r.rule_id.clone());
        }
        for r in g.room.iter() {
            ids.push(r.rule_id.to_string());
        }
        for r in g.sender.iter() {
            ids.push(r.rule_id.to_string());
        }
        for r in g.underride.iter() {
            ids.push(r.rule_id.clone());
        }
        println!("[probe] push rules: {ids:?}");
        let has_user = ids.iter().any(|i| i == ".m.rule.is_user_mention")
            || ids.iter().any(|i| i == ".m.rule.contains_display_name");
        let has_room = ids.iter().any(|i| i == ".m.rule.is_room_mention")
            || ids.iter().any(|i| i == ".m.rule.roomnotif");
        assert!(has_user, "no user-mention push rule in {ids:?}");
        assert!(has_room, "no room-mention push rule in {ids:?}");

        // 7. The intentional mention must actually trigger a notification
        //    for bob (/notifications lists push-matched events).
        let found = {
            let start = Instant::now();
            loop {
                let notifs = b.send(NotificationsRequest::new()).await;
                match notifs {
                    Ok(notifs) => {
                        let hit = notifs.notifications.iter().any(|n| {
                            serde_json::to_value(&n.event)
                                .ok()
                                .and_then(|v| v["event_id"].as_str().map(str::to_owned))
                                .is_some_and(|id| id == mention_event.as_str())
                        });
                        if hit {
                            println!(
                                "[probe] /notifications contains the mention event ({}/{} entries)",
                                notifs.notifications.len(),
                                notifs.notifications.len()
                            );
                            break true;
                        }
                        if start.elapsed() > Duration::from_secs(20) {
                            println!("[probe] /notifications has no mention event");
                            break false;
                        }
                    }
                    Err(error) => {
                        println!("[probe] /notifications error: {error}");
                        break false;
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        };
        if kind == "synapse" {
            assert!(found, "synapse /notifications did not list the mention");
        }

        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
