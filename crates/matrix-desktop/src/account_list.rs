//! View model of the drawer account list: one row per catalog account that has a user ID.
use crate::AccountCatalog;
use matrix_domain::AccountId;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub id: AccountId,
    pub name: String,
    pub user_id: String,
    pub initials: String,
    /// Picture (mxc) to show instead of the initials; None or empty means initials.
    pub avatar: Option<String>,
    /// The account this window is signed in with.
    pub current: bool,
    /// Unread messages and mentions of the account (all its rooms), from its own sync.
    pub unread: u64,
    pub mentions: u64,
}

/// Two-letter avatar text of a name or Matrix ID.
pub fn initials(name: &str) -> String {
    name.trim_start_matches('@').chars().take(2).collect()
}

pub fn account_rows(
    catalog: &AccountCatalog,
    current_user_id: Option<&str>,
    counts: &HashMap<AccountId, (u64, u64)>,
) -> Vec<AccountRow> {
    catalog
        .accounts()
        .iter()
        .filter_map(|entry| {
            let user_id = entry.user_id.clone()?;
            let name = if entry.label.is_empty() {
                user_id.clone()
            } else {
                entry.label.clone()
            };
            Some(AccountRow {
                id: entry.id,
                initials: initials(&name),
                avatar: matrix_engine::avatar::normalize_avatar(entry.avatar.clone()),
                current: current_user_id == Some(user_id.as_str()),
                unread: counts.get(&entry.id).map_or(0, |c| c.0),
                mentions: counts.get(&entry.id).map_or(0, |c| c.1),
                name,
                user_id,
            })
        })
        .collect()
}

impl AccountRow {
    /// The live answer of the account's session (`Some(None)` = it has no picture) wins over the saved one.
    pub fn with_live_avatar(mut self, live: Option<Option<&str>>) -> Self {
        if let Some(live) = live {
            self.avatar = matrix_engine::avatar::normalize_avatar(live.map(str::to_owned));
        }
        self
    }

    /// The picture when it is already loaded, else None: the row then shows the initials.
    pub fn picture<T>(&self, loaded: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        loaded(self.avatar.as_deref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(avatar: Option<&str>) -> AccountRow {
        AccountRow {
            id: AccountId::new(1),
            name: "Ada".into(),
            user_id: "@ada:x".into(),
            initials: "Ad".into(),
            avatar: avatar.map(str::to_owned),
            current: false,
            unread: 0,
            mentions: 0,
        }
    }

    #[test]
    fn live_answer_beats_saved_and_empty_means_none() {
        let saved = row(Some("mxc://s/old"));
        assert_eq!(saved.clone().with_live_avatar(None).avatar.as_deref(), Some("mxc://s/old"));
        assert_eq!(saved.clone().with_live_avatar(Some(Some("mxc://s/new"))).avatar.as_deref(), Some("mxc://s/new"));
        assert_eq!(saved.clone().with_live_avatar(Some(None)).avatar, None);
        assert_eq!(saved.with_live_avatar(Some(Some(""))).avatar, None);
    }

    #[test]
    fn picture_only_when_loaded_otherwise_initials() {
        assert_eq!(row(Some("mxc://s/a")).picture(|m| Some(m.len())), Some(9));
        assert_eq!(row(Some("mxc://s/a")).picture(|_| None::<u8>), None); // not loaded yet
        assert_eq!(row(None).picture(|_| Some(1)), None);
    }
}
