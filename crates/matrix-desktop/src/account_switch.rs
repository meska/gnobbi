//! Several accounts in one window: every catalog account syncs in its own worker, the window
//! shows one. A switch parks the shown account (its worker keeps running, see
//! `AccountState::pump`) and clears every view that depended on it.
use crate::*;
use matrix_desktop::AccountState;
use matrix_domain::notifications::Notification;

/// Starts a worker for each other catalog account that can unlock from the OS vault without
/// asking (a profile busy in another window, a phrase profile or a missing vault entry is
/// skipped). Also returns the catalog ID of `active_root`.
#[cfg(feature = "live")]
pub(crate) fn start_background(
    active_root: Option<&(std::path::PathBuf, Option<std::path::PathBuf>)>,
) -> (
    Vec<matrix_engine::room_sync::RoomListWorker>,
    Vec<(AccountId, AccountState)>,
    Option<AccountId>,
) {
    use matrix_engine::{auth::AuthError, signin::SignInProfile};
    use std::io::Read;
    let Some(dirs) =
        matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok())
    else {
        return (vec![], vec![], None);
    };
    let catalog = matrix_desktop::AccountCatalog::load(&dirs);
    let active = active_root.and_then(|(root, _)| catalog.id_for_root(&dirs, root));
    let ca_file = active_root.and_then(|(_, ca)| ca.clone());
    let trust = ca_file.as_ref().and_then(|path| {
        let mut pem = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(1048577)
            .read_to_end(&mut pem)
            .ok()?;
        (pem.len() <= 1048576).then_some(pem)
    });
    let (mut workers, mut accounts) = (vec![], vec![]);
    for entry in catalog.accounts() {
        let Some(root) = catalog.data_dir(&dirs, entry.id) else {
            continue;
        };
        if Some(entry.id) == active
            || entry.user_id.is_none()
            || active_root.is_some_and(|(r, _)| *r == root)
        {
            continue;
        }
        let Some(source) = matrix_engine::signin::platform_source() else {
            continue;
        };
        let source: std::sync::Arc<dyn matrix_engine::signin::ProtectionSource + Send + Sync> =
            source.into();
        let Ok(mut profile) = SignInProfile::open(root.clone()) else {
            continue;
        };
        let Some(store_id) = profile.active_account() else {
            continue;
        };
        // Another window (the "Add account" process) holds the store: the unlock would fail
        // as Storage and the row would claim the sign-in is lost. Leave it out instead.
        if store_busy(&root, store_id) {
            continue;
        }
        let trust = trust.clone();
        let Ok((mut worker, events)) =
            matrix_engine::room_sync::RoomListWorker::spawn(move || async move {
                profile
                    .unlock_vault_timed(source, trust, std::time::Duration::from_secs(15))
                    .await
                    .map_err(|_| AuthError::Storage)
            })
        else {
            continue;
        };
        let routes = matrix_desktop::AccountRoutes {
            events: Some(events),
            media: worker.take_media_route(),
            verification: worker.take_verification_route(),
            operation: worker.take_operation_route(),
            send: worker.take_send_route(),
            send_control: worker.send_control(),
            history: worker.take_history_route(),
            avatars: worker.avatar_slot(),
        };
        let mut state = AccountState::new(store_id, Some((root, ca_file.clone())), routes);
        state.control = Some(worker.control());
        accounts.push((entry.id, state));
        workers.push(worker);
    }
    (workers, accounts, active)
}

/// Whether another window has this account's store open (its `.lock` is taken).
#[cfg(feature = "live")]
fn store_busy(root: &std::path::Path, id: AccountId) -> bool {
    use matrix_engine::persistence::{AccountStore, StoreError};
    matches!(AccountStore::open(root, id), Err(StoreError::Busy))
}

/// Why a switch to a parked account cannot happen, or None when it can.
pub(crate) fn switch_blocker(status: Option<&RoomSyncStatus>) -> Option<&'static str> {
    match status {
        None => Some("Non disponibile"),
        Some(RoomSyncStatus::AuthenticationFailed) => Some("Accedi di nuovo"),
        _ => None,
    }
}

impl Shell {
    /// Keeps the background accounts syncing; their new-message alerts go out like the shown one's.
    pub(crate) fn pump_parked(&mut self) {
        for (id, state) in self.parked.iter_mut() {
            state.notifications.set_message_preview(self.prefs.message_preview);
            for alert in state.pump() {
                if let Some(handle) = &self.notification_handle {
                    handle.submit(Self::alert_from(&self.accounts, Some(*id), alert));
                }
            }
        }
    }

