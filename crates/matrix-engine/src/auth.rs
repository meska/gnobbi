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
    #[cfg(unix)]
    pub(crate) store: Option<crate::persistence::AccountStore>,
}

pub(crate) fn secure_http_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
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
                .http_client(http)
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
                .send()
                .await
                .map_err(|error| {
                    if matches!(
                        error.client_api_error_kind(),
                        Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                    ) {
                        AuthError::CredentialsRejected
                    } else {
                        AuthError::LoginFailed
                    }
                })?;
            let session = Self {
                account_id,
                client: Some(client),
                #[cfg(unix)]
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
            #[cfg(unix)]
            if let Some(store) = self.store.as_ref() {
                store.forget().map_err(|_| AuthError::Storage)?;
            }
            self.client = None;
        }
        Ok(())
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
