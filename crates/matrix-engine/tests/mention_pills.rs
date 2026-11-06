//! Mention pills received and sent, on a disposable homeserver (Synapse or Tuwunel): m.mentions + matrix.to
//! links to a user, «@room» and a room, plus the fallback table of the MATRIX_MENZIONI_UI notes D4
//! (not a member, link without m.mentions, ambiguous name, me and another in one line, malformed
//! formatted_body, event link, «@name» without link, «@room» without m.mentions.room).
//! alice is "me"; bob and carol both call themselves «Sam». Synthetic accounts only.
//! Driver: PROBE_TARGET=mention_pills PROBE_TEST=mention_pills_received_and_sent PROBE_SERVER=synapse|tuwunel
//! python3 the timeline_probe script (via the ct script).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    ViewRequests,
    history::HistoryEvent,
    receipts::ReadVisibility,
    rich_text::MentionKind,
    room_sync::RoomSync,
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{Mentions, room::message::RoomMessageEventContent},
            serde::Raw,
            OwnedEventId, OwnedRoomId, OwnedUserId,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Mention, SendRequest, fresh_transaction_id, send_text},
};
use std::time::{Duration, Instant};

const HS: &str = "matrix-desktop.test";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn build(config: &serde_json::Value, who: &str, name: Option<&str>) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    c.matrix_auth()
        .login_username(format!("@{who}:{HS}"), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
    if let Some(n) = name {
        c.account().set_display_name(Some(n)).await.unwrap();
    }
    c
}

