//! Sign-in and restore for the egui frontend: the same engine route and form state machine as the
//! GPUI app (`SignInForm`), so messages, vault restore and error categories stay identical.
use matrix_desktop::{AppDirs, DemoHistory, FormStatus, HistorySession, SignInForm};
use matrix_engine::{
    Receiver, RoomListEvent, RoomSyncStatus,
    room_sync::RoomListWorker,
    signin::{self, RecoveryState, SignInCommand, SignInEvent, SignInMode, SignInUi},
};

pub struct Session {
    pub form: SignInForm,
    ui: SignInUi,
    /// Kept alive for the whole session: dropping it stops the backend.
    _worker: RoomListWorker,
    rooms: Receiver<RoomListEvent>,
    pub account: Option<matrix_domain::AccountId>,
    pub sidebar: matrix_desktop::Sidebar,
    history: HistorySession,
    pending: Option<matrix_engine::history::HistoryRequest>,
    route: Option<(matrix_engine::Sender<matrix_engine::history::HistoryRequest>, Receiver<matrix_engine::history::HistoryResult>)>,
    pub messages: Option<DemoHistory>,
    send: Option<(matrix_engine::Sender<matrix_engine::sending::SendRequest>, Receiver<matrix_engine::sending::SendEvent>)>,
    drafts: std::collections::HashMap<matrix_domain::RoomScope, String>,
    pub draft: String,
    draft_scope: Option<matrix_domain::RoomScope>,
    pub heights: crate::timeline::Heights,
    pub first_visible: usize,
    pub query: String,
    pub sync: Option<RoomSyncStatus>,
    pub homeserver: String,
    pub user: String,
    pub password: String,
    pub phrase: String,
}

impl Session {
    /// Opens the default profile under the app data directory; a saved account unlocks by itself through the OS vault.
    pub fn start(dirs: &AppDirs) -> Result<Session, String> {
        let root = dirs.data.join("profile");
        matrix_desktop::create_private_dir(&root).map_err(|e| e.to_string())?;
        let vault = signin::platform_source();
        let with_vault = vault.is_some();
        let (ui, pending) = match vault {
            Some(source) => signin::interactive_with_source(root, None, source),
            None => signin::interactive(root, None),
        };
        let (mut worker, rooms) = RoomListWorker::spawn(move || async move {
            pending.run().await.map_err(|_| matrix_engine::auth::AuthError::Storage)
        })
        .map_err(|e| e.to_string())?;
        let route = worker.take_history_route();
        let send = worker.take_send_route();
        let mut form = SignInForm::default();
        form.vault = with_vault;
        Ok(Session {
            form,
            ui,
            _worker: worker,
            rooms,
            account: None,
            history: HistorySession::new(),
            pending: None,
            route,
            send,
            drafts: Default::default(),
            draft: String::new(),
            draft_scope: None,
            messages: None,
            heights: Default::default(),
            first_visible: usize::MAX,
            sidebar: matrix_desktop::Sidebar::connected(matrix_domain::AccountId::new(0)),
            query: String::new(),
            sync: None,
            homeserver: String::new(),
            user: String::new(),
            password: String::new(),
            phrase: String::new(),
        })
    }

