//! Headless SDK capability probes, not homeserver interoperability.
use matrix_engine::sdk::Client;

#[test]
fn sdk_client_can_cross_the_background_runtime_boundary() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Client>();
}

/// Probe upstream initialization, not application login or server validation.
#[test]
fn sdk_initializes_an_unauthenticated_memory_client_without_discovery() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let client = Client::builder()
            .homeserver_url("https://offline-sdk-smoke.invalid")
            .sliding_sync_version_builder(matrix_engine::sdk::sliding_sync::VersionBuilder::None)
            // Test-only capability fixture: never use this in real discovery.
            .server_versions([matrix_engine::sdk::ruma::api::MatrixVersion::V1_0])
            .build()
            .await
            .unwrap();
        assert_eq!(
            client.homeserver().as_str(),
            "https://offline-sdk-smoke.invalid/"
        );
        assert!(client.user_id().is_none());
        assert!(client.rooms().is_empty());
        let _ = client.encryption();
    });
}

/// Opens real SDK SQLite stores under the test target, never personal data.
#[test]
fn sdk_opens_bundled_sqlite_stores_in_an_owned_fixture_directory() {
    let root = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("sdk-store-fixture-{}", std::process::id()));
    // Refuse to reuse or delete any preexisting directory.
    std::fs::create_dir(&root).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let client = Client::builder()
            .homeserver_url("https://offline-sdk-smoke.invalid")
            .sliding_sync_version_builder(matrix_engine::sdk::sliding_sync::VersionBuilder::None)
            .server_versions([matrix_engine::sdk::ruma::api::MatrixVersion::V1_0])
            .sqlite_store(&root, Some("offline-fixture-only-not-a-real-secret"))
            .build()
            .await
            .unwrap();
        assert!(client.user_id().is_none());
        assert!(client.rooms().is_empty());
        let _ = client.encryption();
        assert!(std::fs::read_dir(&root).unwrap().next().is_some());
    });
    // Join blocking store work before removing only the directory we created.
    drop(runtime);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(!root.exists());
}
