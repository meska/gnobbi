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
