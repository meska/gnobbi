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