async fn room_of(c: &Client, id: &OwnedRoomId) -> Room {
    let started = Instant::now();
    loop {
        if let Some(r) = c.get_room(id) {
            return r;
        }
        assert!(started.elapsed() < Duration::from_secs(40), "room not synced");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// «text» of each pill and what it stands for: me, other:<id>, @room, room.
fn pills(e: &HistoryEvent) -> Vec<(String, String)> {
    e.mentions
        .iter()
        .map(|m| {
            let kind = match &m.kind {
                MentionKind::User { sender, me: true } => format!("me:{}", sender.id),
                MentionKind::User { sender, me: false } => format!("other:{}", sender.id),
                MentionKind::AtRoom => "@room".to_owned(),
                MentionKind::Room => "room".to_owned(),
            };
            (e.body[m.range.clone()].to_owned(), kind)
        })
        .collect()
}

/// The events of the room, by event id, once `want` of them are in (waits for the sync).
async fn events(sync: &RoomSync, views: &mut ViewRequests, scope: &RoomScope, want: &[String]) -> Vec<HistoryEvent> {
    let hidden = ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
    let started = Instant::now();
    loop {
        let (page, _) = sync.open_history(views.start(scope.clone()).unwrap(), hidden).await.unwrap();
        let seen: Vec<_> = page.events.iter().filter(|e| want.iter().any(|w| w == e.event_id.as_str())).collect();
        if seen.len() == want.len() {
            return page.events;
        }
        assert!(started.elapsed() < Duration::from_secs(40), "events missing: {} of {}", seen.len(), want.len());
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn find<'a>(all: &'a [HistoryEvent], id: &str) -> &'a HistoryEvent {
    all.iter().find(|e| e.event_id.as_str() == id).unwrap_or_else(|| panic!("event {id} not read"))
}

fn link(id: &str, label: &str) -> String {
    format!("<a href=\"https://matrix.to/#/{id}\">{label}</a>")
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn mention_pills_received_and_sent() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = build(&config, "alice", Some("Alice QA")).await;
        let b = build(&config, "bob", Some("Sam")).await;
        let c = build(&config, "carol", Some("Sam")).await;
        let (alice, bob, carol): (OwnedUserId, OwnedUserId, OwnedUserId) =
            (a.user_id().unwrap().into(), b.user_id().unwrap().into(), c.user_id().unwrap().into());
        let mut creation = CreationContent::new();
        creation.federate = false;
        let room_id = a
            .create_room(assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone(), carol.clone()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap()
            .room_id()
            .to_owned();
        b.join_room_by_id(&room_id).await.unwrap();
        c.join_room_by_id(&room_id).await.unwrap();
        let services: Vec<_> = {
            let mut v = Vec::new();
            for client in [&a, &b, &c] {
                let s = SyncService::builder(client.clone()).build().await.unwrap();
                s.start().await;
                v.push(s);
            }
            v
        };
        let (ra, rb) = (room_of(&a, &room_id).await, room_of(&b, &room_id).await);
        let sync = RoomSync::from_client(AccountId::new(1), a.clone());
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(room_id.as_str()) };
        let mut views = ViewRequests::default();

        // Received: bob writes, alice reads. (id, plain body, html, mentions)
        let ghost = format!("@ghost:{HS}");
        let room_link = format!("#qa:{HS}");
        let send = |body: &str, html: Option<String>, mentions: Option<Mentions>| {
            let mut content = match html {
                Some(h) => RoomMessageEventContent::text_html(body, h),
                None => RoomMessageEventContent::text_plain(body),
            };
            if let Some(m) = mentions {
                content = content.add_mentions(m);
            }
            let rb = rb.clone();
            async move { rb.send(content).await.unwrap().response.event_id.to_string() }
        };
        let user = |id: &OwnedUserId| Mentions::with_user_ids([id.clone()]);
        let mut sent = Vec::new();
        let me_only = send("ciao Alice QA", Some(format!("ciao {}", link(alice.as_str(), "Alice QA"))), Some(user(&alice))).await;
        let me_and_other = send(
            "Alice QA e Sam",
            Some(format!("{} e {}", link(alice.as_str(), "Alice QA"), link(carol.as_str(), "Sam"))),
            Some(Mentions::with_user_ids([alice.clone(), carol.clone()])),
        )
        .await;
        let at_room = send("@room avviso", Some("@room avviso".into()), Some(assign!(Mentions::new(), { room: true }))).await;
        let at_room_plain = send("@room senza flag", Some("@room senza flag".into()), None).await;
        let room = send("vieni in #qa", Some(format!("vieni in {}", link(&room_link, "#qa"))), None).await;
        let not_member = send("scrivi a Fantasma", Some(format!("scrivi a {}", link(&ghost, "Fantasma"))), Some(Mentions::with_user_ids([ghost.parse().unwrap()]))).await;
        let no_mentions = send("il capo", Some(link(alice.as_str(), "il capo")), None).await;
        let event_link = send("vedi qui", Some(format!("vedi {}", link(&format!("{room_id}/$abcdef:{HS}"), "qui"))), None).await;
        let at_name = send("ciao @Alice QA", None, Some(user(&alice))).await;
        let malformed = send(
            "rotto Alice QA",
            Some(format!("<b>rotto <a href=\"https://matrix.to/#/{alice}\">Alice QA <i>")),
            Some(user(&alice)),
        )
        .await;
        let garbage = send("solo testo", Some("<<a href=>>&&;<a href=\"https://matrix.to/#/@\">".into()), None).await;
        let at_room_no_html = send("@room testo semplice", None, Some(assign!(Mentions::new(), { room: true }))).await;
        sent.extend([&me_only, &me_and_other, &at_room, &at_room_plain, &room, &not_member, &no_mentions, &event_link, &at_name, &malformed, &garbage, &at_room_no_html].map(|s| s.clone()));
        let bubble = send("nome in bolla", None, None).await; // bob's own bubble name, the reference for the pill name
        sent.push(bubble.clone());

        // Sent from the composer path: alice mentions bob (a picked member) and herself; types «@room» and a room link.
        let compose = |body: &str, mentions: Vec<Mention>| {
            let (a, scope) = (a.clone(), scope.clone());
            let body = body.to_owned();
            async move {
                send_text(&a, AccountId::new(1), &SendRequest { scope, transaction_id: fresh_transaction_id(), body, attachment: None, poll: None, mentions })
                    .await
                    .unwrap()
                    .as_str()
                    .to_owned()
            }
        };
        let own_other = compose("ciao @Sam", vec![Mention { user_id: bob.to_string(), label: "@Sam".into() }]).await;
        let own_self = compose("nota per me @Alice QA", vec![Mention { user_id: alice.to_string(), label: "@Alice QA".into() }]).await;
        let own_at_room = compose("@room tutti", vec![]).await;
        let own_room_link = compose(&format!("vedi [la stanza](https://matrix.to/#/{room_link})"), vec![]).await;
        sent.extend([own_other.clone(), own_self.clone(), own_at_room.clone(), own_room_link.clone()]);

        let all = events(&sync, &mut views, &scope, &sent).await;
        let bob_name = find(&all, &bubble).sender.name.clone().unwrap();
        let bob_id = bob.to_string();
        let carol_id = carol.to_string();
        let alice_id = alice.to_string();
        assert!(bob_name.contains(&bob_id), "two «Sam» must read «Sam (@id)» on the bubble: {bob_name:?}");
        let mut report = Vec::new();
        let mut check = |name: &str, ok: bool, detail: String| {
            eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
            if !ok {
                report.push(name.to_owned());
            }
        };

        let e = find(&all, &me_only);
        check("user_me", pills(e) == [("Alice QA".to_owned(), format!("me:{alice_id}"))] && e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        let e = find(&all, &me_and_other);
        let sam_carol = e.mentions.iter().find_map(|m| match &m.kind {
            MentionKind::User { sender, me: false } => sender.name.clone(),
            _ => None,
        });
        check("me_and_other_one_line", e.mentions.len() == 2 && matches!(e.mentions[0].kind, MentionKind::User { me: true, .. }) && matches!(e.mentions[1].kind, MentionKind::User { me: false, .. }) && e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        let carol_pill = pills(e).into_iter().nth(1).map(|p| p.0);
        check("ambiguous_name_is_name_and_id", carol_pill.as_deref().is_some_and(|t| t.contains(&carol_id) && t.starts_with("Sam")) && sam_carol == carol_pill, format!("{carol_pill:?} sender_name={sam_carol:?}"));
        // The pill of bob would carry exactly his bubble name.
        check("bubble_name_differs_per_person", bob_name != carol_pill.clone().unwrap_or_default(), format!("{bob_name:?}"));
        let e = find(&all, &at_room);
        // bob has power 0: the server's room-notification rule needs 50, so the pill shows and the row stays plain.
        check("at_room_with_flag_member_pill_not_highlighted", pills(e) == [("@room".into(), "@room".into())] && !e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        let e = find(&all, &at_room_plain);
        check("at_room_without_flag_is_text", pills(e).is_empty(), format!("{:?}", pills(e)));
        let e = find(&all, &at_room_no_html);
        check("at_room_plain_body_with_flag", pills(e) == [("@room".into(), "@room".into())], format!("{:?}", pills(e)));
        let e = find(&all, &room);
        check("room_link", pills(e).len() == 1 && pills(e)[0].1 == "room" && !e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        let e = find(&all, &not_member);
        check("user_not_in_room_is_text", pills(e).is_empty() && e.body.contains("Fantasma"), format!("{:?} body={:?}", pills(e), e.body));
        let e = find(&all, &no_mentions);
        check("link_without_m_mentions_is_pill_not_highlighted", pills(e).len() == 1 && !e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        let e = find(&all, &event_link);
        check("event_link_is_text", pills(e).is_empty() && e.body.contains("qui"), format!("{:?} body={:?}", pills(e), e.body));
        let e = find(&all, &at_name);
        check("at_name_without_link_is_text", pills(e).is_empty() && e.body.contains("@Alice QA"), format!("{:?}", pills(e)));
        let e = find(&all, &malformed);
        check("malformed_html_keeps_row", e.body.contains("rotto"), format!("{:?} -> {:?}", pills(e), e.body));
        let e = find(&all, &garbage);
        check("garbage_html_keeps_row_no_pill", pills(e).is_empty() && !e.body.is_empty(), format!("body={:?}", e.body));

        // Sent from the composer (read back as alice's own rows).
        let e = find(&all, &own_other);
        check("composer_user_pill_other_named_as_bubble", e.outgoing && pills(e) == [(bob_name.clone(), format!("other:{bob_id}"))], format!("{:?} body={:?}", pills(e), e.body));
        let e = find(&all, &own_self);
        check("composer_self_mention_never_highlighted", e.outgoing && !e.highlighted && pills(e).len() == 1, format!("{:?} hl={}", pills(e), e.highlighted));
        let e = find(&all, &own_at_room);
        check("composer_at_room_typed_is_text", e.outgoing && pills(e).is_empty(), format!("{:?}", pills(e)));
        let e = find(&all, &own_room_link);
        check("composer_room_link_pill", e.outgoing && pills(e).len() == 1 && pills(e)[0].1 == "room", format!("{:?} body={:?}", pills(e), e.body));
        // What went on the wire for the composer's user mention.
        let raw = ra.event(&OwnedEventId::try_from(own_other.as_str()).unwrap(), None).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.raw().json().get()).unwrap();
        check("composer_wire_m_mentions_and_link", v["content"]["m.mentions"]["user_ids"] == serde_json::json!([bob_id]) && v["content"]["formatted_body"].as_str().unwrap().contains(&format!("https://matrix.to/#/{bob_id}")), v["content"].to_string());
        let raw = ra.event(&OwnedEventId::try_from(own_at_room.as_str()).unwrap(), None).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(raw.raw().json().get()).unwrap();
        check("composer_at_room_wire_has_no_room_flag", v["content"]["m.mentions"]["room"] != serde_json::json!(true), v["content"].to_string());

        // Membership changes after the fact: carol leaves → her pill goes back to text, bob becomes unique.
        c.get_room(&room_id).unwrap().leave().await.unwrap();
        let started = Instant::now();
        let after = loop {
            let all = events(&sync, &mut views, &scope, &sent).await;
            if pills(find(&all, &me_and_other)).len() == 1 || started.elapsed() > Duration::from_secs(40) {
                break all;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let e = find(&after, &me_and_other);
        check("left_user_pill_becomes_text", pills(e).len() == 1 && matches!(e.mentions[0].kind, MentionKind::User { me: true, .. }), format!("{:?}", pills(e)));
        let name_now = find(&after, &bubble).sender.name.clone();
        let own_now = find(&after, &own_other);
        check("name_unique_after_leave_pill_equals_bubble", name_now.as_deref() == Some("Sam") && pills(own_now).first().map(|p| p.0.as_str()) == Some("Sam"), format!("bubble={name_now:?} pill={:?}", pills(own_now)));
        // A moderator's «@room» does highlight (m.mentions.room + power level 50).
        ra.update_power_levels(vec![(&bob, 50i32.into())]).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let at_room_mod = send("@room dal moderatore", Some("@room dal moderatore".into()), Some(assign!(Mentions::new(), { room: true }))).await;
        sent.push(at_room_mod.clone());
        let last = events(&sync, &mut views, &scope, &sent).await;
        let e = find(&last, &at_room_mod);
        check("at_room_from_moderator_highlights", pills(e) == [("@room".into(), "@room".into())] && e.highlighted, format!("{:?} hl={}", pills(e), e.highlighted));
        for s in services {
            let _ = s.stop().await;
        }
        assert!(report.is_empty(), "failed: {report:?}");
    });
}
