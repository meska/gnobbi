//! Official SyncService + RoomListService against the owner Tuwunel (CT 245).
//! Driven by the tuwunel_smoke script, which provisions one m.federate:false room
//! and passes a private auth config + room id via env. No secret here: this
//! file reads only the FIXTURE_AUTH_CONFIG path and the room id.
#![cfg(feature = "sdk")]
use futures_util::StreamExt;
use matrix_engine::{
    sdk::{Client, sliding_sync::VersionBuilder},
    sdk_ui::{eyeball_im::VectorDiff, sync_service::SyncService},
};
use std::time::Duration;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn wait<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(40), fut)
        .await
        .unwrap_or_else(|_| panic!("timeout waiting for {what}"))
}

#[test]
#[ignore = "requires the tuwunel_smoke script"]
fn login_starts_sync_and_lists_the_provisioned_room() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let room_id = env("PROBE_ROOM");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(
                config["mxid"].as_str().unwrap(),
                config["password"].as_str().unwrap(),
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
        service.stop().await;
        let _ = client.matrix_auth().logout().await;
    });
}
