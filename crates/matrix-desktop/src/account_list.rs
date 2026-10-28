//! View model of the drawer account list: one row per catalog account that has a user ID.
use crate::AccountCatalog;
use matrix_domain::AccountId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub id: AccountId,
    pub name: String,
    pub user_id: String,
    pub initials: String,
    /// The account this window is signed in with.
    pub current: bool,
}

/// Two-letter avatar text of a name or Matrix ID.
pub fn initials(name: &str) -> String {
    name.trim_start_matches('@').chars().take(2).collect()
}

pub fn account_rows(catalog: &AccountCatalog, current_user_id: Option<&str>) -> Vec<AccountRow> {
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
                name,
                user_id,
            })
        })
        .collect()
}
