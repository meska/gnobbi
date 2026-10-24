// sign-in bootstrap contracts, using the same isolated TLS server as auth.
#[cfg(unix)]
#[test]
#[ignore = "GUI identity readback driver; needs the external test harness (not in this repo)"]
fn disposable_gui_signin_identity() {
    use crate::signin::{Secret, SignInProfile};
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    runtime().block_on(async {
        let mut profile = SignInProfile::open(root.join("graphical-profile")).unwrap();
        let session = profile.unlock(
            Secret::new("disposable local unlock phrase".into()).unwrap(),
            Some(std::fs::read(root.join("ca.pem")).unwrap()),
        ).await.unwrap();
        let identity = session.whoami().await.unwrap();
        assert_eq!(identity.user_id(), "@alice:matrix-desktop.test");
        std::fs::write(root.join("handoff-identity.json"), serde_json::to_vec(
            &serde_json::json!({"user_id":identity.user_id(), "device_id":identity.device_id()})
        ).unwrap()).unwrap();
    });
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_interactive_unlock_retries_without_another_remote_login() {
    use crate::signin::{
        Secret, SignInCommand, SignInEvent, SignInMode, SignInProfile, SignInPrompt, interactive,
    };
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("interactive-unlock-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let phrase = "interactive restore phrase";
    let device = runtime().block_on(async {
        let mut profile = SignInProfile::open(root.clone()).unwrap();
        let session = profile
            .password_login(
                config["url"].as_str().unwrap().into(),
                "@alice:matrix-desktop.test".into(),
                Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
                Secret::new(phrase.into()).unwrap(),
                Some(pem.clone()),
            )
            .await
            .unwrap();
        session.whoami().await.unwrap().device_id().to_owned()
    });
    runtime().block_on(async {
        let before = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            "devices",
            serde_json::json!({}),
        )
        .await;
        let (mut ui, pending) = interactive(root, Some(pem));
        let consumer = async {
            assert_eq!(
                ui.events.recv().await,
                Some(SignInEvent::Prompt(SignInPrompt {
                    mode: SignInMode::Unlock,
                    error: None
                }))
            );
            for input in ["incorrect unlock phrase", phrase] {
                assert!(
                    ui.commands
                        .try_send(SignInCommand::Unlock {
                            phrase: Secret::new(input.into()).unwrap()
                        })
                        .is_ok()
                );
                let event = ui.events.recv().await.unwrap();
                if input == phrase {
                    assert_eq!(event, SignInEvent::Authenticated(AccountId::new(1)));
                } else {
                    assert_eq!(
                        event,
                        SignInEvent::Prompt(SignInPrompt {
                            mode: SignInMode::Unlock,
                            error: Some(crate::signin::SignInError::Authentication(
                                AuthError::Storage
                            ))
                        })
                    );
                }
            }
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        assert!(session.unwrap().whoami().await.unwrap().device_id() == device);
        let after = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            "devices",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            before["devices"].as_array().unwrap().len(),
            after["devices"].as_array().unwrap().len()
        );
    });
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_interactive_retries_then_hands_the_verified_session_to_the_caller() {
    use crate::signin::{
        Secret, SignInCommand, SignInEvent, SignInMode, SignInPrompt, interactive,
    };
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("interactive-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    runtime().block_on(async {
        let (mut ui, pending) = interactive(root, Some(pem));
        let consumer = async {
            assert_eq!(
                ui.events.recv().await,
                Some(SignInEvent::Prompt(SignInPrompt {
                    mode: SignInMode::Password,
                    error: None
                }))
            );
            for password in [
                "incorrect-fixture-password",
                config["alice_password"].as_str().unwrap(),
            ] {
                assert!(
                    ui.commands
                        .try_send(SignInCommand::Password {
                            homeserver: config["url"].as_str().unwrap().into(),
                            user: "@alice:matrix-desktop.test".into(),
                            password: Secret::new(password.into()).unwrap(),
                            phrase: Secret::new("interactive unlock phrase".into()).unwrap(),
                        })
                        .is_ok()
                );
                let event = ui.events.recv().await.unwrap();
                if password == "incorrect-fixture-password" {
                    assert_eq!(
                        event,
                        SignInEvent::Prompt(SignInPrompt {
                            mode: SignInMode::Password,
                            error: Some(crate::signin::SignInError::Authentication(
                                AuthError::CredentialsRejected
                            ))
                        })
                    );
                } else {
                    assert_eq!(event, SignInEvent::Authenticated(AccountId::new(2)));
                }
            }
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        let session = session.unwrap();
        assert_eq!(session.account_id(), AccountId::new(2));
        assert!(session.whoami().await.unwrap().user_id() == "@alice:matrix-desktop.test");
    });
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_profile_rechecks_interrupted_state_before_remote_login() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let root = parent.join("signin-interrupted-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    runtime().block_on(async {
        let mut profile = SignInProfile::open(root.clone()).unwrap();
        let before = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            "devices",
            serde_json::json!({}),
        )
        .await;
        std::fs::write(root.join("active-account.pending"), "1").unwrap();
        let result = profile
            .password_login(
                config["url"].as_str().unwrap().into(),
                "@alice:matrix-desktop.test".into(),
                Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
                Secret::new("interrupted profile phrase".into()).unwrap(),
                Some(pem),
            )
            .await;
        assert!(matches!(result, Err(SignInError::Storage)));
        assert!(
            !root.join("account-1").exists(),
            "interrupted state must be checked before another login/store"
        );
        let after = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            "devices",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            before["devices"].as_array().unwrap().len(),
            after["devices"].as_array().unwrap().len()
        );
    });
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_password_bootstrap_restores_the_same_device() {
    use crate::signin::{PasswordSignIn, Secret, UnlockSession};
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let account = AccountId::new(40);
    let phrase = "disposable sign-in unlock phrase";
    let (user, device) = runtime().block_on(async {
        let request = PasswordSignIn::new(
            root.to_owned(),
            account,
            config["url"].as_str().unwrap().to_owned(),
            "@alice:matrix-desktop.test".into(),
            Secret::new(config["alice_password"].as_str().unwrap().to_owned()).unwrap(),
            Secret::new(phrase.into()).unwrap(),
            Some(pem.clone()),
        )
        .unwrap();
        let session = request.run().await.unwrap();
        let identity = session.whoami().await.unwrap();
        (
            identity.user_id().to_owned(),
            identity.device_id().to_owned(),
        )
    });
    // A new runtime, not just a new wrapper, must use the sealed session.
    runtime().block_on(async {
        let session = UnlockSession::new(
            root.to_owned(),
            account,
            Secret::new(phrase.into()).unwrap(),
            Some(pem),
        )
        .unwrap()
        .run()
        .await
        .unwrap();
        let identity = session.whoami().await.unwrap();
        assert!(identity.user_id() == user);
        assert!(identity.device_id() == device);
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_profile_retries_rejected_credentials_without_reusing_a_dirty_slot() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let root = parent.join("signin-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let phrase = "profile retry unlock phrase";
    let device = runtime().block_on(async {
        let mut profile = SignInProfile::open(root.clone()).unwrap();
        assert_eq!(profile.active_account(), None);
        let rejected = profile
            .password_login(
                config["url"].as_str().unwrap().into(),
                "@alice:matrix-desktop.test".into(),
                Secret::new("incorrect-fixture-password".into()).unwrap(),
                Secret::new(phrase.into()).unwrap(),
                Some(pem.clone()),
            )
            .await;
        assert!(matches!(
            rejected,
            Err(SignInError::Authentication(AuthError::CredentialsRejected))
        ));
        assert_eq!(profile.active_account(), None);
        assert!(
            root.join("account-1").exists(),
            "failed attempt is preserved, never deleted as cache"
        );
        let session = profile
            .password_login(
                config["url"].as_str().unwrap().into(),
                "@alice:matrix-desktop.test".into(),
                Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
                Secret::new(phrase.into()).unwrap(),
                Some(pem.clone()),
            )
            .await
            .unwrap();
        assert_eq!(session.account_id(), AccountId::new(2));
        assert_eq!(profile.active_account(), Some(AccountId::new(2)));
        session.whoami().await.unwrap().device_id().to_owned()
    });
    runtime().block_on(async {
        let mut profile = SignInProfile::open(root).unwrap();
        assert_eq!(profile.active_account(), Some(AccountId::new(2)));
        let session = profile
            .unlock(Secret::new(phrase.into()).unwrap(), Some(pem))
            .await
            .unwrap();
        assert!(session.whoami().await.unwrap().device_id() == device);
    });
}

#[cfg(unix)]
struct VaultFake(std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, [u8; 1]>>, Option<crate::signin::VaultError>);
#[cfg(unix)]
impl crate::signin::ProtectionSource for VaultFake {
    fn create(&self, store: &crate::persistence::AccountStore) -> Result<crate::signin::Secret, crate::signin::VaultError> {
        if let Some(error) = self.1 { return Err(error); }
        self.0.lock().unwrap().insert(store.path.clone(), [0]);
        crate::signin::Secret::new("v".repeat(64)).map_err(|_| crate::signin::VaultError::Unavailable)
    }
    fn read(&self, store: &crate::persistence::AccountStore) -> Result<crate::signin::Secret, crate::signin::VaultError> {
        self.0.lock().unwrap().get(&store.path).ok_or(crate::signin::VaultError::Missing)?;
        crate::signin::Secret::new("v".repeat(64)).map_err(|_| crate::signin::VaultError::Unavailable)
    }
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_vault_route_logs_in_without_a_phrase_and_defers_recovery() {
    use crate::signin::{
        RecoveryState, Secret, SignInCommand, SignInError, SignInEvent, SignInMode, SignInPrompt,
        VaultError, interactive_with_source,
    };
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let root = parent.join("vault-route-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let login = |password: &str| SignInCommand::VaultPassword {
        homeserver: config["url"].as_str().unwrap().into(),
        user: "@alice:matrix-desktop.test".into(),
        password: Secret::new(password.into()).unwrap(),
    };
    let prompt = |error| SignInEvent::Prompt(SignInPrompt { mode: SignInMode::Password, error });
    runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(
            root.clone(),
            Some(pem),
            Box::new(VaultFake(Default::default(), Some(VaultError::Unavailable))),
        );
        let consumer = async {
            assert_eq!(ui.events.recv().await, Some(prompt(None)));
            // Vault failure: typed, payload-free, retryable.
            assert!(ui.commands.try_send(login(config["alice_password"].as_str().unwrap())).is_ok());
            assert_eq!(ui.events.recv().await, Some(prompt(Some(SignInError::Vault(VaultError::Unavailable)))));
            drop(ui);
        };
        let (result, ()) = tokio::join!(pending.run(), consumer);
        assert!(result.is_err());
        // A failed attempt never selects a profile.
        assert!(!root.join("active-account").exists());
    });
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(root.clone(), Some(pem), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            assert_eq!(ui.events.recv().await, Some(prompt(None)));
            assert!(ui.commands.try_send(login("incorrect-fixture-password")).is_ok());
            assert_eq!(
                ui.events.recv().await,
                Some(prompt(Some(SignInError::Authentication(AuthError::CredentialsRejected))))
            );
            assert!(ui.commands.try_send(login(config["alice_password"].as_str().unwrap())).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            // This account has no secret storage: the recovery key route does not exist.
            assert!(ui.commands.try_send(SignInCommand::EnterRecovery(Secret::new("EsTc 1111 2222 3333".into()).unwrap())).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Unsupported)));
            assert!(ui.commands.try_send(SignInCommand::DeferRecovery).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Deferred)));
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        let session = session.unwrap();
        assert!(session.whoami().await.unwrap().user_id() == "@alice:matrix-desktop.test");
    });
    assert_eq!(std::fs::read(root.join("protection-mode")).unwrap(), b"vault-v1");
    assert!(root.join("active-account").exists());
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_recovery_key_restores_the_exact_old_event_and_failures_change_nothing() {
    use crate::signin::{
        RecoveryFailure, RecoveryState, Secret, SignInCommand, SignInEvent, SignInMode,
        SignInPrompt, interactive_with_source,
    };
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let root = parent.join("vault-recovery-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let (room, event, key, wrong_key, before) = runtime().block_on(async {
        let (mut source, room, event) = recovery_seed(&config).await;
        let key = source.enable_new_recovery().await.expect("source recovery");
        source.upload_key_backup().await.unwrap();
        let other = login(&config, "bob", 27).await;
        let client = other.client.as_ref().unwrap();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        let wrong = client.encryption().secret_storage().create_secret_store().await.unwrap().secret_storage_key();
        let before = recovery_backup_wire(&config).await;
        (room, event, key.expose().to_owned(), wrong, before)
    });
    // Precondition: a fresh device cannot read the old event.
    runtime().block_on(async {
        let fresh = login(&config, "alice", 28).await;
        let client = fresh.client.as_ref().unwrap();
        client.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        recovery_assert_message(client, &room, &event, false).await;
    });
    let secret = |text: &str| Secret::new(text.into()).unwrap();
    let paused_state = std::sync::Mutex::new(None);
    let session = runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(root.clone(), Some(pem), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            let prompt = SignInEvent::Prompt(SignInPrompt { mode: SignInMode::Password, error: None });
            assert_eq!(ui.events.recv().await, Some(prompt));
            assert!(ui.commands.try_send(SignInCommand::VaultPassword {
                homeserver: config["url"].as_str().unwrap().into(),
                user: "@alice:matrix-desktop.test".into(),
                password: secret(config["alice_password"].as_str().unwrap()),
            }).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            // Wrong valid key and malformed key: typed failure, retry stays possible.
            for bad in [wrong_key.as_str(), "not a recovery key"] {
                assert!(ui.commands.try_send(SignInCommand::EnterRecovery(secret(bad))).is_ok());
                assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
                assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Failed(RecoveryFailure::InvalidKey))));
            }
            assert!(recovery_backup_wire(&config).await == before, "failed attempts must not touch the server backup");
            // Transport failure while pending.
            let paused = PausedServer::new(&config);
            assert!(ui.commands.try_send(SignInCommand::EnterRecovery(secret(&key))).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
            *paused_state.lock().unwrap() = ui.events.recv().await;
            drop(paused);
            assert!(recovery_backup_wire(&config).await == before, "transport failure must not touch the server backup");
            assert!(ui.commands.try_send(SignInCommand::EnterRecovery(secret(&key))).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Recovered)));
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        session.unwrap()
    });
    assert!(matches!(
        paused_state.into_inner().unwrap(),
        Some(SignInEvent::Recovery(RecoveryState::Failed(RecoveryFailure::Network | RecoveryFailure::Timeout)))
    ));
    // The session must drop inside a runtime (SDK pool).
    runtime().block_on(async move {
        recovery_assert_message(session.client.as_ref().unwrap(), &room, &event, true).await;
        assert!(recovery_backup_wire(&config).await == before, "recovery must not mutate the server backup");
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_recovery_passphrase_uses_the_same_route() {
    use crate::signin::{RecoveryState, Secret, SignInCommand, SignInEvent, interactive_with_source};
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let root = parent.join("vault-recovery-passphrase-profile");
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let (room, event) = runtime().block_on(async {
        let (mut source, room, event) = recovery_seed(&config).await;
        source.client.as_ref().unwrap().encryption().recovery().enable().with_passphrase("fixture recovery passphrase").await.unwrap();
        source.upload_key_backup().await.unwrap();
        (room, event)
    });
    let session = runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(root, Some(pem), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            ui.events.recv().await.unwrap();
            assert!(ui.commands.try_send(SignInCommand::VaultPassword {
                homeserver: config["url"].as_str().unwrap().into(),
                user: "@alice:matrix-desktop.test".into(),
                password: Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
            }).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            assert!(ui.commands.try_send(SignInCommand::EnterRecovery(Secret::new("fixture recovery passphrase".into()).unwrap())).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Recovered)));
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        session.unwrap()
    });
    runtime().block_on(async move {
        recovery_assert_message(session.client.as_ref().unwrap(), &room, &event, true).await;
    });
}

