//! Sign-in and restore for the egui frontend: the same engine route and form state machine as the
//! GPUI app (`SignInForm`), so messages, vault restore and error categories stay identical.
use matrix_desktop::{AppDirs, FormStatus, SignInForm};
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
        let (worker, rooms) = RoomListWorker::spawn(move || async move {
            pending.run().await.map_err(|_| matrix_engine::auth::AuthError::Storage)
        })
        .map_err(|e| e.to_string())?;
        let mut form = SignInForm::default();
        form.vault = with_vault;
        Ok(Session {
            form,
            ui,
            _worker: worker,
            rooms,
            account: None,
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
        changed
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
