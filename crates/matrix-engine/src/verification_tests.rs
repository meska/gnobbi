// Real protocol tests; the launcher creates a fresh isolated Synapse per contract.
#[test]
#[ignore = "fixture peer only; needs the external test harness (not in this repo)"]
fn fixture_gui_verification_peer() {
    runtime().block_on(async {
        use crate::verification::{VerificationAction as Action, VerificationState as State};
        use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent;
        let config = fixture();
        let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
        let client = matrix_sdk::Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .disable_well_known_lookup(true).respect_login_well_known(false)
            .http_client(http(&config)).build().await.unwrap();
        client.matrix_auth().login_username("@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap())
            .device_id("PEER").send().await.unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let incoming = requests.clone();
        client.add_event_handler(move |event: ToDeviceKeyVerificationRequestEvent| {
            let incoming = incoming.clone();
            async move { incoming.lock().unwrap().push((event.sender.to_string(), event.content.transaction_id.to_string())); }
        });
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        verification_sync(&client).await;
        verification_sync(&client).await;
        let peer = AccountSession { account_id: AccountId::new(24), client: Some(client.clone()), store: None };
        std::fs::write(root.join("peer-ready"), "ready").unwrap();
        let mut flow: Option<crate::verification::VerificationFlow> = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(85);
        while !root.join("peer-stop").exists() {
            assert!(std::time::Instant::now() < deadline, "fixture peer deadline");
            verification_sync(&client).await;
            let request = requests.lock().unwrap().pop();
            if let Some((user, id)) = request {
                let mut next = peer.receive_device_verification(&user, &id).await.unwrap();
                next.perform(Action::Accept).await.unwrap();
                flow = Some(next);
            }
            if let Some(flow) = &mut flow {
                if let Some(sas) = client.encryption().get_verification(client.user_id().unwrap(), &flow.snapshot().await.unwrap().flow_id).await.and_then(|v| v.sas())
                    && matches!(sas.state(), matrix_sdk::encryption::verification::SasState::Started { .. }) {
                        flow.perform(Action::AcceptSas).await.unwrap();
                    }
                let snapshot = flow.snapshot().await.unwrap();
                if let State::Compare { decimals } = snapshot.state {
                    std::fs::write(root.join("peer-sas"), serde_json::to_vec(&decimals).unwrap()).unwrap();
                    if root.join("peer-confirm").exists() {
                        std::fs::remove_file(root.join("peer-confirm")).unwrap();
                        flow.perform(Action::Confirm).await.unwrap();
                    }
                }
                if matches!(snapshot.state, State::Verified | State::Mismatched | State::Cancelled) {
                    let label = match snapshot.state { State::Verified => "verified", State::Mismatched => "mismatched", _ => "cancelled" };
                    if snapshot.state == State::Verified { assert!(snapshot.device_verified); }
                    std::fs::write(root.join("peer-state"), label).unwrap();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_request_is_scoped_and_untrusted() {
    runtime().block_on(async {
        let config = fixture();
        let (session, peer) = verification_pair(&config).await;
        let flow = session
            .request_device_verification(
                peer.user_id().unwrap().as_str(),
                peer.device_id().unwrap().as_str(),
            )
            .await
            .unwrap();
        let snapshot = flow.snapshot().await.unwrap();
        assert_eq!(snapshot.account_id, AccountId::new(23));
        assert_eq!(snapshot.user_id, peer.user_id().unwrap().as_str());
        assert_eq!(snapshot.device_id, peer.device_id().unwrap().as_str());
        assert_eq!(
            snapshot.state,
            crate::verification::VerificationState::Waiting
        );
        assert!(!snapshot.device_verified);
        assert!(!snapshot.flow_id.is_empty());
    });
}

async fn verification_pair(config: &serde_json::Value) -> (AccountSession, matrix_sdk::Client) {
    let (_room, _alice, peer) = encrypted_pair(config).await;
    let session = login(config, "alice", 23).await;
    let client = session.client.as_ref().unwrap();
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
    verification_sync(client).await;
    verification_sync(client).await;
    // The extra requester device was logged in after encrypted_pair. Deliver the
    // device-list change and flush the SDK key query before requesting verification.
    verification_sync(&peer).await;
    verification_sync(&peer).await;
    (session, peer)
}

async fn verification_sync(client: &matrix_sdk::Client) {
    client
        .sync_once(matrix_sdk::config::SyncSettings::default().timeout(std::time::Duration::ZERO))
        .await
        .unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_sas_confirmation_verifies_exact_devices() {
    runtime().block_on(async {
        use crate::verification::{VerificationAction as Action, VerificationState as State};
        let config = fixture();
        let (alice, peer) = verification_pair(&config).await;
        let bob = AccountSession {
            account_id: AccountId::new(24),
            client: Some(peer.clone()),
            store: None,
        };
        let client = alice.client.as_ref().unwrap();
        let (mut flow, mut incoming) = verification_comparison(&alice, &bob).await;
        let left = flow.snapshot().await.unwrap();
        let right = incoming.snapshot().await.unwrap();
        assert!(matches!(left.state, State::Compare { .. }));
        assert_eq!(
            left.state, right.state,
            "real ECDH SAS must match on both devices"
        );
        assert!(!left.device_verified);
        assert!(!right.device_verified);
        flow.perform(Action::Confirm).await.unwrap();
        assert_ne!(flow.snapshot().await.unwrap().state, State::Verified);
        verification_sync(&peer).await;
        incoming.perform(Action::Confirm).await.unwrap();
        verification_sync(client).await;
        verification_sync(&peer).await;
        verification_sync(client).await;
        let left = flow.snapshot().await.unwrap();
        let right = incoming.snapshot().await.unwrap();
        assert_eq!(left.state, State::Verified);
        assert_eq!(right.state, State::Verified);
        assert!(left.device_verified);
        assert!(right.device_verified);
        assert_eq!(left.device_id, peer.device_id().unwrap().as_str());
        assert_eq!(right.device_id, client.device_id().unwrap().as_str());
    });
}

async fn verification_comparison(
    alice: &AccountSession,
    bob: &AccountSession,
) -> (
    crate::verification::VerificationFlow,
    crate::verification::VerificationFlow,
) {
    use crate::verification::{VerificationAction as Action, VerificationState as State};
    let client = alice.client.as_ref().unwrap();
    let peer = bob.client.as_ref().unwrap();
    assert!(
        peer.encryption()
            .get_device(client.user_id().unwrap(), client.device_id().unwrap())
            .await
            .unwrap()
            .is_some(),
        "peer must track requester before delivery"
    );
    let mut flow = alice
        .request_device_verification(
            peer.user_id().unwrap().as_str(),
            peer.device_id().unwrap().as_str(),
        )
        .await
        .unwrap();
    let id = flow.snapshot().await.unwrap().flow_id;
    verification_sync(peer).await;
    let mut incoming = bob
        .receive_device_verification(client.user_id().unwrap().as_str(), &id)
        .await
        .unwrap();
    assert_eq!(incoming.snapshot().await.unwrap().state, State::Requested);
    incoming.perform(Action::Accept).await.unwrap();
    verification_sync(client).await;
    assert_eq!(flow.snapshot().await.unwrap().state, State::Ready);
    flow.perform(Action::StartSas).await.unwrap();
    verification_sync(peer).await;
    incoming.perform(Action::AcceptSas).await.unwrap();
    verification_sync(client).await;
    verification_sync(peer).await;
    verification_sync(client).await;
    (flow, incoming)
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_cancel_keeps_both_devices_untrusted() {
    runtime().block_on(async {
        use crate::verification::{VerificationAction as Action, VerificationState as State};
        let config = fixture();
        let (alice, peer) = verification_pair(&config).await;
        let client = alice.client.as_ref().unwrap();
        let bob = AccountSession {
            account_id: AccountId::new(24),
            client: Some(peer.clone()),
            store: None,
        };
        let mut flow = alice
            .request_device_verification(
                peer.user_id().unwrap().as_str(),
                peer.device_id().unwrap().as_str(),
            )
            .await
            .unwrap();
        let id = flow.snapshot().await.unwrap().flow_id;
        verification_sync(&peer).await;
        let incoming = bob
            .receive_device_verification(client.user_id().unwrap().as_str(), &id)
            .await
            .unwrap();
        flow.perform(Action::Cancel).await.unwrap();
        verification_sync(&peer).await;
        for snapshot in [
            flow.snapshot().await.unwrap(),
            incoming.snapshot().await.unwrap(),
        ] {
            assert_eq!(snapshot.state, State::Cancelled);
            assert!(!snapshot.device_verified);
        }
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_mismatch_never_trusts_devices() {
    runtime().block_on(async {
        use crate::verification::{VerificationAction as Action, VerificationState as State};
        let config = fixture();
        let (alice, peer) = verification_pair(&config).await;
        let bob = AccountSession {
            account_id: AccountId::new(24),
            client: Some(peer.clone()),
            store: None,
        };
        let (mut flow, incoming) = verification_comparison(&alice, &bob).await;
        flow.perform(Action::Mismatch).await.unwrap();
        verification_sync(&peer).await;
        for snapshot in [
            flow.snapshot().await.unwrap(),
            incoming.snapshot().await.unwrap(),
        ] {
            assert_eq!(snapshot.state, State::Mismatched);
            assert!(!snapshot.device_verified);
        }
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_start_failure_is_terminal_untrusted() {
    runtime().block_on(async {
        use crate::verification::{
            VerificationAction as Action, VerificationError as Error, VerificationState as State,
        };
        let config = fixture();
        let (mut alice, peer) = verification_pair(&config).await;
        let bob = AccountSession {
            account_id: AccountId::new(24),
            client: Some(peer.clone()),
            store: None,
        };
        let mut flow = alice
            .request_device_verification(
                peer.user_id().unwrap().as_str(),
                peer.device_id().unwrap().as_str(),
            )
            .await
            .unwrap();
        let id = flow.snapshot().await.unwrap().flow_id;
        verification_sync(&peer).await;
        let mut incoming = bob
            .receive_device_verification(
                alice.client.as_ref().unwrap().user_id().unwrap().as_str(),
                &id,
            )
            .await
            .unwrap();
        incoming.perform(Action::Accept).await.unwrap();
        verification_sync(alice.client.as_ref().unwrap()).await;
        assert_eq!(flow.snapshot().await.unwrap().state, State::Ready);
        alice.logout().await.unwrap();
        assert_eq!(flow.perform(Action::StartSas).await, Err(Error::Transport));
        let snapshot = flow.snapshot().await.unwrap();
        assert_eq!(snapshot.state, State::Failed);
        assert!(!snapshot.device_verified);
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_send_failure_is_not_success_or_trust() {
    runtime().block_on(async {
        use crate::verification::{
            VerificationAction as Action, VerificationError as Error, VerificationState as State,
        };
        let config = fixture();
        let (mut alice, peer) = verification_pair(&config).await;
        let mut flow = alice
            .request_device_verification(
                peer.user_id().unwrap().as_str(),
                peer.device_id().unwrap().as_str(),
            )
            .await
            .unwrap();
        alice.logout().await.unwrap();
        assert_eq!(flow.perform(Action::Cancel).await, Err(Error::Transport));
        let snapshot = flow.snapshot().await.unwrap();
        assert_eq!(snapshot.state, State::Failed);
        assert!(!snapshot.device_verified);
        assert_eq!(
            flow.perform(Action::Confirm).await,
            Err(Error::InvalidState)
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_verification_worker_routes_devices_request_and_cancel() {
    use crate::verification::{
        VerificationAction as Action, VerificationCommand as Command, VerificationEvent as Event,
        VerificationState as State,
    };
    let config = fixture();
    let (session, _peer) = runtime().block_on(verification_pair(&config));
    let (mut worker, mut rooms) =
        crate::room_sync::RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    let (mut commands, mut events) = worker.take_verification_route().unwrap();
    runtime().block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !matches!(
                rooms.recv().await,
                Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
            ) {}
            commands
                .send(Command::List {
                    account_id: AccountId::new(23),
                })
                .await
                .unwrap_or_else(|_| panic!("command rejected"));
            let devices = match events.recv().await.unwrap() {
                Event::Devices {
                    account_id,
                    devices,
                } => {
                    assert_eq!(account_id, AccountId::new(23));
                    devices
                }
                _ => panic!("expected device list"),
            };
            let device = devices.first().expect("other alice device");
            assert_eq!(device.user_id, "@alice:matrix-desktop.test");
            commands
                .send(Command::Begin {
                    account_id: AccountId::new(23),
                    user_id: device.user_id.clone(),
                    device_id: device.device_id.clone(),
                })
                .await
                .unwrap_or_else(|_| panic!("command rejected"));
            let snapshot = match events.recv().await.unwrap() {
                Event::Snapshot(s) => s,
                _ => panic!("expected request"),
            };
            assert_eq!(snapshot.state, State::Waiting);
            assert!(!snapshot.device_verified);
            commands
                .send(Command::Act {
                    target: snapshot.target(),
                    action: Action::Cancel,
                })
                .await
                .unwrap_or_else(|_| panic!("command rejected"));
            loop {
                if let Event::Snapshot(snapshot) = events.recv().await.unwrap()
                    && snapshot.state == State::Cancelled
                {
                    assert!(!snapshot.device_verified);
                    break;
                }
            }
        })
        .await
        .unwrap();
    });
    worker.shutdown().unwrap();
}
