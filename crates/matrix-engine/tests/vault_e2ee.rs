#![cfg(all(feature = "linux-vault", target_os = "linux"))]
#![recursion_limit = "256"]
//! Encrypted history across real client restarts, for a vault-protected profile
//! (alice) and a legacy local-phrase profile (bob), both with local account 1.
//! One phase per process, so a phase boundary is a genuine restart. Driven by
//! the vault_e2e script inside a private namespace; disposable accounts only.
use futures_util::FutureExt;
use matrix_domain::{RoomId, RoomScope, TransactionId};
use matrix_engine::{
    RoomListEvent, RoomSyncStatus, ViewRequests,
    auth::AccountSession,
    room_sync::RoomSync,
    sending::SendRequest,
    signin::{Secret, SignInError, SignInProfile, VaultError},
    vault::SecretService,
};
use std::path::PathBuf;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

struct Who {
    key: &'static str,
    peer: &'static str,
    name: &'static str,
    vault: bool,
}
fn who() -> Who {
    match env("DIME_E2E_WHO").as_str() {
        "v" => Who {
            key: "v",
            peer: "l",
            name: "alice",
            vault: true,
        },
        "l" => Who {
            key: "l",
            peer: "v",
            name: "bob",
            vault: false,
        },
        other => panic!("unknown profile {other}"),
    }
}

async fn identity(session: &AccountSession) -> String {
    let who = session.whoami().await.unwrap();
    let key = session.ed25519_key().await.unwrap();
    format!("{}\n{}\n{}", who.user_id(), who.device_id(), key)
}

async fn open(
    profile: &mut SignInProfile,
    who: &Who,
    ca: Vec<u8>,
) -> Result<AccountSession, SignInError> {
    if who.vault {
        profile.unlock_vault(&SecretService, Some(ca)).await
    } else {
        profile
            .unlock(Secret::new(env("DIME_E2E_PHRASE"))?, Some(ca))
            .await
    }
}

/// Runs the live sliding path until its first Ready (keys uploaded, rooms and members synced).
async fn sync_twice(sync: &mut RoomSync) {
    let (mut tx, mut rx) = matrix_engine::bounded(std::num::NonZeroUsize::new(256).unwrap());
    let run = matrix_engine::sliding::run(sync, &mut tx, || {});
    tokio::pin!(run);
    let ready = async {
        while let Some(event) = rx.recv().await {
            if matches!(event, RoomListEvent::Status(RoomSyncStatus::Ready)) {
                return;
            }
        }
        panic!("sync stream closed before Ready");
    };
    tokio::select! {
        _ = &mut run => panic!("sync ended before Ready"),
        () = tokio::time::timeout(std::time::Duration::from_secs(60), ready).map(|r| r.expect("never Ready")) => {}
    }
}

fn scope(session: &AccountSession, room: &str) -> RoomScope {
    RoomScope {
        account_id: session.account_id(),
        room_id: RoomId::new(room),
    }
}

#[test]
#[ignore = "requires the vault_e2e script e2ee"]
fn phase() {
    let who = who();
    let dir = PathBuf::from(env("DIME_E2E_DIR"));
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let password = config[format!("{}_password", who.name)]
        .as_str()
        .unwrap()
        .to_owned();
    let user = format!("@{}:matrix-desktop.test", who.name);
    let expected_identity = dir.join(format!("{}-identity.txt", who.key));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut profile = SignInProfile::open(PathBuf::from(env("DIME_E2E_ROOT"))).unwrap();
    let phase = env("DIME_E2E_PHASE");
    // The SDK client must be dropped inside the live runtime.
    rt.block_on(async {
        match phase.as_str() {
            "login" => {
                let session = if who.vault {
                    profile
                        .password_login_vault(
                            &SecretService,
                            url,
                            user,
                            Secret::new(password).unwrap(),
                            Some(ca),
                        )
                        .await
                } else {
                    profile
                        .password_login(
                            url,
                            user,
                            Secret::new(password).unwrap(),
                            Secret::new(env("DIME_E2E_PHRASE")).unwrap(),
                            Some(ca),
                        )
                        .await
                }
                .unwrap();
                assert_eq!(session.account_id().as_u64(), 1);
                std::fs::write(&expected_identity, identity(&session).await).unwrap();
                sync_twice(&mut RoomSync::new(session)).await;
            }
            "send" | "read" => {
                let session = open(&mut profile, &who, ca).await.unwrap();
                // Same account/device/Ed25519 key as the original login, not a new device.
                assert_eq!(
                    identity(&session).await,
                    std::fs::read_to_string(&expected_identity).unwrap()
                );
                let room = std::fs::read_to_string(dir.join("room.txt")).unwrap();
                let scope = scope(&session, &room);
                let mut sync = RoomSync::new(session);
                if phase == "send" || env("DIME_E2E_SYNC") == "1" {
                    sync_twice(&mut sync).await;
                }
                if phase == "send" {
                    let id = sync
                        .send_text(&SendRequest {
                            scope,
                            transaction_id: TransactionId::new(format!("vault-e2ee-{}", who.key)),
                            body: env("DIME_E2E_OWN_BODY"),
                            attachment: None, mentions: Vec::new(),
                        })
                        .await
                        .unwrap();
                    std::fs::write(dir.join(format!("{}-sent.txt", who.key)), id.as_str()).unwrap();
                } else {
                    let mut views = ViewRequests::default();
                    let tag = views.start(scope).unwrap();
                    let page = sync.history_page(tag, None).await.unwrap();
                    for (key, body, outgoing) in [
                        (who.peer, env("DIME_E2E_PEER_BODY"), false),
                        (who.key, env("DIME_E2E_OWN_BODY"), true),
                    ] {
                        let id =
                            std::fs::read_to_string(dir.join(format!("{key}-sent.txt"))).unwrap();
                        let event = page
                            .events
                            .iter()
                            .find(|e| e.event_id.as_str() == id)
                            .expect("encrypted event present");
                        assert_eq!(event.body, body, "decrypted plaintext");
                        assert_eq!(event.outgoing, outgoing);
                    }
                }
            }
            "fail" => {
                let result = match env("DIME_E2E_EXPECT").as_str() {
                    "vault-on-legacy" | "missing" | "unavailable" | "locked" => {
                        profile.unlock_vault(&SecretService, Some(ca)).await
                    }
                    "phrase-on-vault" | "wrong-phrase" | "recovery-material" => {
                        profile
                            .unlock(Secret::new(env("DIME_E2E_SECRET")).unwrap(), Some(ca))
                            .await
                    }
                    other => panic!("unknown expectation {other}"),
                };
                let error = result.err().expect("unlock must fail");
                match env("DIME_E2E_EXPECT").as_str() {
                    "vault-on-legacy" | "phrase-on-vault" => {
                        assert_eq!(error, SignInError::ProtectionMismatch)
                    }
                    "missing" => assert_eq!(error, SignInError::Vault(VaultError::Missing)),
                    "unavailable" => assert_eq!(error, SignInError::Vault(VaultError::Unavailable)),
                    "locked" => assert_eq!(error, SignInError::Vault(VaultError::Locked)),
                    _ => assert!(!matches!(error, SignInError::ProtectionMismatch)),
                }
            }
            other => panic!("unknown phase {other}"),
        }
    });
}
