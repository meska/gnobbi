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
                current: current_user_id == Some(user_id.as_str()),
                unread: counts.get(&entry.id).map_or(0, |c| c.0),
                mentions: counts.get(&entry.id).map_or(0, |c| c.1),
                name,
                user_id,
            })
        })
        .collect()
}
