//! Account-scoped password authentication. Run on a backend runtime.
//! Default login is in-memory; Unix persistence is an explicit AccountStore opt-in.
//! No public session/token export, logging or implicit sync.
use crate::discovery::HomeserverUrl;
use matrix_domain::AccountId;
use matrix_sdk::{Client, config::RequestConfig};
use std::time::Duration;
#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;

/// Payload-free failures; SDK error bodies and credentials never cross the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    Transport,
    InvalidUserId,
    LoginFailed,
    RateLimited,
    CredentialsRejected,
    IdentityMismatch,
    LoggedOut,
    SessionExpired,
    Storage,
}

/// Server-confirmed identity, deliberately not Debug/Display.
pub struct SessionIdentity {
    user_id: String,
    device_id: String,
}
impl SessionIdentity {
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

/// One local account's private SDK client. Not Clone/Debug: no token exposure.
pub struct AccountSession {
    pub(crate) account_id: AccountId,
    pub(crate) client: Option<Client>,
    pub(crate) store: Option<crate::persistence::AccountStore>,
}

/// OS only: no user name and no hostname (privacy; a hostname variant needs Marco's consent).
pub(crate) const DEVICE_NAME: &str = if cfg!(target_os = "macos") {
    "Gnobbi (macOS)"
} else if cfg!(target_os = "windows") {
    "Gnobbi (Windows)"
} else if cfg!(target_os = "ios") {
    "Gnobbi (iOS)"
} else if cfg!(target_os = "android") {
    "Gnobbi (Android)"
} else {
    "Gnobbi (Linux)"
};

pub(crate) fn secure_http_builder() -> reqwest::ClientBuilder {
    let http = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    // Android: rustls-platform-verifier is not initialised there, so verify against the embedded roots.
    #[cfg(target_os = "android")]
    let http = http.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der).ok()),
    );
    http
}

/// Extra trust for a private homeserver, added to the platform roots (Android: to the embedded
/// roots of `secure_http_builder`; `tls_certs_only` accumulates).
pub(crate) fn with_trust_roots(
    http: reqwest::ClientBuilder,
    certs: Vec<reqwest::Certificate>,
) -> reqwest::ClientBuilder {
    #[cfg(target_os = "android")]
    return http.tls_certs_only(certs);
    #[cfg(not(target_os = "android"))]
    certs.into_iter().fold(http, |http, cert| http.add_root_certificate(cert))
}

impl AccountSession {
    /// Requires an exact, fully qualified Matrix user ID, not a localpart.
    /// `homeserver` must be the validated/preflighted endpoint explicitly selected
    /// by the caller. Login responses cannot silently change that endpoint.
    pub async fn password_login(
        account_id: AccountId,
        homeserver: HomeserverUrl,
        user_id: &str,
        password: &str,
    ) -> Result<Self, AuthError> {
        let http = secure_http_builder()
            .build()
            .map_err(|_| AuthError::Transport)?;
        Self::password_login_with_http(account_id, homeserver, user_id, password, http).await
    }

