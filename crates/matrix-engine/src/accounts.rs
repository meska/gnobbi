//! Backend-owned account selection. No SDK client, token or implicit default export.
//! UI projections and background worker routes must remain separately account-scoped.
use crate::auth::AccountSession;
use matrix_domain::AccountId;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAccount;

#[derive(Default)]
pub struct AccountSessions {
    sessions: HashMap<AccountId, AccountSession>,
    active: Option<AccountId>,
}

impl AccountSessions {
    /// Caller allocates stable local IDs. Never replace an existing capability;
    /// return ownership to the caller on collision or logged-out input (no implicit
    /// revoke or drop). This is local liveness, not a new network authentication check.
    pub fn insert(&mut self, session: AccountSession) -> Result<(), AccountSession> {
        if session.client.is_none() {
            return Err(session);
        }
        match self.sessions.entry(session.account_id()) {
            std::collections::hash_map::Entry::Occupied(_) => Err(session),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(session);
                Ok(())
            }
        }
    }

    /// Selection never logs in, logs out, deletes stores or selects a fallback.
    pub fn switch(&mut self, account: AccountId) -> Result<(), UnknownAccount> {
        if !self.sessions.contains_key(&account) {
            return Err(UnknownAccount);
        }
        self.active = Some(account);
        Ok(())
    }

    /// Revoke only the requested account. Failure preserves selection and ownership
    /// for retry. No implicit switch, no other store deletion, no logout/all.
    /// Call only after that account's separately owned workers have been joined.
    pub async fn logout(&mut self, account: AccountId) -> Result<(), crate::auth::AuthError> {
        let session = self
            .sessions
            .get_mut(&account)
            .ok_or(crate::auth::AuthError::LoggedOut)?;
        session.logout().await?;
        self.sessions.remove(&account);
        if self.active == Some(account) {
            self.active = None;
        }
        Ok(())
    }

    pub fn active(&self) -> Option<&AccountSession> {
        self.active.and_then(|id| self.sessions.get(&id))
    }
}