    /// Drains what the backend sent since the last frame. True when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        // First-time recovery setup is not offered here (S7); a key that arrives is dropped, which zeroizes it.
        while self.ui.setup.try_recv().is_ok() {
            changed = true;
        }
        while let Ok(event) = self.ui.events.try_recv() {
            changed = true;
            self.form.apply(event);
            match event {
                SignInEvent::Prompt(p) if p.mode == SignInMode::UnlockVault && p.error.is_none() => self.send_submit(),
                SignInEvent::Authenticated(account) => {
                    self.account = Some(account);
                    self.sidebar.set_navigation_context(account, true);
                }
                SignInEvent::Recovery(RecoveryState::Offered | RecoveryState::Failed(_) | RecoveryState::Unsupported) => {
                    let _ = self.ui.commands.try_send(SignInCommand::DeferRecovery);
                }
                _ => {}
            }
        }
        while let Ok(event) = self.rooms.try_recv() {
            changed = true;
            match event {
                RoomListEvent::Status(s) => self.sync = Some(s),
                RoomListEvent::Diff(diff) => {
                    self.sidebar.apply_diff(diff);
                }
                RoomListEvent::Order(order) => self.sidebar.set_order(order),
                // Alerts and timeline activity belong to later slices.
                _ => {}
            }
        }
        changed |= self.drive_history();
        changed
    }

    /// Opens the selected room's history, applies the pages that came back and asks for older ones near the top.
    fn drive_history(&mut self) -> bool {
        let mut changed = false;
        let scope = self.sidebar.selected().map(|r| r.scope.clone());
        if let Some(request) = self.history.select(scope.clone()) {
            self.messages = Some(DemoHistory::new(request.tag.scope().clone(), 0));
            self.heights = Default::default();
            self.pending = Some(request);
            changed = true;
        } else if scope.is_none() && self.messages.take().is_some() {
            changed = true;
        }
        // The draft follows the room: stored when leaving, restored when coming back.
        if self.draft_scope != scope {
            if let Some(old) = self.draft_scope.take() {
                self.drafts.insert(old, std::mem::take(&mut self.draft));
            }
            self.draft = scope.as_ref().and_then(|s| self.drafts.remove(s)).unwrap_or_default();
            self.draft_scope = scope;
        }
        if let Some((_, rx)) = &mut self.send {
            // Outcomes show up as the message's own row once the live view refreshes (S5: no separate failure UI yet).
            while rx.try_recv().is_ok() {}
        }
        let mut pages = Vec::new();
        if let Some((_, rx)) = &mut self.route {
            while let Ok(result) = rx.try_recv() {
                pages.push(result);
            }
        }
        for result in pages {
            let live = self.history.is_live(&result.tag);
            let events = if live { self.history.apply_live(result) } else { self.history.apply(result) };
            if let (Some(events), Some(messages)) = (events, &mut self.messages) {
                if live { messages.refresh_server(events); } else { messages.prepend_server(events); }
                changed = true;
            }
        }
        if self.pending.is_none() {
            self.pending = self.history.refresh();
        }
        if self.pending.is_none() && self.messages.is_some() {
            self.pending = self.history.load_older_near_top(self.first_visible);
        }
        if let (Some(request), Some((tx, _))) = (self.pending.take(), &mut self.route) {
            if let Err(e) = tx.try_send(request) {
                match e {
                    matrix_engine::TrySendError::Full(r) | matrix_engine::TrySendError::Busy(r) => self.pending = Some(r),
                    matrix_engine::TrySendError::Closed(_) => {}
                }
            }
        }
        changed
    }

    /// Sends the text to the open room; the live view picks up the echo.
    pub fn send(&mut self, body: String) {
        let (Some(scope), Some((tx, _))) = (self.draft_scope.clone(), &mut self.send) else { return };
        let request = matrix_engine::sending::SendRequest {
            scope: scope.clone(),
            transaction_id: matrix_engine::sending::fresh_transaction_id(),
            body,
            attachment: None,
            poll: None,
            mentions: Vec::new(),
        };
        if tx.try_send(request).is_ok() {
            self.history.activity(&scope);
        }
    }

    /// Hands the typed fields to the form and sends the command it builds, if it is ready.
    pub fn submit(&mut self) {
        self.form.homeserver = self.homeserver.trim().to_owned();
        self.form.user = self.user.trim().to_owned();
        self.form.password.replace(None, &self.password);
        self.form.phrase.replace(None, &self.phrase);
        self.password.clear();
        self.phrase.clear();
        self.send_submit();
    }

    fn send_submit(&mut self) {
        if let Some(command) = self.form.submit() {
            let _ = self.ui.commands.try_send(command);
        }
    }

    pub fn status(&self) -> FormStatus {
        self.form.status()
    }
}
