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
    use matrix_sdk::encryption::{recovery::RecoveryError as Sdk, secret_storage::SecretStorageError as Store};
    match error {
        Sdk::SecretStorage(Store::SecretStorageKey(_)) => RecoveryError::WrongKey,
        Sdk::SecretStorage(Store::MissingKeyInfo { key_id: None }) => RecoveryError::NoSecretStorage,
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
    use matrix_sdk::encryption::{recovery::RecoveryError as Sdk, secret_storage::SecretStorageError as Store};
    let sdk = match error {
        Sdk::Sdk(e) | Sdk::SecretStorage(Store::Sdk(e)) => e,
        Sdk::BackupExistsOnServer => return SetupError::Partial,
        _ => return SetupError::Failed,
    };
    if sdk.as_uiaa_response().is_some() {
        SetupError::AuthRequired
    } else if matches!(sdk.client_api_error_kind(), Some(matrix_sdk::ruma::api::error::ErrorKind::Unrecognized)) {
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
    /// old message exists.
    pub async fn recover_all_room_keys(&mut self, key: &str) -> Result<(), RecoveryError> {
        let client = self.client.as_ref().ok_or(RecoveryError::LoggedOut)?;
        if key.is_empty() || key.len() > 1024 {
            return Err(RecoveryError::InvalidSecret);
        }
        tokio::time::timeout(Duration::from_secs(40), async {
            client.encryption().recovery().recover(key).await.map_err(|e| classify(&e))?;
            if !client.encryption().backups().are_enabled().await {
                return Err(RecoveryError::BackupUnavailable);
            }
            // The room list is only known after a sync; a fresh vault sign-in has not run one.
            client
                .sync_once(matrix_sdk::config::SyncSettings::default())
                .await
                .map_err(|_| RecoveryError::Transport)?;
            for room in client.rooms() {
                client
                    .encryption()
                    .backups()
                    .download_room_keys_for_room(room.room_id())
                    .await
                    .map_err(|_| RecoveryError::Transport)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| RecoveryError::Deadline)?
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
            let storage = encryption.secret_storage().fetch_default_key_id().await.map_err(transport)?.is_some();
            let backup = encryption.backups().fetch_exists_on_server().await.map_err(transport)?;
            match (storage, backup) {
                (true, true) => return Err(SetupError::AlreadyExists),
                (true, false) | (false, true) => return Err(SetupError::Partial),
                (false, false) => (),
            }
            // A published identity without local private keys would be replaced by the bootstrap.
            let complete = encryption.cross_signing_status().await.is_some_and(|status| status.is_complete());
            if !complete && encryption.request_user_identity(&me).await.map_err(transport)?.is_some() {
                return Err(SetupError::IdentityRequired);
            }
            encryption.recovery().enable().wait_for_backups_to_upload().await
                .map(|key| RecoverySecret(zeroize::Zeroizing::new(key)))
                .map_err(|e| classify_setup(&e))
        })
        .await
        .map_err(|_| SetupError::Deadline)?
    }
}
