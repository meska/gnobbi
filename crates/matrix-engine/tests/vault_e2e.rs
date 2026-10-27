#![cfg(all(feature = "linux-vault", target_os = "linux"))]
//! Real homeserver + real Secret Service. One phase per process so that a phase
//! boundary is a genuine client restart. Driven by the vault_e2e script inside a
//! private namespace; never run against a personal account or bus.
use matrix_engine::{
    signin::{Secret, SignInError, SignInProfile, VaultError},
    vault::SecretService,
};
use std::path::PathBuf;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

/// The SDK client must be dropped inside a live runtime, so identity is read
/// and the session released within the same `block_on`.
fn identity(
    rt: &tokio::runtime::Runtime,
    session: matrix_engine::auth::AccountSession,
) -> (String, String) {
    rt.block_on(async move {
        let who = session.whoami().await.unwrap();
        (who.user_id().to_owned(), who.device_id().to_owned())
    })
}

struct Fixture {
    url: String,
    ca: Vec<u8>,
    password: String,
}
fn fixture() -> Fixture {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    Fixture {
        url: config["url"].as_str().unwrap().into(),
        ca: std::fs::read(config["ca"].as_str().unwrap()).unwrap(),
        password: config["alice_password"].as_str().unwrap().into(),
    }
}

const ALICE: &str = "@alice:matrix-desktop.test";

#[test]
#[ignore = "requires the vault_e2e script"]
fn phase() {
    let root = PathBuf::from(env("DIME_E2E_ROOT"));
    let device_file = PathBuf::from(env("DIME_E2E_DEVICE"));
    let fx = fixture();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut profile = SignInProfile::open(root.clone()).unwrap();
    match env("DIME_E2E_PHASE").as_str() {
        "login" => {
            let session = rt
                .block_on(profile.password_login_vault(
                    &SecretService,
                    fx.url,
                    ALICE.into(),
                    Secret::new(fx.password).unwrap(),
                    Some(fx.ca),
                ))
                .unwrap();
            let (user, device) = identity(&rt, session);
            assert_eq!(user, ALICE);
            std::fs::write(device_file, device).unwrap();
        }
        "restore" => {
            let session = rt
                .block_on(profile.unlock_vault(&SecretService, Some(fx.ca)))
                .unwrap();
            let (user, device) = identity(&rt, session);
            assert_eq!(user, ALICE);
            assert_eq!(device, std::fs::read_to_string(device_file).unwrap());
        }
        // Interactive route, no command sent: the backend reads the key itself and opens the profile.
        "restore-auto" => {
            drop(profile);
            let (mut ui, pending) = matrix_engine::signin::interactive_with_source(
                root.clone(),
                Some(fx.ca),
                Box::new(SecretService),
            );
            let event = rt.block_on(async move {
                let (result, event) = tokio::join!(pending.run(), ui.events.recv());
                let session = result.unwrap();
                let who = session.whoami().await.unwrap();
                (event, who.user_id().to_owned(), who.device_id().to_owned())
            });
            assert!(matches!(
                event.0,
                Some(matrix_engine::signin::SignInEvent::Authenticated(_))
            ));
            assert_eq!(event.1, ALICE);
            assert_eq!(event.2, std::fs::read_to_string(device_file).unwrap());
        }
        // Expected Vault(Unavailable) (bus removed) or Vault(Missing) (copied profile).
        "restore-fails" => {
            let expected = match env("DIME_E2E_EXPECT").as_str() {
                "unavailable" => VaultError::Unavailable,
                "missing" => VaultError::Missing,
                other => panic!("unknown expectation {other}"),
            };
            let result = rt.block_on(profile.unlock_vault(&SecretService, Some(fx.ca)));
            assert!(matches!(result, Err(SignInError::Vault(e)) if e == expected));
        }
        "wrong-password" => {
            let result = rt.block_on(profile.password_login_vault(
                &SecretService,
                fx.url.clone(),
                ALICE.into(),
                Secret::new("definitely-not-the-password".into()).unwrap(),
                Some(fx.ca.clone()),
            ));
            assert!(matches!(result, Err(SignInError::Authentication(_))));
            assert!(!root.join("active-account").exists());
            assert!(!root.join("protection-mode").exists());
            // Safe retry on the same profile with the right password.
            let session = rt
                .block_on(profile.password_login_vault(
                    &SecretService,
                    fx.url,
                    ALICE.into(),
                    Secret::new(fx.password).unwrap(),
                    Some(fx.ca),
                ))
                .unwrap();
            assert_eq!(identity(&rt, session).0, ALICE);
        }
        other => panic!("unknown phase {other}"),
    }
}