#[cfg(unix)]
#[test]
#[ignore = "GUI recovery seed driver; needs the external test harness (not in this repo)"]
fn disposable_gui_recovery_seed() {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let config = fixture();
    let out = std::path::PathBuf::from(std::env::var("FIXTURE_RECOVERY_OUT").expect("private output dir"));
    let (room, event, key, wrong, backup) = runtime().block_on(async {
        let (mut source, room, event) = recovery_seed(&config).await;
        let key = source.enable_new_recovery().await.expect("source recovery");
        source.upload_key_backup().await.unwrap();
        let other = login(&config, "bob", 27).await;
        let client = other.client.as_ref().unwrap();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        let wrong = client.encryption().secret_storage().create_secret_store().await.unwrap().secret_storage_key();
        (room, event, key.expose().to_owned(), wrong, recovery_backup_wire(&config).await)
    });
    let private = |name: &str, text: &str| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(out.join(name)).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    };
    private("recovery.key", &key);
    private("wrong.key", &wrong);
    private("backup-before.json", &backup.to_string());
    std::fs::write(out.join("ids.json"), serde_json::json!({"room": room, "event": event.as_str()}).to_string()).unwrap();
}

/// Seeds for the GUI setup certification: an encrypted room for alice, or a backup (and nothing
/// else) for bob, so that his account is in the partial state setup must refuse.
#[cfg(unix)]
#[test]
#[ignore = "GUI setup seed driver; needs the external test harness (not in this repo)"]
fn disposable_gui_setup_seed() {
    let config = fixture();
    let out = std::path::PathBuf::from(std::env::var("FIXTURE_SETUP_OUT").expect("output dir"));
    match std::env::var("FIXTURE_SETUP_MODE").expect("mode").as_str() {
        "alice-room" => {
            let room = runtime().block_on(async { room_request(&config, "alice", reqwest::Method::POST, "createRoom",
                serde_json::json!({"name": "disposable setup", "visibility": "private", "initial_state": [
                    {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}
                ]})).await["room_id"].as_str().unwrap().to_owned() });
            std::fs::write(out.join("ids.json"), serde_json::json!({"room": room}).to_string()).unwrap();
        }
        "bob-backup-only" => runtime().block_on(async {
            let seeder = login(&config, "bob", 29).await;
            let client = seeder.client.as_ref().unwrap();
            client.encryption().wait_for_e2ee_initialization_tasks().await;
            client.encryption().backups().create().await.unwrap();
        }),
        other => panic!("unknown seed mode {other}"),
    }
}

