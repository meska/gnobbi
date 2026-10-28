//! Durable catalog of configured accounts (`accounts.json` in the config dir) and their
//! separate data/cache directories. Never holds secrets. Every listed account may be
//! active (syncing) at the same time: nothing here is a "current account" switch.
use crate::AppDirs;
use matrix_domain::AccountId;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const FILE: &str = "accounts.json";
const VERSION: u64 = 1;
/// Folder of the single-account installs that predate the catalog.
const LEGACY_DIR: &str = "profile";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountEntry {
    pub id: AccountId,
    /// Unknown for a migrated install until its first restore fills it in.
    pub user_id: Option<String>,
    pub homeserver: Option<String>,
    pub label: String,
    /// Relative to the data dir: `accounts/<id>`, or `profile` for a migrated install.
    dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCatalog {
    accounts: Vec<AccountEntry>,
    /// Never reused, even after an account is removed.
    next_id: u64,
    /// The file existed but was unreadable or of an unknown version: it is left untouched
    /// and `save` refuses to overwrite it.
    damaged: bool,
}

impl Default for AccountCatalog {
    fn default() -> Self {
        AccountCatalog { accounts: vec![], next_id: 1, damaged: false }
    }
}

fn safe_relative(dir: &Path) -> bool {
    !dir.as_os_str().is_empty() && dir.components().all(|c| matches!(c, Component::Normal(_)))
}

fn parse(bytes: &[u8]) -> Option<AccountCatalog> {
    let Value::Object(map) = serde_json::from_slice::<Value>(bytes).ok()? else {
        return None;
    };
    if map.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let next_id = map.get("next_id")?.as_u64()?;
    let mut accounts: Vec<AccountEntry> = vec![];
    for item in map.get("accounts")?.as_array()? {
        let id = item.get("id")?.as_u64()?;
        let dir = PathBuf::from(item.get("dir")?.as_str()?);
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
        if id >= next_id || !safe_relative(&dir) || accounts.iter().any(|a| a.id.as_u64() == id) {
            return None;
        }
        accounts.push(AccountEntry {
            id: AccountId::new(id),
            user_id: text("user_id"),
            homeserver: text("homeserver"),
            label: text("label").unwrap_or_default(),
            dir,
        });
    }
    Some(AccountCatalog { accounts, next_id, damaged: false })
}

impl AccountCatalog {
    /// Missing file: empty catalog. Corrupt or unknown version: empty and `damaged`.
    pub fn load(dirs: &AppDirs) -> Self {
        match std::fs::read(dirs.config.join(FILE)) {
            Ok(bytes) => parse(&bytes).unwrap_or(AccountCatalog { damaged: true, ..Default::default() }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(_) => AccountCatalog { damaged: true, ..Default::default() },
        }
    }

    pub fn accounts(&self) -> &[AccountEntry] {
        &self.accounts
    }

    pub fn is_damaged(&self) -> bool {
        self.damaged
    }

    pub fn get(&self, id: AccountId) -> Option<&AccountEntry> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// Adopt a pre-catalog single-account install (`<data>/profile` with content) as the
    /// first entry, in place: no files move, so the existing user sees no change.
    /// Returns true when the catalog changed (the caller saves it).
    pub fn migrate_legacy(&mut self, dirs: &AppDirs) -> bool {
        let used = std::fs::read_dir(dirs.data.join(LEGACY_DIR))
            .is_ok_and(|mut entries| entries.next().is_some());
        if self.damaged || !self.accounts.is_empty() || !used {
            return false;
        }
        let id = self.allocate();
        self.accounts.push(AccountEntry {
            id,
            user_id: None,
            homeserver: None,
            label: String::new(),
            dir: LEGACY_DIR.into(),
        });
        true
    }

    /// Register a new account with its own `accounts/<id>` dir. The caller saves.
    pub fn add(&mut self, user_id: &str, homeserver: &str, label: &str) -> AccountId {
        let id = self.allocate();
        self.accounts.push(AccountEntry {
            id,
            user_id: Some(user_id.into()),
            homeserver: Some(homeserver.into()),
            label: label.into(),
            dir: Path::new("accounts").join(id.as_u64().to_string()),
        });
        id
    }

    fn allocate(&mut self) -> AccountId {
        let id = AccountId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// Store root of the account (SDK store, crypto DB, drafts).
    pub fn data_dir(&self, dirs: &AppDirs, id: AccountId) -> Option<PathBuf> {
        Some(dirs.data.join(&self.get(id)?.dir))
    }

    /// Regenerable media cache of the account, apart from every other account's.
    pub fn cache_dir(&self, dirs: &AppDirs, id: AccountId) -> Option<PathBuf> {
        self.get(id)?;
        Some(dirs.cache.join("accounts").join(id.as_u64().to_string()))
    }

    /// Create the owner-only data and cache dirs of the account.
    pub fn ensure_dirs(&self, dirs: &AppDirs, id: AccountId) -> std::io::Result<()> {
        let missing = || std::io::Error::from(std::io::ErrorKind::NotFound);
        for dir in [
            self.data_dir(dirs, id).ok_or_else(missing)?,
            self.cache_dir(dirs, id).ok_or_else(missing)?,
        ] {
            #[cfg(unix)]
            crate::create_private_dir(&dir)?;
            #[cfg(not(unix))]
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Atomic write (temp file, fsync, rename), owner-only. A failure leaves the previous file.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        if self.damaged {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "damaged catalog is not overwritten",
            ));
        }
        #[cfg(unix)]
        crate::create_private_dir(&dirs.config)?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dirs.config)?;
        let tmp = dirs.config.join(format!("{FILE}.tmp"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let accounts: Vec<Value> = self
            .accounts
            .iter()
            .map(|a| {
                json!({
                    "id": a.id.as_u64(), "user_id": a.user_id, "homeserver": a.homeserver,
                    "label": a.label, "dir": a.dir.to_string_lossy(),
                })
            })
            .collect();
        let body = json!({ "version": VERSION, "next_id": self.next_id, "accounts": accounts });
        let mut file = opts.open(&tmp)?;
        file.write_all(body.to_string().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}
