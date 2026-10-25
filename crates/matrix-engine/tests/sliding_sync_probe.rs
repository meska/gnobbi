//! Official SyncService + RoomListService against a disposable homeserver.
//! Driven by the sliding_probe script inside a private namespace.
#![cfg(feature = "sdk")]
use futures_util::StreamExt;
use matrix_engine::{
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    sdk_ui::{eyeball_im::VectorDiff, sync_service::SyncService},
};
use std::{path::PathBuf, time::Duration};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn wait<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(40), fut)
        .await
        .unwrap_or_else(|_| panic!("timeout waiting for {what}"))
}

#[test]
#[ignore = "requires the sliding_probe script"]
fn room_list_arrives_and_a_new_message_updates_it() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let ready = PathBuf::from(env("PROBE_READY"));
    let room_id = env("PROBE_ROOM");
    let body = env("PROBE_BODY");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
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
            .login_username(
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let service = SyncService::builder(client.clone()).build().await.unwrap();
        service.start().await;
        let list = service.room_list_service().all_rooms().await.unwrap();
        let (stream, controller) = list.entries_with_dynamic_adapters(50);
        controller.set_filter(Box::new(
            matrix_engine::sdk_ui::room_list_service::filters::new_filter_all(vec![]),
        ));
        let mut stream = Box::pin(stream);
        let mut seen = false;
        while !seen {
            for diff in wait("room list", stream.next())
                .await
                .expect("stream ended")
            {
                let rooms: Vec<_> = match diff {
                    VectorDiff::Reset { values } | VectorDiff::Append { values } => {
                        values.into_iter().collect()
                    }
                    VectorDiff::PushBack { value } | VectorDiff::PushFront { value } => vec![value],
                    VectorDiff::Insert { value, .. } | VectorDiff::Set { value, .. } => vec![value],
                    _ => vec![],
                };
                seen |= rooms.iter().any(|r| r.room_id().as_str() == room_id);
            }
        }
        std::fs::write(&ready, b"1").unwrap();
        let mut updated = false;
        while !updated {
            for diff in wait("update after message", stream.next())
                .await
                .expect("stream ended")
            {
                if let VectorDiff::Set { value, .. } = diff {
                    if value.room_id().as_str() == room_id {
                        if let matrix_engine::sdk::latest_events::LatestEventValue::Remote(event) =
                            value.latest_event()
                        {
                            let json: serde_json::Value =
                                serde_json::from_str(event.raw().json().get()).unwrap();
                            updated = json["content"]["body"] == body.as_str();
                        }
                    }
                }
            }
        }
        service.stop().await;
    });
}
