#![cfg(all(feature = "sdk", unix))]
use matrix_engine::signin::{SignInEvent, SignInMode, SignInPrompt, interactive};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Root(std::path::PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!(
                "interactive-signin-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        assert!(!path.exists());
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if self.0.exists() {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
// Probes of the existing bounded-channel / worker cancellation contracts.
#[test]
fn interactive_route_bounds_pending_credentials_and_closes_when_backend_is_dropped() {
    use matrix_engine::TrySendError;
    use matrix_engine::signin::{Secret, SignInCommand};
    let root = Root::new();
    let (mut ui, pending) = interactive(root.0.clone(), None);
    let command = || SignInCommand::Unlock {
        phrase: Secret::new("queue fixture unlock phrase".into()).unwrap(),
    };
    assert!(ui.commands.try_send(command()).is_ok());
    assert!(matches!(
        ui.commands.try_send(command()),
        Err(TrySendError::Full(_))
    ));
    drop(pending);
    assert!(matches!(
        ui.commands.try_send(command()),
        Err(TrySendError::Closed(_))
    ));
    assert!(!root.0.exists());
}
#[test]
fn existing_worker_cancels_a_waiting_signin_and_releases_profile_ownership() {
    let root = Root::new();
    let (mut ui, pending) = interactive(root.0.clone(), None);
    let (worker, _events) = matrix_engine::room_sync::RoomListWorker::spawn(move || async move {
        pending
            .run()
            .await
            .map_err(|_| matrix_engine::auth::AuthError::LoginFailed)
    })
    .unwrap();
    let started = std::time::Instant::now();
    loop {
        match ui.events.try_recv() {
            Ok(SignInEvent::Prompt(_)) => break,
            Err(matrix_engine::TryReceiveError::Empty | matrix_engine::TryReceiveError::Busy) => (),
            _ => panic!("backend failed before the prompt"),
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(worker.shutdown().is_ok());
    assert!(matrix_engine::signin::SignInProfile::open(root.0.clone()).is_ok());
    assert_eq!(
        ui.events.try_recv(),
        Err(matrix_engine::TryReceiveError::Closed)
    );
}

#[test]
fn interactive_setup_failure_is_reported_without_paths_or_raw_io_errors() {
    use matrix_engine::signin::SignInError;
    let (mut ui, pending) = interactive("relative".into(), None);
    runtime().block_on(async {
        let result = pending.run().await;
        assert!(matches!(result, Err(SignInError::InvalidStorePath)));
        assert_eq!(
            ui.events.recv().await,
            Some(SignInEvent::Failed(SignInError::InvalidStorePath))
        );
        assert_eq!(ui.events.recv().await, None);
    });
}

#[test]
fn interactive_rejects_wrong_mode_without_leaving_the_prompt_or_creating_a_store() {
    use matrix_engine::signin::{Secret, SignInCommand, SignInError};
    let root = Root::new();
    let (mut ui, pending) = interactive(root.0.clone(), None);
    runtime().block_on(async {
        let consumer = async {
            let _ = ui.events.recv().await.unwrap();
            assert!(
                ui.commands
                    .try_send(SignInCommand::Unlock {
                        phrase: Secret::new("fixture unlock phrase".into()).unwrap()
                    })
                    .is_ok()
            );
            assert_eq!(
                ui.events.recv().await,
                Some(SignInEvent::Prompt(SignInPrompt {
                    mode: SignInMode::Password,
                    error: Some(SignInError::NoSession)
                }))
            );
            assert!(!root.0.join("account-1").exists());
            drop(ui);
        };
        let (result, ()) = tokio::join!(pending.run(), consumer);
        assert!(result.is_err());
    });
}

#[test]
fn interactive_profile_opens_only_on_the_backend_and_stops_when_the_ui_closes() {
    let root = Root::new();
    let (mut ui, pending) = interactive(root.0.clone(), None);
    assert!(
        !root.0.exists(),
        "constructing a UI route must not touch storage"
    );
    runtime().block_on(async {
        let consumer = async {
            assert_eq!(
                ui.events.recv().await,
                Some(SignInEvent::Prompt(SignInPrompt {
                    mode: SignInMode::Password,
                    error: None
                }))
            );
            assert!(root.0.join(".profile-lock").is_file());
            drop(ui);
        };
        let (result, ()) = tokio::join!(pending.run(), consumer);
        assert!(result.is_err());
    });
    assert!(
        matrix_engine::signin::SignInProfile::open(root.0.clone()).is_ok(),
        "UI close releases backend profile ownership"
    );
}

mod vault_route {
    use super::*;
    use matrix_engine::persistence::AccountStore;
    use matrix_engine::signin::{
        ProtectionSource, Secret, SignInCommand, SignInError, VaultError, interactive_with_source,
    };
    use std::sync::{Arc, atomic::AtomicU32};

    #[derive(Clone, Default)]
    struct Fake(Arc<AtomicU32>);
    impl ProtectionSource for Fake {
        fn create(&self, _: &AccountStore) -> Result<Secret, VaultError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(VaultError::Unavailable)
        }
        fn read(&self, _: &AccountStore) -> Result<Secret, VaultError> {
            Err(VaultError::Missing)
        }
    }

    #[test]
    fn vault_password_without_a_phrase_reports_network_failure_and_retries_without_touching_the_vault()
     {
        let root = Root::new();
        let fake = Fake::default();
        let (mut ui, pending) =
            interactive_with_source(root.0.clone(), None, Box::new(fake.clone()));
        runtime().block_on(async {
            let consumer = async {
                assert_eq!(
                    ui.events.recv().await,
                    Some(SignInEvent::Prompt(SignInPrompt {
                        mode: SignInMode::Password,
                        error: None
                    }))
                );
                for _ in 0..2 {
                    assert!(
                        ui.commands
                            .try_send(SignInCommand::VaultPassword {
                                homeserver: "https://127.0.0.1:1/".into(),
                                user: "@alice:example.test".into(),
                                password: Secret::new("fixture password".into()).unwrap(),
                            })
                            .is_ok()
                    );
                    assert_eq!(
                        ui.events.recv().await,
                        Some(SignInEvent::Prompt(SignInPrompt {
                            mode: SignInMode::Password,
                            error: Some(SignInError::Discovery)
                        }))
                    );
                }
                // A local-phrase password command is refused on a vault route.
                assert!(
                    ui.commands
                        .try_send(SignInCommand::Password {
                            homeserver: "https://127.0.0.1:1/".into(),
                            user: "@alice:example.test".into(),
                            password: Secret::new("fixture password".into()).unwrap(),
                            phrase: Secret::new("fixture unlock phrase".into()).unwrap(),
                        })
                        .is_ok()
                );
                assert_eq!(
                    ui.events.recv().await,
                    Some(SignInEvent::Prompt(SignInPrompt {
                        mode: SignInMode::Password,
                        error: Some(SignInError::ProtectionMismatch)
                    }))
                );
                drop(ui);
            };
            let (result, ()) = tokio::join!(pending.run(), consumer);
            assert!(result.is_err());
        });
        assert_eq!(fake.0.load(Ordering::Relaxed), 0);
    }
}

struct NoVault;
impl matrix_engine::signin::ProtectionSource for NoVault {
    fn create(
        &self,
        _: &matrix_engine::persistence::AccountStore,
    ) -> Result<matrix_engine::signin::Secret, matrix_engine::signin::VaultError> {
        Err(matrix_engine::signin::VaultError::Unavailable)
    }
    fn read(
        &self,
        _: &matrix_engine::persistence::AccountStore,
    ) -> Result<matrix_engine::signin::Secret, matrix_engine::signin::VaultError> {
        Err(matrix_engine::signin::VaultError::Unavailable)
    }
}
fn first_mode(binding: Option<&[u8]>, with_source: bool) -> SignInMode {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let root = Root::new();
    std::fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    let write = |name: &str, bytes: &[u8]| {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.0.join(name))
            .unwrap()
            .write_all(bytes)
            .unwrap();
    };
    write("active-account", b"1");
    if let Some(binding) = binding {
        write("protection-mode", binding);
    }
    let (mut ui, pending) = if with_source {
        matrix_engine::signin::interactive_with_source(root.0.clone(), None, Box::new(NoVault))
    } else {
        interactive(root.0.clone(), None)
    };
    runtime().block_on(async {
        let consumer = async {
            let Some(SignInEvent::Prompt(prompt)) = ui.events.recv().await else {
                panic!("expected a prompt");
            };
            drop(ui);
            prompt.mode
        };
        tokio::join!(pending.run(), consumer).1
    })
}
#[test]
fn restoring_a_vault_profile_prompts_without_a_phrase_but_legacy_profiles_keep_unlock() {
    assert_eq!(first_mode(Some(b"vault-v1"), true), SignInMode::UnlockVault);
    assert_eq!(first_mode(None, true), SignInMode::Unlock);
    assert_eq!(first_mode(None, false), SignInMode::Unlock);
}
