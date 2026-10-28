//! Formatted text through the app's own paths against a disposable homeserver, driven by
//! the format_send script: A sends markdown (alone and with a picked mention) with `send_text`, the
//! stored event carries body + `formatted_body`, and B's history rows show the rendered text and
//! styles (hostile markup dropped). Two synthetic accounts; the fixture config path comes from
//! the environment.
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    operations::members,
    ViewRequests,
    room_sync::RoomSync,
    rich_text::Style,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::{
                room::{
                    create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
                    get_room_event::v3::Request as GetEventRequest,
                },
            },
            assign,
            serde::Raw,
            OwnedRoomId,
        },
    },
    sending::{Mention, SendRequest, fresh_transaction_id, send_text},
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

#[test]
#[ignore = "requires the format_send script"]
fn formatted_text_is_sent_and_rendered_by_the_receiver() {
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

        let list = members(&a, AccountId::new(1), &scope).await.unwrap();
        let listed = list.iter().find(|m| m.user_id == bob.as_str()).expect("bob not offered");
        let label = format!("@{}", listed.name);

        let send = |body: String, mentions: Vec<Mention>| {
            let (a, scope) = (a.clone(), scope.clone());
            async move {
                send_text(
                    &a,
                    AccountId::new(1),
                    &SendRequest {
                        scope,
                        transaction_id: fresh_transaction_id(),
                        body,
                        attachment: None,
                        mentions,
                    },
                )
                .await
                .unwrap()
            }
        };
        let rich = "**forte** _corsivo_ ~~barrato~~ `codice`\n- uno\n- due\n\n> cita [sito](https://example.org/p)\n\n<script>alert(1)</script>".to_owned();
        let plain = send("solo testo, niente markup".into(), vec![]).await;
        let first = send(rich.clone(), vec![]).await;
        let mentioned_body = format!("ciao {label} **ok**");
        let second = send(
            mentioned_body.clone(),
            vec![Mention { user_id: bob.to_string(), label: label.clone() }],
        )
        .await;

        // A hostile sender: raw HTML straight into the room, bypassing our markdown.
        let hostile = a
            .get_room(&room_id)
            .unwrap()
            .send(matrix_engine::sdk::ruma::events::room::message::RoomMessageEventContent::text_html(
                "hostile",
                "<b>ok</b><script>alert(1)</script><a href=\"javascript:alert(2)\">x</a><img src=\"https://e.org/i.png\" onerror=\"alert(3)\"><mx-reply>old</mx-reply>",
            ))
            .await
            .unwrap()
            .response
            .event_id;

        // The stored events: typed body, HTML, mentions.
        let raw = |id: matrix_domain::EventId| {
            let (b, room_id) = (b.clone(), room_id.clone());
            async move {
                serde_json::to_value(
                    &b.send(GetEventRequest::new(room_id, id.as_str().try_into().unwrap()))
                        .await
                        .unwrap()
                        .event,
                )
                .unwrap()["content"]
                    .clone()
            }
        };
        let content = raw(plain.clone()).await;
        assert_eq!(content["body"], "solo testo, niente markup");
        assert!(content.get("formatted_body").is_none(), "{content}");
        let content = raw(first.clone()).await;
        assert_eq!(content["body"], rich);
        assert_eq!(content["format"], "org.matrix.custom.html");
        let html = content["formatted_body"].as_str().unwrap();
        for tag in ["<strong>forte</strong>", "<em>corsivo</em>", "<del>barrato</del>", "<code>codice</code>", "<li>uno</li>", "<blockquote>", "<a href=\"https://example.org/p\">sito</a>"] {
            assert!(html.contains(tag), "{tag} missing in {html}");
        }
        assert!(!html.contains("<script"), "typed HTML must be escaped: {html}");
        let content = raw(second.clone()).await;
        assert_eq!(content["body"], mentioned_body);
        assert_eq!(content["m.mentions"]["user_ids"], serde_json::json!([bob.as_str()]));
        assert!(content["formatted_body"].as_str().unwrap().contains(&format!("<a href=\"https://matrix.to/#/{bob}\">{label}</a> <strong>ok</strong>")));
        println!("[format] stored events checked");

        // What B draws: history rows with display text and styles.
        let mut views = ViewRequests::default();
        let sync = RoomSync::from_client(AccountId::new(2), b.clone());
        let start = Instant::now();
        let page = loop {
            let page = sync.history_page(views.start(scope_of(2, &room_id)).unwrap(), None).await.unwrap();
            if page.events.len() >= 4 {
                break page;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "B did not get the events");
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let row = |id: &matrix_domain::EventId| page.events.iter().find(|e| e.event_id == *id).unwrap();
        assert_eq!(row(&plain).body, "solo testo, niente markup");
        assert!(row(&plain).styles.is_empty());
        let rendered = row(&first);
        assert_eq!(
            rendered.body,
            "forte corsivo barrato codice\n\n• uno\n• due\n▎ cita sito (https://example.org/p)\n<script>alert(1)</script>"
        );
        let styled = |style| -> Vec<&str> {
            rendered.styles.iter().filter(|s| s.style == style).map(|s| &rendered.body[s.range.clone()]).collect()
        };
        assert_eq!(styled(Style::Bold), ["forte"]);
        assert_eq!(styled(Style::Italic), ["corsivo"]);
        assert_eq!(styled(Style::Strike), ["barrato"]);
        assert_eq!(styled(Style::Code), ["codice"]);
        assert_eq!(row(&second).body, format!("ciao {label} ok"));
        assert_eq!(
            row(&second).styles.iter().filter(|s| s.style == Style::Bold).map(|s| &row(&second).body[s.range.clone()]).collect::<Vec<_>>(),
            ["ok"]
        );
        let hostile = page.events.iter().find(|e| e.event_id.as_str() == hostile.as_str()).unwrap();
        assert_eq!(hostile.body, "okx");
        assert_eq!(hostile.styles.len(), 1);
        println!("[format] receiver rendering checked");
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}

fn scope_of(account: u64, room_id: &OwnedRoomId) -> RoomScope {
    RoomScope { account_id: AccountId::new(account), room_id: RoomId::new(room_id.as_str()) }
}
