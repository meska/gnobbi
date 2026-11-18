//! Sign-in and restore for the egui frontend: the same engine route and form state machine as the
//! GPUI app (`SignInForm`), so messages, vault restore and error categories stay identical.
use eframe::egui;
use matrix_desktop::{AppDirs, DemoHistory, FormStatus, HistorySession, LiveMedia, LiveOperations, SignInForm};
use matrix_domain::{EventId, TimelineId};
use matrix_engine::history::MediaKind;
use matrix_engine::media::{MediaCommand, MediaEvent};
use matrix_engine::operations::{Operation, OperationCommand, OperationResult};
use matrix_engine::{
    Receiver, RoomListEvent, RoomSyncStatus,
    room_sync::RoomListWorker,
    signin::{self, RecoveryState, SignInCommand, SignInEvent, SignInMode, SignInUi},
};

/// The open thread: its page as last read, and the line being typed.
pub struct ThreadState {
    pub root: EventId,
    views: matrix_engine::ViewRequests,
    pub page: Option<Box<matrix_engine::history::ThreadPage>>,
    pub failed: bool,
    pub draft: String,
    /// Next re-read; the thread has no live feed of its own here.
    next_read: std::time::Instant,
}

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
    pub ops: LiveOperations,
    /// Joined members of the open room and the server's upload limit.
    pub members: Option<(matrix_domain::RoomScope, Vec<matrix_engine::operations::Member>)>,
    members_requested: Option<matrix_domain::RoomScope>,
    upload_limit: Option<u64>,
    pub info_open: bool,
    pub thread: Option<ThreadState>,
    pub verification: Option<matrix_desktop::DeviceVerification>,
    ver_route: Option<(matrix_engine::Sender<matrix_engine::verification::VerificationCommand>, Receiver<matrix_engine::verification::VerificationEvent>)>,
    pub devices_open: bool,
    pub new_open: bool,
    pub new_term: String,
    new_sent: String,
    pub users: matrix_engine::operations::SearchState,
    pub new_failed: bool,
    pending_select: Option<matrix_domain::RoomScope>,
    /// Commands from the UI waiting for the single send slot.
    op_queue: Vec<OperationCommand>,
    /// Member whose card is open and what the engine answered about them.
    pub profile: Option<(String, Option<matrix_engine::operations::UserProfile>)>,
    pub search_open: bool,
    pub search_term: String,
    search_sent: String,
    pub search: matrix_engine::operations::MessageSearchState,
    media: LiveMedia,
    media_route: Option<(matrix_engine::Sender<MediaCommand>, Receiver<MediaEvent>)>,
    /// Thumbnails of the open room's pictures and videos, by event.
    pub pictures: std::collections::HashMap<EventId, egui::TextureHandle>,
    op_route: Option<(matrix_engine::Sender<OperationCommand>, Receiver<OperationResult>)>,
    op_pending: Option<OperationCommand>,
    /// Message the next Enter replies to or edits; the draft it replaced waits in `parked`.
    pub target: Option<(matrix_domain::ScopedTimelineId, crate::composer::Mode)>,
    parked: String,
    /// Right-click menu: where it opened; "delete" waits for a second click.
    pub menu: Option<eframe::egui::Pos2>,
    /// File dropped on the window, sent with the next Enter (the text is its caption).
    pub attachment: Option<(String, Vec<u8>)>,
    /// Short result line for the last save or refused attachment.
    pub notice: Option<String>,
    pub confirm_delete: bool,
    /// The menu opened this frame: the click that opened it must not close it.
    pub menu_fresh: bool,
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
        let op_route = worker.take_operation_route();
        let ver_route = worker.take_verification_route();
        let media_route = worker.take_media_route();
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
            ops: LiveOperations::default(),
            members: None,
            members_requested: None,
            upload_limit: None,
            info_open: false,
            thread: None,
            verification: None,
            ver_route,
            devices_open: false,
            new_open: false,
            new_term: String::new(),
            new_sent: String::new(),
            users: matrix_engine::operations::SearchState::Idle,
            new_failed: false,
            pending_select: None,
            op_queue: Vec::new(),
            profile: None,
            search_open: false,
            search_term: String::new(),
            search_sent: String::new(),
            search: matrix_engine::operations::MessageSearchState::Idle,
            media: LiveMedia::default(),
            media_route,
            pictures: Default::default(),
            op_route,
            op_pending: None,
            target: None,
            parked: String::new(),
            menu: None,
            attachment: None,
            notice: None,
            confirm_delete: false,
            menu_fresh: false,
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
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
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
                    let mut v = matrix_desktop::DeviceVerification::new(account);
                    v.refresh();
                    self.verification = Some(v);
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
        if let (Some(v), Some((tx, rx))) = (&mut self.verification, &mut self.ver_route) {
            while let Ok(event) = rx.try_recv() {
                v.apply(event);
                changed = true;
            }
            v.flush(tx);
        }
        changed |= self.drive_history();
        changed |= self.drive_media(ctx);
        changed
    }

    /// One thumbnail at a time, newest first; a picture that failed stays a text row.
    fn drive_media(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if let (Some(messages), Some(scope)) = (&self.messages, &self.draft_scope) {
            let mut videos = Vec::new();
            let mut wanted = Vec::new();
            for e in messages.events().iter().rev() {
                let (Some(media), TimelineId::Server(id)) = (&e.media, &e.id.id) else { continue };
                match media.kind {
                    MediaKind::Video => videos.push(id.clone()),
                    MediaKind::Image => {}
                    _ => continue,
                }
                if !self.pictures.contains_key(id) {
                    wanted.push(id.clone());
                }
            }
            self.media.mark_videos(videos);
            self.media.next_inline(scope.clone(), wanted);
        }
        if let Some((name, ok)) = self.media.take_saved().map(|(_, r)| (String::new(), r.is_ok())) {
            let _ = name;
            self.notice = Some(if ok { "Salvato in Download".to_owned() } else { "Salvataggio non riuscito".to_owned() });
            changed = true;
        }
        if let Some((tx, rx)) = &mut self.media_route {
            self.media.flush(tx);
            while let Ok(event) = rx.try_recv() {
                if self.media.apply(event) {
                    changed = true;
                    if let (Some(p), Some(id)) = (self.media.take_preview(), self.media.event().cloned()) {
                        let image = egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba);
                        self.pictures.insert(id.clone(), ctx.load_texture(id.as_str(), image, Default::default()));
                    }
                }
            }
        }
        changed
    }

    /// Opens the selected room's history, applies the pages that came back and asks for older ones near the top.
    fn drive_history(&mut self) -> bool {
        let mut changed = false;
        let scope = self.sidebar.selected().map(|r| r.scope.clone());
        if let Some(request) = self.history.select(scope.clone()) {
            self.messages = Some(DemoHistory::new(request.tag.scope().clone(), 0));
            self.media.clear();
            self.pictures.clear();
            self.members = None;
            self.thread = None;
            self.upload_limit = None;
            self.search_sent.clear();
            self.search = matrix_engine::operations::MessageSearchState::Idle;
            self.heights = Default::default();
            self.pending = Some(request);
            changed = true;
        } else if scope.is_none() && self.messages.take().is_some() {
            changed = true;
        }
        // The draft follows the room: stored when leaving, restored when coming back.
        if self.draft_scope != scope {
            self.target = None;
            self.parked.clear();
            self.menu = None;
            if let Some(old) = self.draft_scope.take() {
                self.drafts.insert(old, std::mem::take(&mut self.draft));
            }
            self.draft = scope.as_ref().and_then(|s| self.drafts.remove(s)).unwrap_or_default();
            self.draft_scope = scope;
        }
        changed |= self.drive_operations();
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

    fn drive_operations(&mut self) -> bool {
        let mut changed = false;
        // The new room shows up in the list with the next sync: select it as soon as it does.
        if let Some(scope) = self.pending_select.clone()
            && self.sidebar.select(&scope)
        {
            self.pending_select = None;
            changed = true;
        }
        if self.op_pending.is_none()
            && let Some(scope) = &self.draft_scope
            && self.members_requested.as_ref() != Some(scope)
        {
            self.op_pending = Some(OperationCommand::Members { scope: scope.clone() });
            self.members_requested = Some(scope.clone());
        }
        // The term in the box follows to the engine; closing the panel sends an empty one, which ends the search.
        let wanted = if self.search_open { self.search_term.trim().to_owned() } else { String::new() };
        if self.op_pending.is_none()
            && wanted != self.search_sent
            && let Some(scope) = &self.draft_scope
        {
            self.op_pending = Some(OperationCommand::SearchMessages { scope: scope.clone(), term: wanted.clone() });
            self.search_sent = wanted;
        }
        let wanted = if self.new_open { self.new_term.trim().to_owned() } else { String::new() };
        if self.op_pending.is_none() && wanted != self.new_sent {
            self.op_pending = Some(OperationCommand::SearchUsers { term: wanted.clone(), all: false });
            self.new_sent = wanted;
        }
        if let (Some(th), Some(scope)) = (&mut self.thread, &self.draft_scope)
            && th.next_read <= std::time::Instant::now()
            && let Ok(tag) = th.views.start(scope.clone())
        {
            th.next_read = std::time::Instant::now() + std::time::Duration::from_secs(5);
            self.op_queue.push(OperationCommand::Thread { tag, root: th.root.clone(), want: 50 });
        }
        if self.op_pending.is_none() && !self.op_queue.is_empty() {
            self.op_pending = Some(self.op_queue.remove(0));
        }
        if self.op_pending.is_none() {
            self.op_pending = self.ops.take_command();
        }
        if let (Some(command), Some((tx, _))) = (self.op_pending.take(), &mut self.op_route) {
            match tx.try_send(command) {
                Err(matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c)) => self.op_pending = Some(c),
                _ => {}
            }
        }
        let mut results = Vec::new();
        if let Some((_, rx)) = &mut self.op_route {
            while let Ok(r) = rx.try_recv() {
                results.push(r);
            }
        }
        for result in results {
            if let OperationResult::Thread { tag, root, result } = result {
                if let Some(th) = &mut self.thread
                    && th.root == root
                    && th.views.accepts(&tag)
                {
                    match result {
                        Ok(page) => {
                            th.page = Some(page);
                            th.failed = false;
                        }
                        Err(_) => th.failed = true,
                    }
                    changed = true;
                }
                continue;
            }
            if matches!(result, OperationResult::ThreadRead { .. } | OperationResult::ThreadSent { .. }) {
                if let Some(th) = &mut self.thread {
                    th.next_read = std::time::Instant::now();
                }
                continue;
            }
            match &result {
                OperationResult::UserSearch { state, .. } => {
                    self.users = state.clone();
                    changed = true;
                    continue;
                }
                OperationResult::RoomStarted { result } => {
                    match result {
                        Ok(scope) => {
                            self.pending_select = Some(scope.clone());
                            self.new_open = false;
                            self.new_failed = false;
                        }
                        Err(_) => self.new_failed = true,
                    }
                    changed = true;
                    continue;
                }
                OperationResult::Profile { user_id, result } => {
                    if let Some((open, slot)) = &mut self.profile
                        && open == user_id
                    {
                        *slot = result.clone().ok();
                        changed = true;
                    }
                    continue;
                }
                OperationResult::Ignored { user_id, ignore, result: Ok(_) } => {
                    if let Some((open, Some(p))) = &mut self.profile
                        && open == user_id
                    {
                        p.ignored = *ignore;
                        changed = true;
                    }
                    continue;
                }
                OperationResult::Ignored { .. } => continue,
                _ => {}
            }
            if let OperationResult::MessageSearch { scope, state } = result {
                if self.draft_scope.as_ref() == Some(&scope) {
                    self.search = state;
                    changed = true;
                }
                continue;
            }
            if let OperationResult::Members { scope, members, upload_limit } = result {
                // An answer for a room already left is dropped.
                if self.draft_scope.as_ref() == Some(&scope) {
                    self.upload_limit = upload_limit;
                    if let Ok(list) = members {
                        self.members = Some((scope, list));
                    }
                    changed = true;
                }
                continue;
            }
            let done = matches!(&result, OperationResult::Applied { result: Ok(_), .. });
            if let OperationResult::Applied { request, .. } = &result
                && done
            {
                self.history.activity(&request.scope);
            }
            changed |= self.ops.apply(result) | done;
        }
        changed
    }

    pub fn start_dm(&mut self, user_id: String) {
        self.new_failed = false;
        self.op_queue.push(OperationCommand::StartDm { user_id });
    }

    pub fn create_room(&mut self, name: String, topic: String, public: bool) {
        self.new_failed = false;
        let topic = Some(topic.trim().to_owned()).filter(|t| !t.is_empty());
        self.op_queue.push(OperationCommand::CreateRoom { name: name.trim().to_owned(), topic, public });
    }

    pub fn join(&mut self, target: String) {
        self.new_failed = false;
        self.op_queue.push(OperationCommand::JoinRoom { target: target.trim().to_owned(), via: Vec::new() });
        self.new_open = false;
    }

    pub fn answer_invite(&mut self, scope: matrix_domain::RoomScope, accept: bool) {
        self.op_queue.push(OperationCommand::AnswerInvite { scope, accept });
    }

    pub fn open_thread(&mut self, root: EventId) {
        self.thread = Some(ThreadState { root, views: Default::default(), page: None, failed: false, draft: String::new(), next_read: std::time::Instant::now() });
        self.info_open = false;
        self.search_open = false;
    }

    pub fn send_thread(&mut self, body: String) {
        let (Some(th), Some(scope)) = (&self.thread, self.draft_scope.clone()) else { return };
        self.op_queue.push(OperationCommand::ThreadSend { scope, root: th.root.clone(), reply_to: None, body });
    }

    pub fn open_profile(&mut self, user_id: String) {
        self.op_queue.push(OperationCommand::Profile { user_id: user_id.clone() });
        self.profile = Some((user_id, None));
    }

    pub fn set_ignored(&mut self, user_id: String, ignore: bool) {
        self.op_queue.push(OperationCommand::Ignore { user_id, ignore });
    }

    /// Opens the menu for a right-clicked message and asks the engine what the user may do with it.
    pub fn open_menu(&mut self, id: matrix_domain::ScopedTimelineId, pos: eframe::egui::Pos2) {
        if self.ops.select(id) {
            self.menu = Some(pos);
            self.menu_fresh = true;
            self.confirm_delete = false;
        }
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
        self.confirm_delete = false;
        self.ops.clear_selection();
    }

    /// Saves an attachment into the Download folder under a safe, unused name.
    pub fn save(&mut self, event: EventId, name: &str) {
        let Some(scope) = self.draft_scope.clone() else { return };
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        let dir = std::path::Path::new(&home).join("Downloads");
        // Only the last component of what the sender wrote, never a path.
        let base = std::path::Path::new(name).file_name().map_or("file".into(), |n| n.to_string_lossy().into_owned());
        let base = base.trim_start_matches('.');
        let base = if base.is_empty() { "file" } else { base };
        let mut path = dir.join(base);
        for n in 2.. {
            if !path.exists() {
                break;
            }
            path = dir.join(format!("{n}-{base}"));
        }
        let _ = std::fs::create_dir_all(&dir);
        self.notice = (!self.media.request_save(scope, event, path)).then(|| "Un salvataggio è già in corso".to_owned());
    }

    /// Takes a dropped file as the pending attachment (up to the server's upload limit, 50 MB until it is known).
    pub fn attach(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let data = std::fs::metadata(path).ok().filter(|m| 0 < m.len() && m.len() <= self.upload_limit.unwrap_or(50 << 20)).and_then(|_| std::fs::read(path).ok());
        match (name, data) {
            (Some(name), Some(data)) => {
                self.attachment = Some((name, data));
                self.notice = None;
            }
            _ => self.notice = Some("File non allegabile".to_owned()),
        }
    }

    /// Reply or edit: the composer works on the selected message until Enter or X.
    pub fn begin(&mut self, mode: crate::composer::Mode) {
        let Some(id) = self.ops.selection().cloned() else { return };
        self.parked = std::mem::take(&mut self.draft);
        if mode == crate::composer::Mode::Edit
            && let Some(body) = self.ops.details().map(|d| d.body.clone())
        {
            self.draft = body;
        }
        self.target = Some((id, mode));
        self.menu = None;
    }

    pub fn cancel_target(&mut self) {
        if self.target.take().is_some() {
            self.draft = std::mem::take(&mut self.parked);
            self.ops.clear_selection();
        }
    }

    pub fn react(&mut self, key: &str) {
        self.ops.toggle_reaction(key, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    pub fn delete(&mut self) {
        self.ops.submit(Operation::Redact, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    fn close_menu_keep_selection(&mut self) {
        self.menu = None;
        self.confirm_delete = false;
    }

    /// Enter in the composer: a plain message, or the reply/edit of the chosen one.
    pub fn submit_draft(&mut self, body: String) {
        match self.target.take() {
            Some((_, mode)) => {
                let op = if mode == crate::composer::Mode::Edit { Operation::Edit { body } } else { Operation::Reply { body } };
                self.ops.submit(op, matrix_engine::sending::fresh_transaction_id());
                self.draft = std::mem::take(&mut self.parked);
            }
            None => self.send(body),
        }
    }

    /// Sends the text to the open room; the live view picks up the echo.
    pub fn send(&mut self, body: String) {
        let (Some(scope), Some((tx, _))) = (self.draft_scope.clone(), &mut self.send) else { return };
        let request = matrix_engine::sending::SendRequest {
            scope: scope.clone(),
            transaction_id: matrix_engine::sending::fresh_transaction_id(),
            body,
            attachment: self.attachment.take().map(|(filename, data)| matrix_engine::sending::Attachment { filename, data, voice: None }),
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
