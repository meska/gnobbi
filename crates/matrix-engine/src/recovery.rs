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
}

/// Caller must arrange secure, explicit delivery; this is not a UI DTO.
/// No automatic clipboard, file, diagnostic or screenshot integration.
pub struct RecoverySecret(zeroize::Zeroizing<String>);
impl zeroize::ZeroizeOnDrop for RecoverySecret {}
impl RecoverySecret {
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
}