    pub(crate) async fn password_login_with_http(
        account_id: AccountId,
        homeserver: HomeserverUrl,
        user_id: &str,
        password: &str,
        http: reqwest::Client,
    ) -> Result<Self, AuthError> {
        // SDK login overrides the client's retry policy with short_retry(); a
        // Retry-After can therefore exceed the per-request transport timeout.
        tokio::time::timeout(Duration::from_secs(10), async {
            let expected =
                matrix_sdk::ruma::UserId::parse(user_id).map_err(|_| AuthError::InvalidUserId)?;
            let client = Client::builder()
                .homeserver_url(homeserver.as_str())
                .disable_well_known_lookup(true)
                .respect_login_well_known(false)
                // Without it a room whose last event is not displayable has no latest event until opened, then jumps up.
                .with_enable_automatic_back_pagination(true)
                // Thread replies fetched by back-pagination must not count in the room badge.
                .with_threading_support(matrix_sdk::ThreadingSupport::Enabled { with_subscriptions: false })
                .http_client(http)
                // Like Element X: fetch a missing session key from the existing backup on first
                // decryption failure; never create backups or cross-signing on the account.
                .with_encryption_settings(matrix_sdk::encryption::EncryptionSettings {
                    auto_enable_cross_signing: false,
                    backup_download_strategy:
                        matrix_sdk::encryption::BackupDownloadStrategy::AfterDecryptionFailure,
                    auto_enable_backups: false,
                })
                .request_config(
                    RequestConfig::new()
                        .retry_limit(0)
                        .timeout(Duration::from_secs(10)),
                )
                .build()
                .await
                .map_err(|_| AuthError::Transport)?;
            client
                .matrix_auth()
                .login_username(expected.as_str(), password)
                .initial_device_display_name(DEVICE_NAME)
                .send()
                .await
                .map_err(|error| {
                    use matrix_sdk::ruma::api::error::ErrorKind;
                    match error.client_api_error_kind() {
                        Some(ErrorKind::Forbidden) => AuthError::CredentialsRejected,
                        Some(ErrorKind::LimitExceeded { .. }) => AuthError::RateLimited,
                        _ => AuthError::LoginFailed,
                    }
                })?;
            let session = Self {
                account_id,
                client: Some(client),
                store: None,
            };
            let identity = session.whoami().await?;
            if identity.user_id() != expected.as_str() {
                return Err(AuthError::IdentityMismatch);
            }
            Ok(session)
        })
        .await
        .map_err(|_| AuthError::Transport)?
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Revoke only this device/session, never logout/all or another account.
    /// On transport failure retain the session so the caller can retry; no false
    /// local success. Successful revocation drops our sole SDK client capability.
    pub async fn logout(&mut self) -> Result<(), AuthError> {
        if let Some(client) = self.client.as_ref() {
            if let Err(error) = client.matrix_auth().logout().await
                && !matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::UnknownToken(_))
                )
            {
                return Err(AuthError::Transport);
            }
            if let Some(store) = self.store.as_ref() {
                store.forget().map_err(|_| AuthError::Storage)?;
            }
            self.client = None;
        }
        Ok(())
    }

    /// Public Ed25519 signing key of this device (base64); identifies the crypto identity across restarts.
    pub async fn ed25519_key(&self) -> Result<String, AuthError> {
        let client = self.client.as_ref().ok_or(AuthError::LoggedOut)?;
        let device = client
            .encryption()
            .get_own_device()
            .await
            .map_err(|_| AuthError::Storage)?
            .ok_or(AuthError::IdentityMismatch)?;
        Ok(device
            .ed25519_key()
            .ok_or(AuthError::IdentityMismatch)?
            .to_base64())
    }

    /// Read back the exact authenticated server identity, not cached login metadata.
    pub async fn whoami(&self) -> Result<SessionIdentity, AuthError> {
        let client = self.client.as_ref().ok_or(AuthError::LoggedOut)?;
        let response = client.whoami().await.map_err(|error| {
            if matches!(
                error.client_api_error_kind(),
                Some(matrix_sdk::ruma::api::error::ErrorKind::UnknownToken(_))
            ) {
                AuthError::SessionExpired
            } else {
                AuthError::Transport
            }
        })?;
        Ok(SessionIdentity {
            user_id: response.user_id.to_string(),
            device_id: response
                .device_id
                .ok_or(AuthError::IdentityMismatch)?
                .to_string(),
        })
    }
}

#[cfg(test)]
mod device_name_tests {
    #[test]
    fn device_name_matches_os_without_user_or_host() {
        let expected = if cfg!(target_os = "macos") { "Gnobbi (macOS)" } else if cfg!(target_os = "windows") { "Gnobbi (Windows)" } else if cfg!(target_os = "ios") { "Gnobbi (iOS)" } else if cfg!(target_os = "android") { "Gnobbi (Android)" } else { "Gnobbi (Linux)" };
        assert_eq!(super::DEVICE_NAME, expected);
    }
}
