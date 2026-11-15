//! Non-destructive SDK recovery operations. Backend runtime only.
//! Secrets are deliberately not Debug/Display/Clone/Serialize; never log them.
use crate::auth::AccountSession;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    LoggedOut,
    Failed,
    Deadline,
    AlreadyConfigured,
    InvalidScope,
    IdentityRequired,
    BackupUnavailable,
    InvalidSecret,
    /// The key does not open the account's secret storage (wrong or malformed).
    WrongKey,
    Transport,
    /// The account has no secret storage, so there is nothing to recover from.
    NoSecretStorage,
}

fn classify(error: &matrix_sdk::encryption::recovery::RecoveryError) -> RecoveryError {
    use matrix_sdk::encryption::{
        recovery::RecoveryError as Sdk, secret_storage::SecretStorageError as Store,
    };
    match error {
        Sdk::SecretStorage(Store::SecretStorageKey(_)) => RecoveryError::WrongKey,
        Sdk::SecretStorage(Store::MissingKeyInfo { key_id: None }) => {
            RecoveryError::NoSecretStorage
        }
        Sdk::Sdk(_) | Sdk::SecretStorage(Store::Sdk(_)) => RecoveryError::Transport,
        _ => RecoveryError::Failed,
    }
}

/// Why a first-time recovery setup did not happen. Everything but `Transport`, `Deadline` and
/// `Failed` is a refusal made before anything was written to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    LoggedOut,
    /// Secret storage and key backup both exist already.
    AlreadyExists,
    /// Exactly one of secret storage and key backup exists; never completed or replaced silently.
    Partial,
    /// The account has an identity this device holds no private keys for: not replaced here.
    IdentityRequired,
    /// The server demands interactive authentication; it is never answered automatically.
    AuthRequired,
    /// The homeserver does not offer key backup.
    Unsupported,
    Transport,
    Deadline,
    Failed,
}

fn classify_setup(error: &matrix_sdk::encryption::recovery::RecoveryError) -> SetupError {
    use matrix_sdk::encryption::{
        recovery::RecoveryError as Sdk, secret_storage::SecretStorageError as Store,
    };
    let sdk = match error {
        Sdk::Sdk(e) | Sdk::SecretStorage(Store::Sdk(e)) => e,
        Sdk::BackupExistsOnServer => return SetupError::Partial,
        _ => return SetupError::Failed,
    };
    if sdk.as_uiaa_response().is_some() {
        SetupError::AuthRequired
    } else if matches!(
        sdk.client_api_error_kind(),
        Some(matrix_sdk::ruma::api::error::ErrorKind::Unrecognized)
    ) {
        SetupError::Unsupported
    } else {
        SetupError::Transport
    }
}

