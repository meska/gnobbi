//! Explicit passphrase-unlocked storage. Linux/Unix only; never falls back to plaintext.
//! All filesystem/KDF work belongs on a backend worker, not the UI thread.
use matrix_domain::AccountId;
use matrix_sdk_store_encryption::StoreCipher;
use serde_json::{Value, json};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cached_history_tests.rs"]
mod cached_history_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    UnsafePath,
    Io,
    LockedOrCorrupt,
    AccountMismatch,
    AlreadyExists,
    WeakPassphrase,
    Busy,
}

/// One caller-allocated local account directory beneath an explicit private root.
/// No implicit OS paths, secret retention, Debug, Clone or logging.
pub struct AccountStore {
    pub(crate) account_id: AccountId,
    pub(crate) path: PathBuf,
    _lock: fs::File,
    pub(crate) avatars: Box<crate::avatar_cache::AvatarCache>,
}

impl AccountStore {
    /// Explicit opt-in durable password login; passphrase is never persisted.
    pub async fn password_login(
        self,
        homeserver: crate::discovery::HomeserverUrl,
        user_id: &str,
        password: &str,
        passphrase: &str,
    ) -> Result<crate::auth::AccountSession, crate::auth::AuthError> {
        let http = crate::auth::secure_http_builder()
            .build()
            .map_err(|_| crate::auth::AuthError::Transport)?;
        self.password_login_with_http(homeserver, user_id, password, passphrase, http)
            .await
    }

