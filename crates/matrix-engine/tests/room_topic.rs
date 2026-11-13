//! «Modifica stanza» > Argomento: round trip of m.room.topic through `edit_room` against a disposable
//! homeserver (driven by the timeline_probe script: PROBE_TARGET=room_topic PROBE_TEST=topic_round_trip).
//! The value read back (server state event AND the client's synced room) must equal what was sent;
//! only trailing white space is trimmed, line breaks and leading text stay; "" removes the topic.
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{create_room, edit_room},
    sdk::{Client, reqwest::Certificate, ruma, sliding_sync::VersionBuilder},
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// `content.topic` of the current m.room.topic state event, as the server stores it (None = no event or no key).
async fn server_topic(client: &Client, room: &ruma::RoomId) -> Option<String> {
    use ruma::api::client::state::get_state_event_for_key::v3::Request;
    let request = Request::new(room.to_owned(), ruma::events::StateEventType::RoomTopic, String::new());
    let response = client.send(request).await.ok()?;
    let json: serde_json::Value = serde_json::from_str(response.event_or_content.get()).unwrap();
    json["topic"].as_str().map(str::to_owned)
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn topic_round_trip() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        let service = SyncService::builder(client.clone()).build().await.unwrap();
        service.start().await;

        let account = AccountId::new(1);
        let created = create_room(&client, account, "gnobbi-topic-roundtrip", None, true).await.unwrap();
        let id = ruma::RoomId::parse(created.room_id.as_str()).unwrap();
        until("A has the room", || client.get_room(&id).is_some()).await;
        let room = client.get_room(&id).unwrap();

        let long = (0..400).map(|i| format!("riga {i} con accenti àèìòù e un emoji 🙂")).collect::<Vec<_>>().join("\n");
        // (what, sent). Expected back = sent with trailing white space removed.
        let cases: Vec<(&str, String)> = vec![
            ("3 righe", "uno\ndue\ntre".into()),
            ("3 righe, spazi in testa e a fine riga", "  uno \ndue \n\ttre".into()),
            ("spazi e a capo finali", "uno\ndue\ntre  \n \n".into()),
            ("riga vuota in mezzo", "uno\n\ntre".into()),
            ("unicode", "È già così — 日本語 🙂\nsecondo rigo".into()),
            ("molto lungo (>16 KB)", long),
        ];
        let scope = created.clone();
        for (what, sent) in cases {
            let want = sent.trim_end().to_owned();
            edit_room(&client, account, &scope, None, Some(&sent), None).await.unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(server_topic(&client, &id).await.as_deref(), Some(want.as_str()), "{what}: valore sul server");
            until(&format!("{what}: il client sincronizzato vede lo stesso valore"), || room.topic().as_deref() == Some(want.as_str())).await;
            println!("[topic] {what}: OK ({} byte)", want.len());
        }

        // The editor loads what the client has (room.topic()) and saves it back: unchanged = identical.
        let loaded = room.topic().unwrap();
        edit_room(&client, account, &scope, None, Some(&loaded), None).await.unwrap();
        assert_eq!(server_topic(&client, &id).await.as_deref(), Some(loaded.as_str()), "ricarica+salva senza modifiche");

        // Empty field: the topic is removed.
        edit_room(&client, account, &scope, None, Some("  \n "), None).await.unwrap();
        let after = server_topic(&client, &id).await;
        println!("[topic] svuotato: server topic = {after:?}");
        assert!(after.as_deref().is_none_or(str::is_empty), "svuotato: valore sul server {after:?}");
        until("il client non vede piu' l'argomento", || room.topic().is_none_or(|t| t.is_empty())).await;
        println!("[topic] svuotato: client topic = {:?}", room.topic());
    });
}