/// Caller must arrange secure, explicit delivery; this is not a UI DTO.
/// No automatic clipboard, file, diagnostic or screenshot integration.
pub struct RecoverySecret(zeroize::Zeroizing<String>);
impl zeroize::ZeroizeOnDrop for RecoverySecret {}
impl RecoverySecret {
    /// Builds a secret from a fixed value, for tests that never reach a homeserver.
    #[doc(hidden)]
    pub fn fixture(key: &str) -> Self {
        Self(zeroize::Zeroizing::new(key.to_owned()))
    }
    /// The recovery key must never be formatted into diagnostics.
    /// ```compile_fail
    /// fn leak(secret: matrix_engine::recovery::RecoverySecret) {
    ///     println!("{secret:?}");
    /// }
    /// ```
    /// ```compile_fail
    /// fn leak(secret: matrix_engine::recovery::RecoverySecret) {
    ///     println!("{secret}");
    /// }
    /// ```
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// `recover_all_room_keys` on a client that may already be syncing: `first_sync` is for the
/// sign-in path, where no sync has run yet; a running session already knows its rooms.
pub async fn recover_all_with(
    client: matrix_sdk::Client,
    key: zeroize::Zeroizing<String>,
    first_sync: bool,
) -> Result<usize, RecoveryError> {
    use futures_util::StreamExt;
    if key.is_empty() || key.len() > 1024 {
        return Err(RecoveryError::InvalidSecret);
    }
    tokio::time::timeout(Duration::from_secs(40), async {
        client
            .encryption()
            .recovery()
            .recover(&key)
            .await
            .map_err(|e| classify(&e))?;
        if !client.encryption().backups().are_enabled().await {
            return Err(RecoveryError::BackupUnavailable);
        }
        // The room list is only known after a sync; a fresh vault sign-in has not run one.
        // The secrets are imported already: a failed sync only skips the prefetch below, the
        // keys are then fetched one by one on first decryption failure.
        if first_sync {
            let _ = crate::sliding::first_sync(&client).await;
        }
        Ok(())
    })
    .await
    .map_err(|_| RecoveryError::Deadline)??;
    let rooms: Vec<matrix_sdk::ruma::OwnedRoomId> = client.rooms().iter().map(|r| r.room_id().to_owned()).collect();
    let backups = client.encryption().backups();
    // Keys the backup hands over arrive on the SDK's per-room stream; their sessions are the count.
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counters: Vec<_> = rooms
        .iter()
        .map(|id| {
            let (mut stream, count) = (Box::pin(backups.room_keys_for_room_stream(id)), count.clone());
            tokio::spawn(async move {
                while let Some(Ok(keys)) = futures_util::StreamExt::next(&mut stream).await {
                    count.fetch_add(keys.values().map(|s| s.len()).sum(), std::sync::atomic::Ordering::Relaxed);
                }
            })
        })
        .collect();
    let failed = futures_util::stream::iter(rooms.iter().cloned())
        .map(|id| download_one(&backups, id))
        .buffer_unordered(8)
        .filter(|ok| std::future::ready(!ok))
        .count()
        .await;
    tokio::task::yield_now().await;
    counters.iter().for_each(|t| t.abort());
    // Every room failing is a network problem the user can retry; some failing is not.
    if failed > 0 && failed == rooms.len() {
        return Err(RecoveryError::Transport);
    }
    Ok(count.load(std::sync::atomic::Ordering::Relaxed))
}

// shortcut: 10 s per room, 8 rooms at once; a failed room is retried by AfterDecryptionFailure.
// A named fn (not a closure) keeps the future `Send` for the service task.
async fn download_one(backups: &matrix_sdk::encryption::backups::Backups, id: matrix_sdk::ruma::OwnedRoomId) -> bool {
    tokio::time::timeout(Duration::from_secs(10), backups.download_room_keys_for_room(&id))
        .await
        .is_ok_and(|r| r.is_ok())
}

/// «Recupero completo»: backup active and the account's cross-signing private keys held here
/// (which also means this device is signed by its owner). Only then «Recupera chiavi» goes away.
pub async fn keys_recovered(client: &matrix_sdk::Client) -> bool {
    let encryption = client.encryption();
    encryption.backups().are_enabled().await
        && encryption.cross_signing_status().await.is_some_and(|s| s.is_complete())
}

impl AccountSession {
    /// Import SDK secrets and download the requested room's encrypted key backup.
    /// This never resets identity, rotates recovery or deletes a backup on error.
    pub async fn recover_room_keys(
        &mut self,
        key: &str,
        scope: &matrix_domain::RoomScope,
    ) -> Result<(), RecoveryError> {
        let client = self.client.as_ref().ok_or(RecoveryError::LoggedOut)?;
        if scope.account_id != self.account_id {
            return Err(RecoveryError::InvalidScope);
        }
        if key.is_empty() || key.len() > 1024 {
            return Err(RecoveryError::InvalidSecret);
        }
        let room_id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
            .map_err(|_| RecoveryError::InvalidScope)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            client
                .encryption()
                .recovery()
                .recover(key)
                .await
                .map_err(|_| RecoveryError::Failed)?;
            if !client.encryption().backups().are_enabled().await {
                return Err(RecoveryError::BackupUnavailable);
            }
            client
                .encryption()
                .backups()
                .download_room_keys_for_room(&room_id)
                .await
                .map_err(|_| RecoveryError::Failed)
        })
        .await
        .map_err(|_| RecoveryError::Deadline)?
    }
    /// Import secrets with an existing recovery key, then download the backed-up keys of every
    /// known room. Never resets, deletes or replaces anything; success is not proof that every
    /// old message exists. The deadline covers only the secret import and the first sync; the
    /// per-room downloads have no overall cap, a slow or failed room never fails the others and
    /// is left to BackupDownloadStrategy::AfterDecryptionFailure (fetched on first use).
    pub async fn recover_all_room_keys(&mut self, key: &str) -> Result<(), RecoveryError> {
        let client = self.client.clone().ok_or(RecoveryError::LoggedOut)?;
        recover_all_with(client, zeroize::Zeroizing::new(key.to_owned()), true).await.map(drop)
    }
    /// Return success only when the SDK confirms all currently known keys uploaded.
    pub async fn upload_key_backup(&mut self) -> Result<(), RecoveryError> {
        let client = self.client.as_ref().ok_or(RecoveryError::LoggedOut)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            client
                .encryption()
                .backups()
                .wait_for_steady_state()
                .await
                .map_err(|_| RecoveryError::Failed)
        })
        .await
        .map_err(|_| RecoveryError::Deadline)?
    }
    /// Create SDK secret storage/backup only on explicit caller request.
    /// Timeout can leave partial remote setup: never delete keys to roll back.
    pub async fn enable_new_recovery(&mut self) -> Result<RecoverySecret, RecoveryError> {
        let client = self.client.as_ref().ok_or(RecoveryError::LoggedOut)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            if client
                .encryption()
                .secret_storage()
                .fetch_default_key_id()
                .await
                .map_err(|_| RecoveryError::Failed)?
                .is_some()
                || client
                    .encryption()
                    .backups()
                    .fetch_exists_on_server()
                    .await
                    .map_err(|_| RecoveryError::Failed)?
            {
                return Err(RecoveryError::AlreadyConfigured);
            }
            if !client
                .encryption()
                .cross_signing_status()
                .await
                .is_some_and(|status| status.is_complete())
            {
                return Err(RecoveryError::IdentityRequired);
            }
            client
                .encryption()
                .recovery()
                .enable()
                .await
                .map(|key| RecoverySecret(zeroize::Zeroizing::new(key)))
                .map_err(|_| RecoveryError::Failed)
        })
        .await
        .map_err(|_| RecoveryError::Deadline)?
    }
    /// First-time setup of secret storage and key backup, only when both are absent on the server.
    /// Uses the SDK bootstrap (which also creates a missing identity); a required interactive
    /// authentication is reported, never answered. A timeout can leave partial remote setup:
    /// nothing is deleted to roll back.
    pub async fn setup_recovery(&mut self) -> Result<RecoverySecret, SetupError> {
        let client = self.client.as_ref().ok_or(SetupError::LoggedOut)?;
        let me = client.user_id().ok_or(SetupError::LoggedOut)?.to_owned();
        let transport = |_| SetupError::Transport;
        tokio::time::timeout(Duration::from_secs(60), async {
            let encryption = client.encryption();
            let storage = encryption
                .secret_storage()
                .fetch_default_key_id()
                .await
                .map_err(transport)?
                .is_some();
            let backup = encryption
                .backups()
                .fetch_exists_on_server()
                .await
                .map_err(transport)?;
            match (storage, backup) {
                (true, true) => return Err(SetupError::AlreadyExists),
                (true, false) | (false, true) => return Err(SetupError::Partial),
                (false, false) => (),
            }
            // A published identity without local private keys would be replaced by the bootstrap.
            let complete = encryption
                .cross_signing_status()
                .await
                .is_some_and(|status| status.is_complete());
            if !complete
                && encryption
                    .request_user_identity(&me)
                    .await
                    .map_err(transport)?
                    .is_some()
            {
                return Err(SetupError::IdentityRequired);
            }
            encryption
                .recovery()
                .enable()
                .wait_for_backups_to_upload()
                .await
                .map(|key| RecoverySecret(zeroize::Zeroizing::new(key)))
                .map_err(|e| classify_setup(&e))
        })
        .await
        .map_err(|_| SetupError::Deadline)?
    }
}