    pub(crate) async fn password_login_with_http(
        self,
        homeserver: crate::discovery::HomeserverUrl,
        user_id: &str,
        password: &str,
        passphrase: &str,
        http: reqwest::Client,
    ) -> Result<crate::auth::AccountSession, crate::auth::AuthError> {
        use crate::auth::{AccountSession, AuthError};
        let expected =
            matrix_sdk::ruma::UserId::parse(user_id).map_err(|_| AuthError::InvalidUserId)?;
        if passphrase.chars().count() < 16
            || fs::read_dir(&self.path)
                .map_err(|_| AuthError::Storage)?
                .any(|entry| entry.map_or(true, |e| e.file_name() != ".lock"))
        {
            return Err(AuthError::Storage);
        }
        let client = self.client(&homeserver, passphrase, http).await?;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client
                .matrix_auth()
                .login_username(expected.as_str(), password)
                .send(),
        )
        .await
        .map_err(|_| AuthError::Transport)?
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
        let mut session = AccountSession {
            account_id: self.account_id,
            client: Some(client),
            store: None,
        };
        let identity = session.whoami().await?;
        if identity.user_id() != expected.as_str() {
            return Err(AuthError::IdentityMismatch);
        }
        let sdk_session = session
            .client
            .as_ref()
            .and_then(|c| c.matrix_auth().session())
            .ok_or(AuthError::LoggedOut)?;
        self.seal(
            passphrase,
            &json!({"homeserver": homeserver.as_str(), "session": sdk_session}),
        )
        .map_err(|_| AuthError::Storage)?;
        session.store = Some(self);
        Ok(session)
    }

    /// Decrypt before opening SDK stores, then confirm the exact user/device remotely.
    pub async fn restore(
        self,
        passphrase: &str,
    ) -> Result<crate::auth::AccountSession, crate::auth::AuthError> {
        let http = crate::auth::secure_http_builder()
            .build()
            .map_err(|_| crate::auth::AuthError::Transport)?;
        self.restore_with_http(passphrase, http).await
    }

    /// Explicit per-connection CA opt-in, e.g. a private homeserver's root.
    /// Adds trust without bypassing hostname, expiry or certificate verification.
    /// Does not mutate the OS trust store; never load a server-supplied certificate.
    pub async fn restore_with_trusted_root(
        self,
        passphrase: &str,
        pem: &[u8],
    ) -> Result<crate::auth::AccountSession, crate::auth::AuthError> {
        let cert =
            reqwest::Certificate::from_pem(pem).map_err(|_| crate::auth::AuthError::Transport)?;
        let http = crate::auth::secure_http_builder()
            .add_root_certificate(cert)
            .build()
            .map_err(|_| crate::auth::AuthError::Transport)?;
        self.restore_with_http(passphrase, http).await
    }

    pub(crate) async fn restore_with_http(
        self,
        passphrase: &str,
        http: reqwest::Client,
    ) -> Result<crate::auth::AccountSession, crate::auth::AuthError> {
        use crate::auth::{AccountSession, AuthError};
        let value = self.unseal(passphrase).map_err(|_| AuthError::Storage)?;
        let homeserver = crate::discovery::HomeserverUrl::parse(
            value["homeserver"].as_str().ok_or(AuthError::Storage)?,
        )
        .map_err(|_| AuthError::Storage)?;
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).map_err(|_| AuthError::Storage)?;
        let expected = sdk_session.meta.clone();
        let client = self.client(&homeserver, passphrase, http).await?;
        client
            .restore_session(sdk_session)
            .await
            .map_err(|_| AuthError::Storage)?;
        let session = AccountSession {
            account_id: self.account_id,
            client: Some(client),
            store: Some(self),
        };
        // shortcut: a hanging network is cut at 3 s so saved rooms show; a slower valid server re-checks at sync.
        let checked = tokio::time::timeout(std::time::Duration::from_secs(3), session.whoami())
            .await
            .unwrap_or(Err(AuthError::Transport));
        match checked {
            Ok(identity)
                if identity.user_id() != expected.user_id.as_str()
                    || identity.device_id() != expected.device_id.as_str() =>
            {
                return Err(AuthError::IdentityMismatch);
            }
            // Unreachable homeserver: keep the saved login; a rejected token still fails.
            Err(AuthError::Transport) | Ok(_) => {}
            Err(error) => return Err(error),
        }
        Ok(session)
    }

    async fn client(
        &self,
        homeserver: &crate::discovery::HomeserverUrl,
        passphrase: &str,
        http: reqwest::Client,
    ) -> Result<matrix_sdk::Client, crate::auth::AuthError> {
        Self::private_directory(&self.path).map_err(|_| crate::auth::AuthError::Storage)?;
        Self::regular_files(&self.path).map_err(|_| crate::auth::AuthError::Storage)?;
        let client = matrix_sdk::Client::builder()
            .homeserver_url(homeserver.as_str())
            .disable_well_known_lookup(true)
            .respect_login_well_known(false)
            .http_client(http)
            .request_config(
                matrix_sdk::config::RequestConfig::new()
                    .retry_limit(0)
                    .timeout(std::time::Duration::from_secs(10)),
            )
            .sqlite_store(&self.path, Some(passphrase))
            .build()
            .await
            .map_err(|_| crate::auth::AuthError::Storage)?;
        // The sqlite store above also holds the event cache, sealed with the same key.
        client
            .event_cache()
            .subscribe()
            .map_err(|_| crate::auth::AuthError::Storage)?;
        Ok(client)
    }

    pub fn create(root: &Path, account_id: AccountId) -> Result<Self, StoreError> {
        Self::private_directory(root)?;
        let path = root.join(format!("account-{}", account_id.as_u64()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    StoreError::AlreadyExists
                } else {
                    StoreError::Io
                }
            })?;
        let lock = Self::lock(&path)?;
        Ok(Self {
            account_id,
            avatars: Box::new(crate::avatar_cache::AvatarCache::new(avatar_dir(&path))),
            path,
            _lock: lock,
        })
    }

    pub fn open(root: &Path, account_id: AccountId) -> Result<Self, StoreError> {
        Self::private_directory(root)?;
        let path = root.join(format!("account-{}", account_id.as_u64()));
        Self::private_directory(&path)?;
        Self::regular_files(&path)?;
        let lock = Self::lock(&path)?;
        Ok(Self {
            account_id,
            avatars: Box::new(crate::avatar_cache::AvatarCache::new(avatar_dir(&path))),
            path,
            _lock: lock,
        })
    }

    fn lock(path: &Path) -> Result<fs::File, StoreError> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.join(".lock"))
            .map_err(|_| StoreError::Io)?;
        file.try_lock().map_err(|_| StoreError::Busy)?;
        Ok(file)
    }

    fn regular_files(path: &Path) -> Result<(), StoreError> {
        for entry in fs::read_dir(path).map_err(|_| StoreError::UnsafePath)? {
            let meta = fs::symlink_metadata(entry.map_err(|_| StoreError::UnsafePath)?.path())
                .map_err(|_| StoreError::UnsafePath)?;
            if !meta.is_file() || meta.nlink() != 1 {
                return Err(StoreError::UnsafePath);
            }
        }
        Ok(())
    }

    pub(crate) fn private_directory(path: &Path) -> Result<(), StoreError> {
        let meta = fs::symlink_metadata(path).map_err(|_| StoreError::UnsafePath)?;
        if !path.is_absolute()
            || !meta.is_dir()
            || meta.permissions().mode() & 0o777 != 0o700
            || fs::canonicalize(path).map_err(|_| StoreError::UnsafePath)? != path
        {
            return Err(StoreError::UnsafePath);
        }
        Ok(())
    }

    pub(crate) fn forget(&self) -> Result<(), StoreError> {
        Self::private_directory(&self.path)?;
        match fs::remove_dir_all(self.avatars.dir()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(StoreError::Io),
            _ => (),
        }
        // The credential goes first, then the whole saved cache (state, crypto, events, media).
        let mut names = vec![std::ffi::OsString::from("session.sealed")];
        for entry in fs::read_dir(&self.path).map_err(|_| StoreError::Io)? {
            let name = entry.map_err(|_| StoreError::Io)?.file_name();
            if name != ".lock" && name != "session.sealed" {
                names.push(name);
            }
        }
        for name in names {
            match fs::remove_file(self.path.join(name)) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err(StoreError::Io),
            }
        }
        fs::File::open(&self.path)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| StoreError::Io)
    }

    pub(crate) fn seal(&self, passphrase: &str, value: &Value) -> Result<(), StoreError> {
        if passphrase.chars().count() < 16 {
            return Err(StoreError::WeakPassphrase);
        }
        Self::private_directory(&self.path)?;
        let cipher = StoreCipher::new().map_err(|_| StoreError::LockedOrCorrupt)?;
        let export = cipher
            .export(passphrase)
            .map_err(|_| StoreError::LockedOrCorrupt)?;
        let encrypted = cipher
            .encrypt_value(&json!({"account": self.account_id.as_u64(), "value": value}))
            .map_err(|_| StoreError::LockedOrCorrupt)?;
        let bytes =
            serde_json::to_vec(&json!({"version": 1, "cipher": export, "payload": encrypted}))
                .map_err(|_| StoreError::LockedOrCorrupt)?;
        // Publish only a fully written encrypted record; never overwrite another session.
        let pending = self.path.join("session.pending");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&pending)
            .map_err(|_| StoreError::AlreadyExists)?;
        let result = (|| {
            file.write_all(&bytes).map_err(|_| StoreError::Io)?;
            file.sync_all().map_err(|_| StoreError::Io)?;
            fs::hard_link(&pending, self.path.join("session.sealed"))
                .map_err(|_| StoreError::AlreadyExists)?;
            fs::File::open(&self.path)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| StoreError::Io)
        })();
        let cleanup = fs::remove_file(pending).map_err(|_| StoreError::Io);
        result.and(cleanup)
    }

    pub(crate) fn unseal(&self, passphrase: &str) -> Result<Value, StoreError> {
        Self::private_directory(&self.path)?;
        let path = self.path.join("session.sealed");
        let meta = fs::symlink_metadata(&path).map_err(|_| StoreError::Io)?;
        if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 || meta.len() > 1_048_576 {
            return Err(StoreError::UnsafePath);
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|_| StoreError::Io)?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|_| StoreError::Io)?;
        if bytes.len() > 1_048_576 {
            return Err(StoreError::UnsafePath);
        }
        let record: Value =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::LockedOrCorrupt)?;
        if record["version"] != 1 {
            return Err(StoreError::LockedOrCorrupt);
        }
        let exported: Vec<u8> = serde_json::from_value(record["cipher"].clone())
            .map_err(|_| StoreError::LockedOrCorrupt)?;
        let payload: Vec<u8> = serde_json::from_value(record["payload"].clone())
            .map_err(|_| StoreError::LockedOrCorrupt)?;
        let cipher =
            StoreCipher::import(passphrase, &exported).map_err(|_| StoreError::LockedOrCorrupt)?;
        let plain: Value = cipher
            .decrypt_value(&payload)
            .map_err(|_| StoreError::LockedOrCorrupt)?;
        if plain["account"].as_u64() != Some(self.account_id.as_u64()) {
            return Err(StoreError::AccountMismatch);
        }
        Ok(plain["value"].clone())
    }
}

/// Beside the account dir, not inside it: that dir may hold regular files only.
fn avatar_dir(account: &Path) -> PathBuf {
    let mut name = account.file_name().unwrap_or_default().to_owned();
    name.push("-avatars");
    account.with_file_name(name)
}