/// Server-side state a refused setup must leave alone: backup version, default secret-storage key,
/// published cross-signing keys. The fresh sign-in device is deliberately not part of it.
#[cfg(unix)]
async fn setup_wire(config: &serde_json::Value) -> serde_json::Value {
    let base = format!("{}_matrix/client/v3/", config["url"].as_str().unwrap());
    let token = config["alice_token"].as_str().unwrap();
    let get = |path: String| {
        let request = http(config).get(format!("{base}{path}")).bearer_auth(token);
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            (status, serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap())
        }
    };
    let query = http(config).post(format!("{base}keys/query")).bearer_auth(token)
        .header("Content-Type", "application/json").body(r#"{"device_keys":{"@alice:matrix-desktop.test":[]}}"#).send().await.unwrap();
    let query: serde_json::Value = serde_json::from_slice(&query.bytes().await.unwrap()).unwrap();
    serde_json::json!({
        "backup": get("room_keys/version".into()).await,
        "default_key": get("user/@alice:matrix-desktop.test/account_data/m.secret_storage.default_key".into()).await,
        "master": query["master_keys"], "self_signing": query["self_signing_keys"], "user_signing": query["user_signing_keys"],
    })
}

/// Seed alice's server state, sign in again on a new device through the vault route and ask for
/// recovery setup. Returns the typed refusal; the wire state must be byte-identical afterwards.
#[cfg(unix)]
fn setup_refused(root_name: &str, seed: fn(&matrix_sdk::Client) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>>) -> crate::signin::SetupFailure {
    use crate::signin::{RecoveryState, Secret, SetupEvent, SignInCommand, SignInEvent, interactive_with_source};
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let before = runtime().block_on(async {
        let seeder = login(&config, "alice", 25).await;
        let client = seeder.client.as_ref().unwrap();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        seed(client).await;
        setup_wire(&config).await
    });
    let (failure, session) = runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(parent.join(root_name), Some(pem), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            ui.events.recv().await.unwrap();
            assert!(ui.commands.try_send(SignInCommand::VaultPassword {
                homeserver: config["url"].as_str().unwrap().into(),
                user: "@alice:matrix-desktop.test".into(),
                password: Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
            }).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            assert!(ui.commands.try_send(SignInCommand::SetupRecovery).is_ok());
            assert!(matches!(ui.setup.recv().await, Some(SetupEvent::Pending)));
            let Some(SetupEvent::Failed(kind)) = ui.setup.recv().await else { panic!("setup must be refused") };
            assert!(ui.commands.try_send(SignInCommand::DeferRecovery).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Deferred)));
            kind
        };
        let (session, kind) = tokio::join!(pending.run(), consumer);
        (kind, session.unwrap())
    });
    runtime().block_on(async move {
        assert!(setup_wire(&config).await == before, "a refused setup must not touch the server");
        drop(session);
    });
    failure
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_setup_recovery_creates_backup_and_a_second_device_restores_the_exact_event() {
    use crate::signin::{RecoveryState, Secret, SetupEvent, SignInCommand, SignInEvent, SignInMode, SignInPrompt, interactive_with_source};
    let config = fixture();
    let parent = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let room = runtime().block_on(async { room_request(&config, "alice", reqwest::Method::POST, "createRoom",
        serde_json::json!({"name": "disposable setup", "visibility": "private", "initial_state": [
            {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}
        ]})).await["room_id"].as_str().unwrap().to_owned() });
    let (key, first, event) = runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(parent.join("setup-first"), Some(pem.clone()), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Prompt(SignInPrompt { mode: SignInMode::Password, error: None })));
            assert!(ui.commands.try_send(SignInCommand::VaultPassword {
                homeserver: config["url"].as_str().unwrap().into(),
                user: "@alice:matrix-desktop.test".into(),
                password: Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
            }).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            assert!(ui.commands.try_send(SignInCommand::SetupRecovery).is_ok());
            assert!(matches!(ui.setup.recv().await, Some(SetupEvent::Pending)));
            let Some(SetupEvent::Ready(key)) = ui.setup.recv().await else { panic!("setup must be ready") };
            assert!(ui.commands.try_send(SignInCommand::DeferRecovery).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Deferred)));
            key.expose().to_owned()
        };
        let (session, key) = tokio::join!(pending.run(), consumer);
        let mut session = session.unwrap();
        let client = session.client.as_ref().unwrap().clone();
        client.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        let event = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap()
            .send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("recover old encrypted città 🦀"))
            .await.unwrap().response.event_id;
        session.upload_key_backup().await.unwrap();
        (key, session, event)
    });
    let second = runtime().block_on(async {
        let (mut ui, pending) = interactive_with_source(parent.join("setup-second"), Some(pem), Box::new(VaultFake(Default::default(), None)));
        let consumer = async {
            ui.events.recv().await.unwrap();
            assert!(ui.commands.try_send(SignInCommand::VaultPassword {
                homeserver: config["url"].as_str().unwrap().into(),
                user: "@alice:matrix-desktop.test".into(),
                password: Secret::new(config["alice_password"].as_str().unwrap().into()).unwrap(),
            }).is_ok());
            assert!(matches!(ui.events.recv().await, Some(SignInEvent::Authenticated(_))));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Offered)));
            assert!(ui.commands.try_send(SignInCommand::EnterRecovery(Secret::new(key).unwrap())).is_ok());
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Pending)));
            assert_eq!(ui.events.recv().await, Some(SignInEvent::Recovery(RecoveryState::Recovered)));
        };
        let (session, ()) = tokio::join!(pending.run(), consumer);
        session.unwrap()
    });
    runtime().block_on(async move {
        recovery_assert_message(second.client.as_ref().unwrap(), &room, &event, true).await;
        drop(first); // SDK sessions must drop inside a runtime.
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_setup_recovery_refuses_an_account_that_already_has_recovery() {
    let kind = setup_refused("setup-exists", |client| Box::pin(async move {
        client.encryption().bootstrap_cross_signing(None).await.unwrap();
        client.encryption().recovery().enable().await.unwrap();
    }));
    assert_eq!(kind, crate::signin::SetupFailure::AlreadyExists);
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_setup_recovery_refuses_a_backup_without_secret_storage() {
    let kind = setup_refused("setup-backup-only", |client| Box::pin(async move {
        client.encryption().backups().create().await.unwrap();
    }));
    assert_eq!(kind, crate::signin::SetupFailure::Partial);
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_setup_recovery_refuses_secret_storage_without_a_backup() {
    let kind = setup_refused("setup-storage-only", |client| Box::pin(async move {
        client.encryption().secret_storage().create_secret_store().await.unwrap();
    }));
    assert_eq!(kind, crate::signin::SetupFailure::Partial);
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_signin_setup_recovery_never_replaces_an_identity_this_device_cannot_sign_with() {
    let kind = setup_refused("setup-foreign-identity", |client| Box::pin(async move {
        client.encryption().bootstrap_cross_signing(None).await.unwrap();
    }));
    assert_eq!(kind, crate::signin::SetupFailure::IdentityRequired);
}
