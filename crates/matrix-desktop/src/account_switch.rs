//! Several accounts in one window: every catalog account syncs in its own worker, the window
//! shows one. A switch parks the shown account (its worker keeps running, see
//! `AccountState::pump`) and clears every view that depended on it.
use crate::*;
use matrix_desktop::AccountState;

/// Starts a worker for each other catalog account that can unlock from the OS vault without
/// asking (a profile busy in another window, a phrase profile or a missing vault entry is
/// skipped). Also returns the catalog ID of `active_root`.
#[cfg(all(feature = "live", unix))]
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
            history: worker.take_history_route(),
        };
        let mut state = AccountState::new(store_id, Some((root, ca_file.clone())), routes);
        state.control = Some(worker.control());
        accounts.push((entry.id, state));
        workers.push(worker);
    }
    (workers, accounts, active)
}

impl Shell {
    /// Keeps the background accounts syncing; their new-message alerts go out like the shown one's.
    pub(crate) fn pump_parked(&mut self) {
        for state in self.parked.values_mut() {
            for alert in state.pump() {
                if let Some(handle) = &self.notification_handle {
                    handle.submit(alert);
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
        self.active_id.is_some_and(|active| active != id)
            && self
                .parked
                .get(&id)
                .is_some_and(|s| s.status != RoomSyncStatus::AuthenticationFailed)
    }

    /// Shows account `id` (catalog ID). The one shown so far keeps syncing in the background.
    pub(crate) fn switch_account(
        &mut self,
        id: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_menu_open(false, window, cx);
        if !self.can_switch_to(id) {
            return;
        }
        let (Some(current), Some(next)) = (self.active_id, self.parked.remove(&id)) else {
            return;
        };
        let mut leaving = self.take_state();
        leaving.park();
        self.parked.insert(current, leaving);
        self.put_state(next);
        self.active_id = Some(id);
        self.clear_account_views(cx);
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
        self.members_requested = None;
        self.read_dirty = false;
        self.confirm_redaction = None;
        self.room_menu = None;
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