    /// Names the account an alert came from, once there is more than one to tell apart.
    pub(crate) fn alert_from(
        accounts: &matrix_desktop::AccountCatalog,
        id: Option<AccountId>,
        alert: Notification,
    ) -> Notification {
        let entry = id.and_then(|id| accounts.get(id));
        match entry {
            Some(a) if accounts.accounts().len() > 1 => {
                let name = Some(a.label.clone())
                    .filter(|l| !l.is_empty())
                    .or(a.user_id.clone());
                alert.from_account(a.id, name.unwrap_or_default())
            }
            _ => alert,
        }
    }

    /// Unread messages and mentions per catalog account: the shown one from the window, the
    /// others from their own sync.
    pub(crate) fn account_counts(&self) -> HashMap<AccountId, (u64, u64)> {
        let all = &self.sidebar.navigation_entries()[0];
        let shown = self.active_id.map(|id| (id, (all.unread, all.mentions)));
        let parked = self.parked.iter().map(|(id, s)| (*id, s.unread()));
        shown.into_iter().chain(parked).collect()
    }

    /// A clicked alert: shows its account (when it is not the shown one) with its room open.
    pub(crate) fn open_clicked_alert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, scope)) = self
            .notification_handle
            .as_ref()
            .and_then(|h| h.take_click())
        else {
            return;
        };
        self.open_alert(id, scope, window, cx);
    }

    /// Dispatch of one already decoded click (the real wrapper above and the Shell tests share it).
    pub(crate) fn open_alert(
        &mut self,
        id: Option<AccountId>,
        scope: RoomScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.activate_window();
        match id.filter(|id| Some(*id) != self.active_id) {
            None => {
                if self.sidebar.select(&scope) {
                    self.restore.cancel();
                    self.sync_room(cx);
                    cx.notify();
                }
            }
            Some(id) => {
                if self.parked.contains_key(&id) {
                    self.switch_account_with_room(id, Some(&scope), window, cx);
                }
            }
        }
    }

    /// Draft key of the shown room: the catalog ID tells accounts apart, because the ID a
    /// worker reports is only unique inside its own profile folder.
    pub(crate) fn draft_scope(&self) -> RoomScope {
        RoomScope {
            account_id: self.active_id.unwrap_or(self.active_scope.account_id),
            room_id: self.active_scope.room_id.clone(),
        }
    }

    /// Moves everything account-dependent out of the window, leaving blank placeholders.
    fn take_state(&mut self) -> AccountState {
        let blank = AccountId::new(0);
        AccountState {
            account_id: self.active_scope.account_id,
            root: self.account_root.take(),
            routes: std::mem::take(&mut self.routes),
            sidebar: std::mem::replace(&mut self.sidebar, Sidebar::connected(blank)),
            status: std::mem::replace(&mut self.status, RoomSyncStatus::Connecting),
            notifications: std::mem::replace(
                &mut self.notifications,
                matrix_domain::notifications::NotificationPolicy::new(blank),
            ),
            verification: std::mem::replace(
                &mut self.verification,
                matrix_desktop::DeviceVerification::new(blank),
            ),
            delivery: std::mem::take(&mut *self.delivery.borrow_mut()),
            operations: std::mem::take(&mut *self.operations.borrow_mut()),
            media: std::mem::take(&mut self.media),
            control: self.control.take(),
        }
    }

    fn put_state(&mut self, state: AccountState) {
        // Another account's store: the open panel's receipt authorization does not cross over.
        self.revoke_thread_read();
        self.active_scope = RoomScope {
            account_id: state.account_id,
            room_id: matrix_domain::RoomId::new("!not-selected:local.invalid"),
        };
        self.account_root = state.root;
        self.routes = state.routes;
        self.sidebar = state.sidebar;
        self.status = state.status;
        self.notifications = state.notifications;
        self.verification = state.verification;
        *self.delivery.borrow_mut() = state.delivery;
        *self.operations.borrow_mut() = state.operations;
        self.media = state.media;
        self.control = state.control;
    }

    /// Whether `id` is another account that can be shown now.
    pub(crate) fn can_switch_to(&self, id: AccountId) -> bool {
        self.active_id != Some(id)
            && self
                .parked
                .get(&id)
                .is_some_and(|s| switch_blocker(Some(&s.status)).is_none())
    }

    /// Shows account `id` (catalog ID). The one shown so far keeps syncing in the background.
    /// When it cannot be shown the drawer stays open and the row says why.
    pub(crate) fn switch_account(
        &mut self,
        id: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_account_with_room(id, None, window, cx);
    }

    pub(crate) fn switch_account_with_room(
        &mut self,
        id: AccountId,
        room: Option<&RoomScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_id == Some(id) {
            self.set_menu_open(false, window, cx);
            return;
        }
        let status = self.parked.get(&id).map(|s| s.status);
        let blocker = switch_blocker(status.as_ref());
        // No secrets: only catalog IDs and why a click was refused.
        eprintln!(
            "gnobbi: switch to {id:?}: shown={:?} parked={} refused={blocker:?}",
            self.active_id,
            status.is_some()
        );
        if let Some(reason) = blocker {
            self.switch_error = Some((id, reason));
            cx.notify();
            return;
        }
        self.switch_error = None;
        self.set_menu_open(false, window, cx);
        let Some(next) = self.parked.remove(&id) else {
            return;
        };
        let mut leaving = self.take_state();
        // No shown account in the catalog: nothing to park it under, so choosing one just shows it.
        if let Some(current) = self.active_id {
            leaving.park();
            self.parked.insert(current, leaving);
        }
        self.put_state(next);
        self.active_id = Some(id);
        self.clear_account_views(cx);
        self.restore.reset();
        if let Some(room) = room && self.sidebar.select(room) {
            self.restore.cancel();
        }
        self.restore_last_room(cx); // arms for the new account; its room may already be listed
        self.sync_room(cx);
        cx.notify();
    }

    /// Nothing of the previous account may stay on screen or reachable: open room and its
    /// timeline, history requests, selection, panels, search, composer text and attachment.
    /// The previous account's drafts stay saved under its own key.
    pub(crate) fn clear_account_views(&mut self, cx: &mut Context<Self>) {
        self.history = HistorySession::new();
        self.timeline = None;
        self.saved_timelines.clear();
        self.pending_history = None;
        self.pending_operation_command = None;
        self.new_room = None;
        self.quick = None;
        self.pending_open = None;
        self.pending_space = None;
        self.groups_edit = None;
        self.members_requested = None;
        self.room_members.clear();
        self.profile = None;
        self.read_dirty = false;
        self.confirm_redaction = None;
        self.room_menu = None;
        self.confirm_leave = None;
        self.compact_chat = false;
        self.set_room_details_open(false);
        self.set_spaces_open(false);
        self.set_notifications_open(false);
        self.set_verification_open(false);
        self.media.clear();
        self.operations.borrow_mut().clear_selection();
        if let Some(handle) = &self.notification_handle {
            handle.cancel_pending();
        }
        let scope = self.draft_scope();
        self.composer
            .update(cx, |editor, cx| editor.switch_room(scope, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a failed unlock refuses a started account, whether or not one is shown; every refusal fits the one-line row.
    #[test]
    fn blocker_only_for_failed_unlock_and_one_line() {
        for s in [
            RoomSyncStatus::Connecting,
            RoomSyncStatus::Ready,
            RoomSyncStatus::SyncFailed,
        ] {
            assert!(switch_blocker(Some(&s)).is_none());
        }
        let all = [
            switch_blocker(None),
            switch_blocker(Some(&RoomSyncStatus::AuthenticationFailed)),
        ];
        for m in all.into_iter().map(|m| m.expect("refusal")) {
            assert!(!m.is_empty() && !m.contains('\n') && m.chars().count() <= 30);
        }
        assert_ne!(all[0], all[1]);
    }

    /// A store held by another window is reported busy; free or missing ones are not.
    #[cfg(feature = "live")]
    #[test]
    fn store_held_by_another_window_is_busy() {
        use std::os::unix::fs::DirBuilderExt;
        let root = std::env::temp_dir().join(format!("gnobbi-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut dir = std::fs::DirBuilder::new();
        dir.mode(0o700).recursive(true);
        dir.create(root.join("account-5")).unwrap();
        // The store insists on a canonical path (macOS temp is behind a symlink).
        let root = root.canonicalize().unwrap();
        let id = AccountId::new(5);
        assert!(!store_busy(&root, id));
        let held = matrix_engine::persistence::AccountStore::open(&root, id).unwrap();
        assert!(store_busy(&root, id));
        drop(held);
        assert!(!store_busy(&root, id));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
