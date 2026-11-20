//! Sign-in and restore for the egui frontend: the same engine route and form state machine as the
//! GPUI app (`SignInForm`), so messages, vault restore and error categories stay identical.
use matrix_desktop::i18n;
use eframe::egui;
use crate::avatar::Avatar;
use matrix_desktop::{AppDirs, DemoHistory, FormStatus, HistorySession, LiveMedia, LiveOperations, SignInForm};
use matrix_domain::{EventId, TimelineId};
use matrix_engine::history::MediaKind;
use matrix_engine::media::{MediaCommand, MediaEvent};
use matrix_engine::operations::{Operation, OperationCommand, OperationResult};
use matrix_engine::{
    Receiver, RoomListEvent, RoomSyncStatus,
    room_sync::RoomListWorker,
    signin::{self, RecoveryState, SignInEvent, SignInMode, SignInUi},
};

/// The open thread: its page as last read, and the line being typed.
pub struct ThreadState {
    pub root: EventId,
    views: matrix_engine::ViewRequests,
    /// The root (when the room holds it) and the replies, as timeline rows; `None` until the first page.
    pub history: Option<matrix_desktop::DemoHistory>,
    at_start: bool,
    pub heights: crate::timeline::Heights,
    pub failed: bool,
    pub draft: String,
    /// File, picture or voice note going out with the next send.
    pub attachment: Option<(String, Vec<u8>)>,
    voice: Option<matrix_engine::sending::Voice>,
    pub reply_to: Option<(EventId, String)>,
    /// Right-clicked reply and where.
    pub menu: Option<(EventId, egui::Pos2)>,
    want: usize,
    /// Replies held when an older page was asked for; `Some` = that request is on its way.
    pub growing: Option<usize>,
    stalled: bool,
    pub tail_visible: bool,
    read_views: matrix_engine::ViewRequests,
    read_target: Option<EventId>,
    read_pending: bool,
    read_public: Option<bool>,
    read_at: Option<std::time::Instant>,
    /// Next re-read; the thread has no live feed of its own here.
    pub next_read: std::time::Instant,
}

/// Older replies are asked for when the reader is near the top: one request at a time, never once the thread is whole
/// (`total` replies when the root says so, else the timeline's own start) or after a request that brought nothing.
fn wants_older(first_visible: usize, held: usize, total: Option<usize>, at_start: bool, stalled: bool, loading: bool) -> bool {
    first_visible <= 3 && !loading && !stalled && !total.map_or(at_start, |n| held >= n)
}

#[test]
fn thread_pages_back_near_the_top_once_and_stops_at_the_root() {
    assert!(wants_older(0, 50, Some(120), true, false, false), "the count wins over an early `at_start`");
    assert!(wants_older(3, 50, None, false, false, false));
    assert!(!wants_older(4, 50, Some(120), false, false, false), "not near the top");
    assert!(!wants_older(0, 50, Some(120), false, false, true), "one load at a time");
    assert!(!wants_older(0, 50, Some(120), false, true, false), "a page that brought nothing ends it");
    assert!(!wants_older(0, 120, Some(120), false, false, false), "whole thread");
    assert!(!wants_older(0, 3, None, true, false, false), "start reached");
}

impl ThreadState {
    /// Rows pinned on top of the list: the root, when it is there.
    pub fn head(&self) -> usize {
        self.history.as_ref().and_then(|h| h.events().first()).is_some_and(|e| e.id.id == TimelineId::Server(self.root.clone())) as usize
    }

    /// The list scrolled to `first_visible`: asks for the older replies when it is time.
    pub fn scrolled(&mut self, first_visible: usize) {
        let Some(h) = &self.history else { return };
        let held = h.events().len() - self.head();
        let total = h.events().first().filter(|_| self.head() == 1).and_then(|e| e.thread.as_ref()).map(|t| t.num_replies as usize);
        if wants_older(first_visible, held, total, self.at_start, self.stalled, self.growing.is_some()) {
            self.growing = Some(held);
            self.want = self.want.saturating_add(50);
            self.next_read = std::time::Instant::now();
        }
    }
}

/// «Modifica stanza» form: what is typed, the picture dropped on it and whether the last try failed.
pub struct RoomEdit {
    pub name: String,
    pub topic: String,
    pub avatar: Option<std::path::PathBuf>,
    pub failed: bool,
    pub busy: bool,
    /// The room or space being edited.
    pub scope: matrix_domain::RoomScope,
    original: (String, String),
}

pub enum RoomAction {
    Mark(bool),
    Tag(&'static str),
    Mode(matrix_domain::notifications::RoomNotifyMode),
    Leave,
    /// Ask the engine which spaces can take or already hold the room (the menu shows the entries when the answer is in).
    Spaces,
    AddTo(matrix_domain::RoomScope),
    RemoveFrom(matrix_domain::RoomScope),
}

pub struct Session {
    pub form: SignInForm,
    ui: SignInUi,
    /// Kept alive for the whole session: dropping it stops the backend.
    _worker: RoomListWorker,
    rooms: Receiver<RoomListEvent>,
    pub account: Option<matrix_domain::AccountId>,
    /// Catalog id of this account (worker ids repeat between profile folders); set by the window.
    pub catalog_id: Option<matrix_domain::AccountId>,
    /// Display names of the other connected accounts, set by the window (a title names the recipient only with 2+ accounts).
    pub peers: Vec<String>,
    /// A notification click waiting for its room: when it came and how many rooms the user had opened by then.
    pending_click: Option<(std::time::Instant, u64)>,
    pub sidebar: matrix_desktop::Sidebar,
    pub confirm_leave: Option<matrix_domain::RoomScope>,
    pub room_error: Option<String>,
    pub stickers: crate::stickers::Panel,
    /// Spaces for the room whose menu was opened last; `None` inside until the engine answers.
    pub space_choices: Option<(matrix_domain::RoomScope, Option<matrix_engine::operations::SpaceChoices>)>,
    /// «Aggiungi a uno spazio» window: the room and the space picked so far.
    pub space_pick: Option<(matrix_domain::RoomScope, Option<matrix_domain::RoomScope>)>,
    /// Window of the selected space («Info spazio» / «Membri»).
    pub space_view: Option<crate::space::SpaceView>,
    /// What «Esci dallo spazio» is judged on, for the space whose «⋮» was opened last; `None` inside until the engine answers.
    pub space_facts: Option<(matrix_domain::RoomScope, Option<matrix_engine::operations::SpaceChildren>)>,
    /// The engine's answer to the last `place_space`, until the rail takes it.
    pub space_placed: Option<Result<(), matrix_engine::operations::OperationError>>,
    /// «Esci dallo spazio» window: the space and the «Dimentica» switch.
    pub space_exit: Option<(matrix_domain::RoomScope, bool)>,
    /// «Elimina spazio» window: the space and the name typed so far.
    pub space_delete: Option<(matrix_domain::RoomScope, String)>,
    /// The deletion runs (one at a time).
    pub space_deleting: bool,
    /// What the last «Elimina spazio» did: the space name and the report; shown until closed.
    pub space_report: Option<(String, Result<matrix_engine::operations::SpaceDeleted, matrix_engine::operations::OperationError>)>,
    /// Rights and banned users of the member list on screen.
    pub admin: Option<crate::members::Admin>,
    history: HistorySession,
    pending: Option<matrix_engine::history::HistoryRequest>,
    route: Option<(matrix_engine::Sender<matrix_engine::history::HistoryRequest>, Receiver<matrix_engine::history::HistoryResult>)>,
    pub messages: Option<DemoHistory>,
    /// «Messaggi di stato» per room, saved in the profile folder.
    pub status_choices: matrix_desktop::RoomStatusChoices,
    send: Option<(matrix_engine::Sender<matrix_engine::sending::SendRequest>, Receiver<matrix_engine::sending::SendEvent>)>,
    drafts: std::collections::HashMap<matrix_domain::RoomScope, String>,
    pub draft: String,
    pub ops: LiveOperations,
    /// Joined members of the open room and the server's upload limit.
    pub members: Option<(matrix_domain::RoomScope, Vec<matrix_engine::operations::Member>)>,
    members_requested: Option<matrix_domain::RoomScope>,
    /// A role was changed here: members and rights are read again when the room's next event arrives.
    roles_stale: Option<matrix_domain::RoomScope>,
    upload_limit: Option<u64>,
    pub info_open: bool,
    /// The voice note being played; dropping the output stops the sound.
    audio: Option<(EventId, matrix_desktop::audio_player::AudioOutput)>,
    /// A voice note being recorded (the microphone is open); `voice` is the note waiting to be sent with the next send.
    pub recorder: Option<matrix_desktop::voice_recorder::Recorder>,
    /// The recording was started from the thread's composer: its bar shows there and the note goes into the thread.
    record_thread: bool,
    voice: Option<matrix_engine::sending::Voice>,
    /// Set by the app each frame: the window has focus, the newest message is in view, the «Conferme di lettura» setting.
    pub reading: (bool, bool, bool),
    read_dirty: bool,
    /// People picked from the «@» list in the text being written.
    pub mentions: Vec<matrix_engine::sending::Mention>,
    pub aliases: Option<matrix_engine::alias::RoomAliases>,
    aliases_requested: Option<matrix_domain::RoomScope>,
    pub alias_error: Option<&'static str>,
    pub alias_input: String,
    pub focus_search: bool,
    pub quick_open: bool,
    pub edit: Option<RoomEdit>,
    notifications: matrix_domain::notifications::NotificationPolicy,
    _notify_worker: Option<matrix_desktop::notifications::NotificationWorker>,
    notify: Option<matrix_desktop::notifications::NotificationHandle>,
    /// Name the alerts carry as title (the account's own name once known).
    pub account_name: String,
    pub poll_open: bool,
    pub thread: Option<ThreadState>,
    pub verification: Option<matrix_desktop::DeviceVerification>,
    ver_route: Option<(matrix_engine::Sender<matrix_engine::verification::VerificationCommand>, Receiver<matrix_engine::verification::VerificationEvent>)>,
    /// Whether the server lets this account change its (name, picture); None until it has answered.
    pub profile_caps: Option<(bool, bool)>,
    /// A profile save is on its way; then its answer (the error is one line for the card).
    pub profile_busy: bool,
    pub profile_saved: Option<Result<(), &'static str>>,
    /// The file the running save writes; on iOS it goes to the share sheet when the save is done.
    share_after: std::path::PathBuf,
    pub new_open: bool,
    pub new_term: String,
    new_sent: String,
    pub users: matrix_engine::operations::SearchState,
    /// A new user search went out and its first answer (Searching/Idle) is not in yet: older answers are stale.
    users_gate: bool,
    /// Public rooms for the quick search, and the term last sent for them (None: nothing asked).
    pub public_rooms: matrix_engine::search::SearchState<matrix_engine::operations::DirectoryRoom>,
    rooms_sent: Option<matrix_desktop::quick_search::DirectoryRequest>,
    /// «Esplora stanze pubbliche» (own term/server/kind; shares `public_rooms` with the quick search, never both open).
    pub explore: Option<crate::explore::Explore>,
    /// Highlighted row of the quick search, counted over all groups.
    pub quick_sel: usize,
    pub new_failed: bool,
    /// Joins asked from the quick search or «Unisciti», by the command's target; a successful one is removed.
    pub joins: std::collections::HashMap<String, crate::join::JoinState>,
    /// «Crea spazio» was sent and its answer is not in yet.
    pub new_busy: bool,
    pub pending_select: Option<matrix_domain::RoomScope>,
    /// The space just created, shown in the rail as soon as the sync lists it.
    pending_space: Option<matrix_domain::RoomScope>,
    /// `last_room` in the profile folder: the room on screen, written at every change and reopened at the next start.
    last_file: std::path::PathBuf,
    last_room: Option<String>,
    /// The saved room is still to reopen; its start state is `restore_from`.
    restoring: bool,
    /// Start-up restore: the user's room openings (`Sidebar::picks`) and when the sync went live (the 20 s run from there: the saved room can come after the first pages of the list).
    restore_from: Option<(u64, Option<std::time::Instant>)>,
    /// Commands from the UI waiting for the single send slot.
    op_queue: Vec<OperationCommand>,
    /// Member whose card is open and what the engine answered about them.
    pub profile: Option<(String, Option<matrix_engine::operations::UserProfile>)>,
    /// What the user may do to the member of the open card, and the confirmation step of the action asked.
    pub rights: Option<matrix_engine::operations::MemberRights>,
    pub moderation_failed: bool,
    /// An action of the card went out and its answer is not in yet.
    pub moderating: bool,
    pub search_open: bool,
    pub search_term: String,
    search_sent: String,
    pub search: matrix_engine::operations::MessageSearchState,
    pub search_jump: Option<(EventId, bool)>,
    /// The message a jump landed on and when: drawn lit for a moment.
    pub flash: Option<(EventId, std::time::Instant)>,
    media: LiveMedia,
    pub viewer: Option<crate::viewer::Viewer>,
    media_route: Option<(matrix_engine::Sender<MediaCommand>, Receiver<MediaEvent>)>,
    /// Thumbnails of the open room's pictures and videos, by event.
    pub pictures: std::collections::HashMap<EventId, egui::TextureHandle>,
    /// Sender photos by mxc; `None` = asked and none came (initials stay).
    pub avatars: crate::avatar::Avatars,
    avatar_failed: std::collections::HashMap<String, std::time::Instant>,
    avatar_slot: matrix_engine::avatar::AvatarSlot,
    avatar_chan: (std::sync::mpsc::Sender<matrix_engine::avatar::AvatarPicture>, std::sync::mpsc::Receiver<matrix_engine::avatar::AvatarPicture>),
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
    pub confirm_end_poll: bool,
    /// The menu opened this frame: the click that opened it must not close it.
    pub menu_fresh: bool,
    draft_scope: Option<matrix_domain::RoomScope>,
    pub heights: crate::timeline::Heights,
    /// Reading position per room, saved on room change and on quit.
    scroll: crate::timeline::ScrollMemory,
    scroll_file: std::path::PathBuf,
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
    pub fn start(dirs: &AppDirs, root: std::path::PathBuf, trust: Option<Vec<u8>>) -> Result<Session, String> {
        matrix_desktop::create_private_dir(&root).map_err(|e| e.to_string())?;
        let status_dir = root.clone();
        let last_file = root.join("last_room");
        let scroll_file = root.join("scroll_pos.json");
        let last_room = std::fs::read_to_string(&last_file).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let vault = signin::platform_source();
        let with_vault = vault.is_some();
        let (ui, pending) = match vault {
            Some(source) => signin::interactive_with_source(root, trust, source),
            None => signin::interactive(root, trust),
        };
        let (mut worker, rooms) = RoomListWorker::spawn(move || async move {
            pending.run().await.map_err(|_| matrix_engine::auth::AuthError::Storage)
        })
        .map_err(|e| e.to_string())?;
        let route = worker.take_history_route();
        let send = worker.take_send_route();
        let op_route = worker.take_operation_route();
        let notify_worker = matrix_desktop::notifications::NotificationWorker::spawn().ok();
        let ver_route = worker.take_verification_route();
        let media_route = worker.take_media_route();
        let avatar_slot = worker.avatar_slot();
        let mut form = SignInForm::default();
        form.vault = with_vault;
        Ok(Session {
            form,
            ui,
            _worker: worker,
            rooms,
            account: None,
            catalog_id: None,
            peers: Vec::new(),
            pending_click: None,
            history: HistorySession::new(),
            pending: None,
            route,
            send,
            drafts: Default::default(),
            status_choices: matrix_desktop::RoomStatusChoices::load(&status_dir),
            draft: String::new(),
            ops: LiveOperations::default(),
            members: None,
            members_requested: None,
            roles_stale: None,
            upload_limit: None,
            info_open: false,
            audio: None,
            recorder: None,
            record_thread: false,
            voice: None,
            reading: (false, false, true),
            read_dirty: true,
            mentions: Vec::new(),
            aliases: None,
            aliases_requested: None,
            alias_error: None,
            alias_input: String::new(),
            focus_search: false,
            quick_open: false,
            edit: None,
            notifications: matrix_domain::notifications::NotificationPolicy::new(matrix_domain::AccountId::new(0)),
            notify: notify_worker.as_ref().map(|w| w.handle()),
            _notify_worker: notify_worker,
            account_name: String::new(),
            poll_open: false,
            thread: None,
            verification: None,
            ver_route,
            profile_caps: None,
            profile_busy: false,
            profile_saved: None,
            share_after: std::path::PathBuf::new(),
            new_open: false,
            new_term: String::new(),
            new_sent: String::new(),
            users: matrix_engine::operations::SearchState::Idle,
            users_gate: false,
            public_rooms: matrix_engine::search::SearchState::Idle,
            rooms_sent: None,
            explore: None,
            quick_sel: 0,
            new_failed: false,
            joins: Default::default(),
            new_busy: false,
            pending_space: None,
            pending_select: None,
            last_file,
            restoring: last_room.is_some(),
            last_room,
            restore_from: None,
            op_queue: Vec::new(),
            space_placed: None,
            profile: None,
            rights: None,
            moderation_failed: false,
            moderating: false,
            search_open: false,
            search_term: String::new(),
            search_sent: String::new(),
            search: matrix_engine::operations::MessageSearchState::Idle,
            search_jump: None,
            flash: None,
            media: LiveMedia::default(),
            viewer: None,
            media_route,
            pictures: Default::default(),
            avatars: Default::default(),
            avatar_failed: Default::default(),
            avatar_slot,
            avatar_chan: std::sync::mpsc::channel(),
            op_route,
            op_pending: None,
            target: None,
            parked: String::new(),
            menu: None,
            attachment: None,
            notice: None,
            confirm_delete: false,
            confirm_end_poll: false,
            menu_fresh: false,
            draft_scope: None,
            messages: None,
            heights: Default::default(),
            scroll: crate::timeline::ScrollMemory::from_json(&std::fs::read_to_string(&scroll_file).unwrap_or_default()),
            scroll_file,
            first_visible: usize::MAX,
            sidebar: matrix_desktop::Sidebar::connected(matrix_domain::AccountId::new(0)),
            query: String::new(),
            confirm_leave: None,
            room_error: None,
            stickers: Default::default(),
            space_choices: None,
            space_pick: None,
            space_view: None,
            space_facts: None,
            space_exit: None,
            space_delete: None,
            space_deleting: false,
            space_report: None,
            admin: None,
            sync: None,
            homeserver: String::new(),
            user: String::new(),
            password: String::new(),
            phrase: String::new(),
        })
    }

    /// Drains what the backend sent since the last frame. True when something changed.
    pub fn poll(&mut self, ctx: &egui::Context, notifications: bool, preview: bool) -> bool {
        let mut changed = false;
        while let Ok(event) = self.ui.setup.try_recv() {
            self.form.apply_setup(event);
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
                    self.notifications = matrix_domain::notifications::NotificationPolicy::new(account);
                    // Rooms are scoped by account: the list must be built for the account that signed in.
                    self.sidebar = matrix_desktop::Sidebar::connected(account);
                }
                _ => {}
            }
        }
        while let Ok(event) = self.rooms.try_recv() {
            changed = true;
            match event {
                RoomListEvent::Status(s) => {
                    if s == RoomSyncStatus::AuthenticationFailed
                        && let Some(h) = &self.notify
                    {
                        h.cancel_pending();
                    }
                    self.sync = Some(s);
                }
                RoomListEvent::Diff(diff) => {
                    self.sidebar.apply_diff(diff);
                }
                // A new event in a room (an upload that finished, a reply from someone else): read the open room again.
                RoomListEvent::TimelineActivity(scope) => {
                    if let Some(th) = &mut self.thread {
                        th.next_read = std::time::Instant::now();
                    }
                    self.read_dirty |= self.sidebar.selected().is_some_and(|r| r.scope == scope);
                    if self.roles_stale.as_ref() == Some(&scope) {
                        self.roles_stale = None;
                        self.members_requested = None;
                        if let Some((user_id, _)) = &self.profile {
                            self.op_queue.push(OperationCommand::MemberRights { scope: scope.clone(), user_id: user_id.clone() });
                        }
                    }
                    self.history.activity(&scope)
                }
                RoomListEvent::Order(order) => self.sidebar.set_order(order),
                RoomListEvent::Alert(scope, message) => {
                    self.notifications.set_enabled(notifications);
                    self.notifications.set_message_preview(preview);
                    let reading = matrix_desktop::notifications::reading(self.reading.0, self.sidebar.selected().map(|r| &r.scope), &scope);
                    if !reading
                        && let (Some(alert), Some(handle), Some(account)) = (self.notifications.alert(&scope, message), &self.notify, self.account)
                    {
                        let name = if self.account_name.is_empty() { "Gnobbi".to_owned() } else { crate::list::clean_single_line(&self.account_name) };
                        let user = self.verification.as_ref().and_then(|v| v.profile()).map(|(id, _)| id.clone()).unwrap_or_default();
                        let room_row = self.sidebar.rooms().iter().find(|r| r.scope == scope);
                        let room = room_row.map(|r| r.name.clone());
                        let peers: Vec<&str> = self.peers.iter().map(String::as_str).collect();
                        let alert = alert.from_account(self.catalog_id.unwrap_or(account), name.clone()).titled(&name, &user, room.as_deref(), &peers);
                        // The picture is fetched by the existing avatar source (cache first); the notification worker waits for it briefly.
                        let picture = alert.avatar_mxc(room_row.and_then(|r| r.info.avatar.as_deref())).and_then(|mxc| {
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.avatar_slot.try_lock().ok()?.as_ref()?.request(mxc.to_owned(), tx);
                            Some(rx)
                        });
                        handle.submit_with_avatar(alert, picture);
                    }
                }
                // Alerts and timeline activity belong to later slices.
                _ => {}
            }
        }
        if let (Some(v), Some((tx, rx))) = (&mut self.verification, &mut self.ver_route) {
            while let Ok(event) = rx.try_recv() {
                use matrix_engine::verification::VerificationEvent as E;
                match &event {
                    E::ProfileCapabilities { name, avatar, .. } => self.profile_caps = Some((*name, *avatar)),
                    E::ProfileSaved { result, message, .. } => {
                        self.profile_busy = false;
                        self.profile_saved = Some(result.as_ref().map(|_| ()).map_err(|_| message.map_or(i18n::t("Failed"), i18n::t)));
                    }
                    _ => {}
                }
                v.apply(event);
                changed = true;
            }
            v.flush(tx);
        }
        // A click on an alert opens its room (once it is in the list).
        let click = self.notify.as_ref().and_then(|h| h.take_click()).map(|(_, scope)| scope);
        if let Some(scope) = click.or_else(|| self.catalog_id.and_then(|id| matrix_desktop::notifications::take_early_click(Some(id)))) {
            self.pending_select = Some(scope);
            self.pending_click = Some((std::time::Instant::now(), self.sidebar.picks()));
            // A click that launched the app must not lose to the room restored from the last run.
            self.restoring = false;
            // Hidden to the tray or minimized: a bare Focus would leave the window out of sight.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        changed |= self.drive_history();
        changed |= self.drive_media(ctx);
        changed |= self.drive_stickers();
        changed |= self.drive_avatars(ctx);
        changed
    }

    /// While the Sticker tab is shown: asks for the packs of the open room (again after 30 s) and queues their pictures as avatars.
    fn drive_stickers(&mut self) -> bool {
        let Some(scope) = self.draft_scope.clone() else { return false };
        if self.stickers.scope.as_ref() != Some(&scope) {
            self.stickers = crate::stickers::Panel { scope: Some(scope.clone()), ..Default::default() };
        }
        let open = std::mem::take(&mut self.stickers.open);
        if open && self.stickers.fetched.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(30)) {
            self.stickers.fetched = Some(std::time::Instant::now());
            self.op_queue.push(OperationCommand::StickerPacks { scope });
        }
        false
    }

    pub fn send_sticker(&mut self, sticker: matrix_engine::stickers::Sticker) {
        if let Some(scope) = self.draft_scope.clone() {
            self.op_queue.push(OperationCommand::SendSticker { scope, sticker });
        }
    }

    /// Not asked yet, or a transient failure older than 30 s.
    fn avatar_wanted(&self, mxc: &str) -> bool {
        match self.avatars.get(mxc) {
            None => true,
            Some(Avatar::Missing) => self.avatar_failed.get(mxc).is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(30)),
            Some(_) => false,
        }
    }

    /// Asks for the senders' photos of the open room and turns the answers into textures.
    /// A transient failure is asked again after 30 s (like the GPUI timeline); initials stay meanwhile.
    fn drive_avatars(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        while let Ok(p) = self.avatar_chan.1.try_recv() {
            let tex = p.rgba.map(|(w, h, rgba)| ctx.load_texture(&p.mxc, egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba), Default::default()));
            changed |= tex.is_some();
            if tex.is_none() && p.retry {
                // Missing (initials) until the retry after 30 s.
                self.avatar_failed.insert(p.mxc.clone(), std::time::Instant::now());
            } else {
                self.avatar_failed.remove(&p.mxc);
            }
            self.avatars.insert(p.mxc, tex.map_or(Avatar::Missing, Avatar::Ready));
        }
        let Ok(slot) = self.avatar_slot.try_lock() else { return changed };
        let Some(source) = slot.as_ref() else { return changed };
        // Room, DM and space pictures of the list and the rail: a few per tick, so a long list does not burst the media worker.
        let mut budget = 6;
        let mut wanted: Vec<String> = self.sidebar.rooms().iter().filter_map(|r| r.info.avatar.clone()).collect();
        wanted.extend(self.sidebar.navigation_entries().into_iter().filter_map(|e| e.avatar));
        if let Some(crate::space::SpaceView { children: Some(Ok(c)), .. }) = &self.space_view { wanted.extend(c.children.iter().filter_map(|c| c.avatar_url.clone())); }
        wanted.extend(self.sidebar.selected().into_iter().flat_map(|r| &r.info.public_read_receipts).flat_map(|r| &r.users).filter_map(|u| u.avatar.clone()));
        wanted.extend(self.verification.as_ref().and_then(|v| v.profile_avatar()).map(str::to_owned));
        wanted.extend(crate::stickers::urls(&self.stickers).map(str::to_owned));
        if let (Some(_), matrix_engine::search::SearchState::Results { hits, .. }) = (&self.explore, &self.public_rooms) { wanted.extend(hits.iter().filter_map(|h| h.avatar_url.clone())); }
        for mxc in wanted {
            if budget > 0 && self.avatar_wanted(&mxc) {
                budget -= 1;
                self.avatars.insert(mxc.clone(), Avatar::Loading);
                source.request(mxc, self.avatar_chan.0.clone());
            }
        }
        let Some(messages) = &self.messages else { return changed };
        let thread = self.thread.iter().filter_map(|th| th.history.as_ref());
        for mxc in messages.sender_avatar_mxcs().chain(thread.flat_map(|h| h.sender_avatar_mxcs())) {
            if self.avatar_wanted(mxc) {
                self.avatars.insert(mxc.to_owned(), Avatar::Loading);
                source.request(mxc.to_owned(), self.avatar_chan.0.clone());
            }
        }
        changed
    }

    /// One thumbnail at a time, newest first; a picture that failed stays a text row.
    fn drive_media(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if let (Some(messages), Some(scope)) = (&self.messages, &self.draft_scope) {
            let mut videos = Vec::new();
            let mut wanted = Vec::new();
            let thread = self.thread.iter().filter_map(|th| th.history.as_ref()).flat_map(|h| h.events());
            for e in messages.events().iter().chain(thread).rev() {
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
            self.notice = Some(if ok { i18n::t("Saved").to_owned() } else { i18n::t("Saving failed").to_owned() });
            // iOS: the file sits in a private folder until the share sheet takes it (no-op on desktop).
            if ok && cfg!(target_os = "ios") { crate::pick::share(&std::mem::take(&mut self.share_after)); }
            changed = true;
        }
        if let Some((id, result)) = self.media.take_audio() {
            match result {
                Ok(opened) => {
                    let player = matrix_desktop::audio_player::Player::new(opened.samples, opened.sample_rate);
                    match matrix_desktop::audio_player::AudioOutput::start(player) {
                        Ok(out) => self.audio = Some((id, out)),
                        Err(e) => self.notice = Some(e.to_owned()),
                    }
                }
                Err(_) => self.notice = Some(i18n::t("Voice note cannot be played").to_owned()),
            }
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
        if let Some((id, result)) = self.media.take_original() {
            if let Some(viewer) = &mut self.viewer && viewer.id == id { viewer.set_original(ctx, result); }
            changed = true;
        }
        if let Some((id, result)) = self.media.take_video() {
            if let Some(viewer) = &mut self.viewer && viewer.id == id {
                match result {
                    Ok(file) => {
                        let path = file.path().to_owned();
                        self.media.hold_playing(file);
                        viewer.open_player(path);
                    }
                    Err(_) => { viewer.failed = true; viewer.opening = false; },
                }
            }
            changed = true;
        }
        changed
    }

    /// Writes the shown room's reading position (room change, quit); a room not laid out yet keeps its old entry.
    pub fn remember_scroll(&mut self) {
        let Some(room) = self.messages.as_ref().and_then(|m| m.events().first()).map(|e| e.id.scope.room_id.as_str().to_owned()) else { return };
        if let Some(pos) = self.heights.position()
            && self.scroll.set(&room, pos)
        {
            let _ = std::fs::write(&self.scroll_file, self.scroll.to_json());
        }
    }

    /// Opens the selected room's history, applies the pages that came back and asks for older ones near the top.
    fn drive_history(&mut self) -> bool {
        let mut changed = false;
        let scope = self.sidebar.selected().map(|r| r.scope.clone());
        if let Some(request) = self.history.select(scope.clone()) {
            self.remember_scroll();
            self.read_dirty = true;
            self.messages = Some(DemoHistory::new(request.tag.scope().clone(), 0));
            self.media.clear();
            self.viewer = None;
            self.audio = None;
            self.pictures.clear();
            self.members = None;
            self.thread = None;
            self.upload_limit = None;
            self.search_sent.clear();
            self.search_jump = None;
            self.search = matrix_engine::operations::MessageSearchState::Idle;
            self.heights = Default::default();
            self.heights.restore = scope.as_ref().and_then(|s| self.scroll.get(s.room_id.as_str()));
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
            self.heights.restore_wait.0 = self.heights.restore_wait.0.saturating_add(1);
        }
        if let (Some((id, _)), Some(messages)) = (&self.heights.restore, &self.messages) {
            let found = messages.events().iter().any(|e| e.id.id == TimelineId::Server(id.clone()));
            let (pages, since) = &mut self.heights.restore_wait;
            let waited = since.get_or_insert_with(std::time::Instant::now).elapsed();
            if !found && restore_gave_up(self.history.status(), *pages, waited) {
                self.heights.restore = None;
            }
        }
        if self.pending.is_none() {
            self.pending = self.history.refresh();
        }
        if self.search_jump.is_some() && matches!(self.history.status(), matrix_desktop::HistoryStatus::Start | matrix_desktop::HistoryStatus::Failed) {
            let found = self.search_jump.as_ref().is_some_and(|(id, _)| self.messages.as_ref().is_some_and(|m| m.events().iter().any(|e| e.id.id == TimelineId::Server(id.clone()))));
            if !found {
                self.search_jump = None;
                self.notice = Some(i18n::t("Message not available").to_owned());
            }
        }
        if self.pending.is_none() && self.messages.is_some() {
            self.pending = self.history.load_older_near_top(if self.search_jump.is_some() { 0 } else { self.first_visible });
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
        // A click's room that never shows up, or a room the user opened meanwhile, ends the wait.
        if let Some((since, picks)) = self.pending_click
            && (since.elapsed() > std::time::Duration::from_secs(30) || self.sidebar.picks() != picks)
        {
            self.pending_click = None;
            self.pending_select = None;
        }
        if let Some(scope) = self.pending_select.clone()
            && self.sidebar.select(&scope)
        {
            self.pending_select = None;
            self.pending_click = None;
            self.profile = None;
            self.info_open = false;
            changed = true;
        }
        if let Some(scope) = self.pending_space.clone()
            && self.sidebar.set_space(Some(scope))
        {
            self.pending_space = None;
            changed = true;
        }
        let current = self.sidebar.selected().map(|r| r.scope.clone());
        if self.restoring && let Some(want) = &self.last_room {
            let now = current.as_ref().map(|c| c.room_id.as_str());
            if self.sidebar.rooms().is_empty() {
                self.restore_from = None;
            }
            let (picks, live) = self.restore_from.get_or_insert((self.sidebar.picks(), None));
            if live.is_none() && self.sync == Some(RoomSyncStatus::Ready) {
                *live = Some(std::time::Instant::now());
            }
            let (picks, waited) = (*picks, live.map(|t| t.elapsed()).unwrap_or_default());
            if self.sidebar.rooms().is_empty() {
                // list not loaded yet: wait
            } else if restore_over(want, now, self.sidebar.picks() != picks, waited) {
                self.restoring = false;
            } else if let Some(scope) = self.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == want).map(|r| r.scope.clone())
                && self.sidebar.select_outside_view(&scope)
            {
                self.restoring = false;
                changed = true;
            }
        } else if let Some(scope) = current
            && self.last_room.as_deref() != Some(scope.room_id.as_str())
        {
            self.last_room = Some(scope.room_id.as_str().to_owned());
            let _ = std::fs::write(&self.last_file, scope.room_id.as_str());
        }
        // Opening or receiving in the room on screen: the read marker goes out once the newest message is in view (main only, so threads stay unread).
        let (focused, at_bottom, public) = self.reading;
        if self.read_dirty
            && focused
            && at_bottom
            && self.op_pending.is_none()
            && self.account.is_some()
            && let Some(scope) = self.sidebar.selected().map(|r| r.scope.clone())
            && self.messages.is_some()
        {
            self.read_dirty = false;
            self.op_pending = Some(OperationCommand::MarkRoom { scope, read: true, public, main_only: true });
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
            self.op_queue.retain(|op| !matches!(op, OperationCommand::MoreMessages));
            self.op_pending = Some(OperationCommand::SearchMessages { scope: scope.clone(), term: wanted.clone() });
            self.search_sent = wanted;
        }
        let wanted = if self.new_open || self.quick_open || self.admin.as_ref().is_some_and(|a| a.inviting) { self.new_term.trim().to_owned() } else { String::new() };
        if self.op_pending.is_none() && wanted != self.new_sent {
            self.op_pending = Some(OperationCommand::SearchUsers { term: wanted.clone(), all: false });
            self.new_sent = wanted;
            self.users_gate = true;
        }
        let wanted = match &self.explore {
            Some(e) => e.request(),
            None if self.quick_open => matrix_desktop::quick_search::room_request(None, &self.new_term, None),
            None => None,
        };
        if self.op_pending.is_none() && wanted != self.rooms_sent {
            self.op_pending = Some(match &wanted {
                Some((term, server, kind)) => OperationCommand::SearchRooms { term: term.clone(), server: server.clone(), kind: *kind },
                None => OperationCommand::SearchRooms { term: String::new(), server: None, kind: None },
            });
            // The old server's or term's rows must not show under the new request.
            if self.explore.is_some() { self.public_rooms = matrix_engine::search::SearchState::Idle; }
            self.rooms_sent = wanted;
        }
        if let (Some(th), Some(scope)) = (&mut self.thread, &self.draft_scope)
            && th.next_read <= std::time::Instant::now()
            && let Ok(tag) = th.views.start(scope.clone())
        {
            th.next_read = std::time::Instant::now() + std::time::Duration::from_secs(5);
            self.op_queue.push(OperationCommand::Thread { tag, root: th.root.clone(), want: th.want });
        }
        if let (Some(th), Some(scope)) = (&mut self.thread, &self.draft_scope) {
            if !focused || !th.tail_visible || th.read_public.is_some_and(|previous| previous != public) {
                th.read_views.cancel();
                th.read_pending = false;
                th.read_at = None;
                th.read_public = None;
            } else if let Some(target) = th.history.as_ref().and_then(|h| h.events().last()).and_then(|e| match &e.id.id { TimelineId::Server(id) => Some(id.clone()), _ => None })
                && th.read_target.as_ref() != Some(&target)
                && !th.read_pending
                && th.read_at.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1))
                && let Ok(tag) = th.read_views.start(scope.clone())
            {
                th.read_at = Some(std::time::Instant::now());
                th.read_pending = true;
                th.read_public = Some(public);
                self.op_queue.push(OperationCommand::ThreadRead { tag, root: th.root.clone(), target, public });
            }
        }
        // The addresses of the room shown in the Info panel are read once per room.
        if self.info_open
            && let Some(scope) = self.draft_scope.clone()
            && self.aliases_requested.as_ref() != Some(&scope)
        {
            self.aliases_requested = Some(scope.clone());
            self.aliases = None;
            self.alias_error = None;
            self.op_queue.push(OperationCommand::Aliases { scope, action: matrix_engine::alias::AliasAction::Load });
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
            match &result {
                OperationResult::TagsEdited { result } => { self.sidebar.tag_done(result.clone()); self.room_error = self.sidebar.tag_error().map(str::to_owned); changed = true; continue; }
                OperationResult::NotificationModeSet { result, .. } => { self.sidebar.mode_done(result.clone()); self.room_error = self.sidebar.mode_error().map(str::to_owned); changed = true; continue; }
                OperationResult::RoomLeft { scope, result } => { self.sidebar.room_left(scope, result.clone()); self.room_error = self.sidebar.leave_error(scope).map(str::to_owned); changed = true; continue; }
                OperationResult::SpaceLeft { scope, result, .. } => { self.sidebar.space_left(scope, result.clone().map(drop)); self.room_error = self.sidebar.leave_error(scope).map(str::to_owned); changed = true; continue; }
                OperationResult::SpaceDeleted { scope, result } => {
                    let name = self.sidebar.spaces().find(|r| &r.scope == scope).map_or_else(String::new, |r| r.name.clone());
                    self.space_deleting = false;
                    // Left for good: the space drops out of the rail; otherwise it stays and the report says why.
                    if result.as_ref().is_ok_and(|d| matches!(d.left, Some(Ok(_)))) { self.sidebar.space_left(scope, Ok(())); }
                    self.space_report = Some((name, result.clone()));
                    self.space_facts_for(scope.clone());
                    changed = true;
                    continue;
                }
                OperationResult::SpaceChoices { scope, result } => {
                    if let (Some((asked, slot)), Ok(choices)) = (&mut self.space_choices, result) && asked == scope { *slot = Some(choices.clone()); }
                    changed = true;
                    continue;
                }
                OperationResult::SpaceChildren { scope, result } => {
                    if let Some(v) = &mut self.space_view && &v.scope == scope { v.children = Some(result.clone().map_err(|_| ())); }
                    if let (Some((asked, slot)), Ok(c)) = (&mut self.space_facts, result) && asked == scope { *slot = Some(c.clone()); }
                    changed = true;
                    continue;
                }
                OperationResult::SpacePlaced { result, .. } => {
                    self.space_placed = Some(*result);
                    changed = true;
                    continue;
                }
                OperationResult::SpaceChanged { result, .. } => {
                    self.space_choices = None;
                    // The open space window reads its rooms again.
                    if let Some(v) = &self.space_view { self.op_queue.push(OperationCommand::SpaceChildren { scope: v.scope.clone() }); }
                    if let Err(e) = result { self.room_error = Some(i18n::t(matrix_engine::operations::space_error_line(*e)).into()); }
                    changed = true;
                    continue;
                }
                OperationResult::RoomMarked { result: Err(_), .. } => { self.room_error = Some(i18n::t("Operation failed").into()); changed = true; continue; }
                _ => {}
            }
            if let OperationResult::Aliases { scope, result, aliases, .. } = &result {
                if self.draft_scope.as_ref() == Some(scope) {
                    if aliases.is_some() {
                        self.aliases = aliases.clone();
                    }
                    use matrix_engine::alias::AliasOpError as E;
                    self.alias_error = result.as_ref().err().map(|e| match e {
                        E::AliasInUse => i18n::t("Address already in use"),
                        E::Forbidden => i18n::t("Not allowed"),
                        E::InvalidName => i18n::t("Invalid name"),
                        _ => i18n::t("Failed"),
                    });
                    changed = true;
                }
                continue;
            }
            match &result {
                OperationResult::MemberRights { user_id, result, .. } => {
                    if self.profile.as_ref().is_some_and(|(open, _)| open == user_id) {
                        self.rights = result.clone().ok();
                        changed = true;
                    }
                    continue;
                }
                OperationResult::BannedMembers { scope, result } => {
                    if let Some(a) = &mut self.admin && &a.scope == scope {
                        a.banned = result.as_ref().map(|list| list.clone()).map_err(|_| ());
                        changed = true;
                    }
                    continue;
                }
                OperationResult::MemberPermissions { scope, result } => {
                    if let Some(a) = &mut self.admin && &a.scope == scope {
                        a.perms = result.as_ref().ok().copied().unwrap_or_default();
                        changed = true;
                    }
                    continue;
                }
                OperationResult::Moderated { scope, user_id, action, result } => {
                    if let Some(a) = &mut self.admin && &a.scope == scope && a.done(user_id, action, result.is_ok()) {
                        if let Some((_, list)) = self.members.as_mut().filter(|(s, _)| s == scope) { list.retain(|m| &m.user_id != user_id); }
                        if let Some(list) = self.space_view.as_mut().filter(|v| &v.scope == scope).and_then(|v| v.members.as_mut()) { list.retain(|m| &m.user_id != user_id); }
                    }
                    self.moderation_failed = result.is_err();
                    // A new admin: the space is no longer «last admin», «Esci dallo spazio» follows the new facts.
                    if result.is_ok() && matches!(action, matrix_engine::operations::Moderation::SetPower(_)) && self.space_facts.as_ref().is_some_and(|(s, _)| s == scope) {
                        self.space_facts_for(scope.clone());
                    }
                    let open = self.profile.as_ref().is_some_and(|(open, _)| open == user_id);
                    self.moderating &= !open;
                    if result.is_ok() && open {
                        if let matrix_engine::operations::Moderation::SetPower(level) = action {
                            // The engine reads the local store, which follows with the next sync: the new role is shown at once and read again then.
                            let role = matrix_domain::role::MemberRole::from_power_level(*level);
                            if let Some(r) = &mut self.rights {
                                r.role = role;
                            }
                            if let Some((_, list)) = self.members.as_mut().filter(|(s, _)| s == scope) {
                                list.iter_mut().filter(|m| &m.user_id == user_id).for_each(|m| m.role = role);
                                list.sort_by(|a, b| matrix_domain::role::cmp_by_role_then_name((a.role, &a.name), (b.role, &b.name)));
                            }
                            self.roles_stale = Some(scope.clone());
                        } else {
                            // A kicked member is gone: the list is read again.
                            self.profile = None;
                            self.members_requested = None;
                        }
                    }
                    changed = true;
                    continue;
                }
                _ => {}
            }
            if let OperationResult::RoomEdited { result } = &result {
                match (result, &mut self.edit) {
                    (Ok(()), _) => self.edit = None,
                    (Err(_), Some(e)) => {
                        e.failed = true;
                        e.busy = false;
                    }
                    _ => {}
                }
                changed = true;
                continue;
            }
            if let OperationResult::Thread { tag, root, result } = result {
                if let Some(th) = &mut self.thread
                    && th.root == root
                    && th.views.accepts(&tag)
                {
                    match (result, &self.draft_scope) {
                        (Ok(page), Some(scope)) => {
                            let page = *page;
                            if let Some(before) = th.growing.take() {
                                th.stalled = page.replies.len() <= before;
                            }
                            // Rebuilt from the page: older replies land between the root and the ones held, which an append could not do.
                            let mut rows = matrix_desktop::DemoHistory::new(scope.clone(), 0);
                            rows.append_server(page.root.into_iter().chain(page.replies).collect());
                            th.history = Some(rows);
                            th.at_start = page.at_start;
                            th.failed = false;
                        }
                        _ => { th.failed = true; th.growing = None; },
                    }
                    changed = true;
                }
                continue;
            }
            if let OperationResult::ThreadRead { tag, root, target, result } = &result {
                if let Some(th) = &mut self.thread && th.root == *root && th.read_views.accepts(tag) {
                    th.read_pending = false;
                    if matches!(result, Ok(true)) { th.read_target = Some(target.clone()); }
                }
                continue;
            }
            match &result {
                OperationResult::StickerPacks { scope, result } => {
                    if self.draft_scope.as_ref() == Some(scope) { self.stickers.packs = Some(result.clone().map_err(|_| ())); }
                    changed = true;
                    continue;
                }
                OperationResult::StickerSent { result, .. } => {
                    if result.is_err() { self.room_error = Some(i18n::t("Sticker not sent").into()); }
                    changed = true;
                    continue;
                }
                _ => {}
            }
            if matches!(result, OperationResult::ThreadSent { .. }) {
                if let Some(th) = &mut self.thread { th.next_read = std::time::Instant::now(); }
                continue;
            }
            match &result {
                OperationResult::UserSearch { state, .. } => {
                    if accept_user_answer(&mut self.users_gate, state) {
                        self.users = state.clone();
                        changed = true;
                    }
                    continue;
                }
                OperationResult::RoomSearch { state } => {
                    if accept_room_answer(state.term(), &self.explore.as_ref().map_or_else(|| self.new_term.clone(), |e| e.request().map(|r| r.0).unwrap_or_default())) {
                        self.public_rooms = state.clone();
                        changed = true;
                    }
                    continue;
                }
                OperationResult::SpaceCreated { result } => {
                    self.new_busy = false;
                    match result {
                        // shortcut: the notices (not listed, picture refused) are not shown, add a line when someone misses them.
                        Ok(created) => {
                            self.pending_space = Some(created.scope.clone());
                            self.new_open = false;
                        }
                        Err(_) => self.new_failed = true,
                    }
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
                OperationResult::Join { target, phase } => {
                    // shortcut: «offline» is read from our own sync being down, use a system network probe if the two get confused.
                    let online = self.sync != Some(RoomSyncStatus::SyncFailed);
                    if let Some(join) = self.joins.get_mut(target) {
                        join.apply(phase.clone(), online);
                        if let crate::join::JoinState::Joined(scope) = join {
                            // Opened only for a user still waiting in the window that asked for it.
                            if self.quick_open || self.new_open || self.explore.is_some() {
                                self.pending_select = Some(scope.clone());
                                (self.quick_open, self.new_open) = (false, false);
                                self.close_explore();
                            }
                            self.joins.remove(target);
                        }
                    }
                    changed = true;
                    continue;
                }
                _ => {}
            }
            if let OperationResult::MessageSearch { scope, state } = result {
                let matches_term = match &state {
                    matrix_engine::operations::MessageSearchState::Idle => self.search_term.trim().len() < matrix_engine::message_match::MIN_CHARS,
                    matrix_engine::operations::MessageSearchState::Searching { term } | matrix_engine::operations::MessageSearchState::Results { term, .. } | matrix_engine::operations::MessageSearchState::NoResults { term, .. } | matrix_engine::operations::MessageSearchState::Error { term } => term == self.search_term.trim(),
                };
                if self.search_open && matches_term && self.draft_scope.as_ref() == Some(&scope) {
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
                } else if let Some(v) = &mut self.space_view && v.scope == scope {
                    v.members = Some(members.unwrap_or_default());
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

    /// Moves the space `scope` from the space `from` under `to`, before `before`; the answer lands in `space_placed`.
    pub fn place_space(&mut self, scope: matrix_domain::RoomScope, from: Option<matrix_domain::RoomScope>, to: Option<matrix_domain::RoomScope>, before: Option<matrix_domain::RoomScope>) {
        self.op_queue.push(OperationCommand::PlaceInSpace { scope, from, to, before });
    }

    pub fn room_action(&mut self, scope: matrix_domain::RoomScope, action: RoomAction) {
        self.room_error = None;
        let command = match action {
            RoomAction::Mark(read) => Some(OperationCommand::MarkRoom { scope, read, public: self.reading.2, main_only: false }),
            RoomAction::Tag(tag) => self.sidebar.toggle_tag(&scope, tag).map(OperationCommand::EditTags),
            RoomAction::Mode(mode) => self.sidebar.set_mode(&scope, mode).then_some(OperationCommand::SetNotificationMode { scope, mode }),
            RoomAction::Leave => Some(OperationCommand::LeaveRoom { scope }),
            RoomAction::Spaces => { self.space_choices = Some((scope.clone(), None)); Some(OperationCommand::SpaceChoices { scope }) }
            RoomAction::AddTo(space) => Some(OperationCommand::AddToSpace { scope, space }),
            RoomAction::RemoveFrom(space) => Some(OperationCommand::RemoveFromSpace { scope, space }),
        };
        if let Some(command) = command { self.op_queue.push(command); }
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

    pub fn create_space(&mut self, name: &str, public: bool, avatar: Option<matrix_engine::operations::SpaceAvatar>) {
        self.new_failed = false;
        self.new_busy = true;
        self.op_queue.push(OperationCommand::CreateSpace { name: name.trim().to_owned(), topic: None, public, avatar });
    }

    /// A directory row: joined through the server that listed it, like the GPUI quick search.
    pub fn join_public(&mut self, room: &matrix_engine::operations::DirectoryRoom) {
        self.start_join(room.room_id.clone(), vec![room.origin.clone()]);
    }

    pub fn open_explore(&mut self) {
        self.public_rooms = matrix_engine::search::SearchState::Idle;
        self.explore = Some(crate::explore::Explore::new());
    }

    pub fn more_rooms(&mut self) {
        self.op_queue.push(OperationCommand::MoreRooms);
    }

    pub fn close_explore(&mut self) {
        self.explore = None;
        self.public_rooms = matrix_engine::search::SearchState::Idle;
    }

    pub fn join(&mut self, target: &str) {
        self.start_join(target.trim().to_owned(), Vec::new());
    }

    /// Nothing is sent while a join of the same target is running.
    fn start_join(&mut self, target: String, via: Vec<String>) {
        self.new_failed = false;
        if self.joins.entry(target.clone()).or_default().start() {
            self.op_queue.push(OperationCommand::JoinRoom { target, via });
        }
    }

    pub fn answer_invite(&mut self, scope: matrix_domain::RoomScope, accept: bool) {
        self.op_queue.push(OperationCommand::AnswerInvite { scope, accept });
    }

    pub fn more_messages(&mut self) {
        if self.search_open && matches!(&self.search, matrix_engine::operations::MessageSearchState::Results { term, more: matrix_engine::operations::MoreHits::Available | matrix_engine::operations::MoreHits::Failed, .. } if term == self.search_term.trim()) {
            self.op_queue.push(OperationCommand::MoreMessages);
            if let matrix_engine::operations::MessageSearchState::Results { more, .. } = &mut self.search {
                *more = matrix_engine::operations::MoreHits::Loading;
            }
        }
    }

    pub fn open_thread(&mut self, root: EventId) {
        self.thread = Some(ThreadState { root, views: Default::default(), history: None, at_start: false, heights: Default::default(), failed: false, draft: String::new(), attachment: None, voice: None, reply_to: None, menu: None, want: 50, growing: None, stalled: false, tail_visible: false, read_views: Default::default(), read_target: None, read_pending: false, read_public: None, read_at: None, next_read: std::time::Instant::now() });
        self.info_open = false;
        self.search_open = false;
    }

    pub fn send_thread(&mut self, body: String) {
        let (Some(th), Some(scope)) = (&mut self.thread, self.draft_scope.clone()) else { return };
        let attachment = th.attachment.take().map(|(filename, data)| matrix_engine::sending::Attachment { filename, data, voice: th.voice.take() });
        self.op_queue.push(OperationCommand::ThreadSend { scope, root: th.root.clone(), reply_to: th.reply_to.take().map(|(id, _)| id), body, attachment });
    }

    /// Votes for option `ix` of the poll started by `target` (the engine replaces own votes; the same option again withdraws).
    pub fn vote(&mut self, target: EventId, ix: usize) {
        let (Some(scope), Some(messages)) = (self.draft_scope.clone(), &self.messages) else { return };
        let Some(poll) = messages.events().iter().find(|e| e.id.id == TimelineId::Server(target.clone())).and_then(|e| e.poll.clone()) else { return };
        self.ops.vote(scope, target, &poll, ix, matrix_engine::sending::fresh_transaction_id());
    }

    pub fn send_poll(&mut self, draft: matrix_domain::PollDraft) {
        let (Some(scope), Some((tx, _))) = (self.draft_scope.clone(), &mut self.send) else { return };
        let request = matrix_engine::sending::SendRequest {
            scope,
            transaction_id: matrix_engine::sending::fresh_transaction_id(),
            body: String::new(),
            attachment: None,
            poll: Some(draft),
            mentions: Vec::new(),
        };
        let _ = tx.try_send(request);
    }

    pub fn open_edit(&mut self) {
        if let Some(scope) = self.sidebar.selected().map(|r| r.scope.clone()) { self.open_edit_of(&scope); }
    }

    /// «Modifica» of a room or of a space.
    pub fn open_edit_of(&mut self, scope: &matrix_domain::RoomScope) {
        let Some(room) = self.sidebar.rooms().iter().find(|r| &r.scope == scope) else { return };
        let (name, topic) = (room.name.clone(), room.info.topic.clone().unwrap_or_default());
        self.edit = Some(RoomEdit { name: name.clone(), topic: topic.clone(), avatar: None, failed: false, busy: false, scope: scope.clone(), original: (name, topic) });
    }

    /// «⋮» of the space was opened: asks again what «Esci dallo spazio» depends on; the last answer for the same space stays meanwhile.
    pub fn space_facts_for(&mut self, scope: matrix_domain::RoomScope) {
        let known = self.space_facts.take().filter(|(s, _)| *s == scope).and_then(|(_, c)| c);
        self.op_queue.push(OperationCommand::SpaceChildren { scope: scope.clone() });
        self.space_facts = Some((scope, known));
    }

    /// «Esci» of the confirmation window; one leave at a time.
    pub fn leave_space(&mut self, scope: matrix_domain::RoomScope, forget: bool) {
        self.room_error = None;
        if self.sidebar.begin_space_leave(scope.clone()) { self.op_queue.push(crate::space::leave_command(scope, forget)); }
    }

    /// Opens the space window on its rooms or on its members and asks the engine for that list.
    pub fn open_space(&mut self, scope: matrix_domain::RoomScope, members_page: bool) {
        self.room_error = None;
        self.op_queue.push(if members_page { OperationCommand::Members { scope: scope.clone() } } else { OperationCommand::SpaceChildren { scope: scope.clone() } });
        self.space_view = Some(crate::space::SpaceView { scope, members_page, children: None, members: None, adding: None, promote: false });
    }

    /// «Passa il ruolo di amministratore…»: the members page with a «Rendi amministratore» button per candidate.
    pub fn open_promote(&mut self, scope: matrix_domain::RoomScope) {
        self.moderation_failed = false;
        self.open_space(scope, true);
        if let Some(v) = &mut self.space_view { v.promote = true; }
    }

    /// «Elimina spazio» of the confirmation window: one deletion at a time.
    pub fn delete_space(&mut self, scope: matrix_domain::RoomScope) {
        if !std::mem::replace(&mut self.space_deleting, true) {
            self.op_queue.push(OperationCommand::DeleteSpace { scope });
        }
    }

    /// Sends only what changed, so a field the user may not edit is never touched.
    pub fn save_edit(&mut self) {
        let Some(e) = &mut self.edit else { return };
        let scope = e.scope.clone();
        let changed = |new: &str, old: &str| (new.trim() != old).then(|| new.trim().to_owned());
        let (name, topic) = (changed(&e.name, &e.original.0), changed(&e.topic, &e.original.1));
        if name.is_none() && topic.is_none() && e.avatar.is_none() {
            self.edit = None;
            return;
        }
        e.busy = true;
        e.failed = false;
        self.op_queue.push(OperationCommand::EditRoom { scope, name, topic, avatar: e.avatar.take() });
    }

    pub fn alias_action(&mut self, action: matrix_engine::alias::AliasAction) {
        let Some(scope) = self.draft_scope.clone() else { return };
        self.alias_error = None;
        self.op_queue.push(OperationCommand::Aliases { scope, action });
    }

    pub fn mention_profile(&mut self, user_id: String, name: &str) {
        append_profile_mention(&mut self.draft, &mut self.mentions, user_id, name);
    }

    pub fn open_profile(&mut self, user_id: String) {
        self.new_failed = false;
        self.rights = None;
        (self.moderation_failed, self.moderating) = (false, false);
        if let Some(scope) = self.draft_scope.clone() {
            self.op_queue.push(OperationCommand::MemberRights { scope, user_id: user_id.clone() });
        }
        self.op_queue.push(OperationCommand::Profile { user_id: user_id.clone() });
        self.profile = Some((user_id, None));
    }

    pub fn moderate(&mut self, user_id: String, action: matrix_engine::operations::Moderation) {
        let Some(scope) = self.draft_scope.clone() else { return };
        (self.moderation_failed, self.moderating) = (false, true);
        self.op_queue.push(OperationCommand::Moderate { scope, user_id, action });
    }

    /// Reads the rights and the banned users of the list on screen; the answers of another room are dropped.
    pub fn admin_for(&mut self, scope: Option<matrix_domain::RoomScope>) {
        if self.admin.as_ref().map(|a| &a.scope) == scope.as_ref() {
            return;
        }
        if let Some(scope) = &scope {
            self.op_queue.push(OperationCommand::MemberPermissions { scope: scope.clone() });
            self.op_queue.push(OperationCommand::BannedMembers { scope: scope.clone() });
        }
        self.admin = scope.map(crate::members::Admin::new);
    }

    pub fn moderate_in(&mut self, scope: matrix_domain::RoomScope, user_id: String, action: matrix_engine::operations::Moderation) {
        self.op_queue.push(OperationCommand::Moderate { scope, user_id, action });
    }

    /// What was clicked in a member list.
    pub fn member_act(&mut self, act: crate::members::Act) {
        use crate::members::Act;
        if let Act::Profile(user) = act {
            return self.open_profile(user);
        }
        let Some(a) = &mut self.admin else { return };
        a.failed = false;
        match act {
            Act::Ask { user_id, name, ban } => a.dialog = Some(crate::members::Dialog { user_id, name, ban, reason: String::new() }),
            Act::Invite => {
                a.inviting = true;
                self.new_term.clear();
            }
            Act::Unban(user_id) => {
                a.busy = true;
                let scope = a.scope.clone();
                self.moderate_in(scope, user_id, matrix_engine::operations::Moderation::Unban(None));
            }
            Act::Profile(_) => {}
        }
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
            self.confirm_end_poll = false;
        }
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
        self.confirm_delete = false;
        self.confirm_end_poll = false;
        self.ops.clear_selection();
    }

    pub fn open_viewer(&mut self, id: EventId, video: bool, name: String) {
        let Some(scope) = self.draft_scope.clone() else { return };
        self.media.cancel_original();
        if !video { self.media.request_original(scope, id.clone()); }
        self.viewer = Some(crate::viewer::Viewer::new(id, video, name));
    }

    pub fn show_viewer(&mut self, ctx: &egui::Context) {
        let Some(viewer) = &mut self.viewer else { return };
        let action = viewer.show(ctx, self.pictures.get(&viewer.id));
        match action {
            crate::viewer::Action::Close => { self.media.cancel_original(); self.viewer = None; }
            crate::viewer::Action::Save(path) => {
                if path.exists() {
                    self.notice = Some(i18n::t("Choose a file that does not exist yet").to_owned());
                    return;
                }
                if let Some(scope) = self.draft_scope.clone() {
                    self.share_after = path.clone();
                    self.notice = (!self.media.request_save(scope, viewer.id.clone(), path)).then(|| i18n::t("A save is already in progress").to_owned());
                }
            }
            crate::viewer::Action::Open => {
                if let Some(scope) = self.draft_scope.clone() { viewer.failed = false; self.media.request_video(scope, viewer.id.clone()); }
            }
            crate::viewer::Action::None => {}
        }
    }

    /// Saves an attachment into the Download folder under a safe, unused name.
    pub fn save(&mut self, event: EventId, name: &str) {
        let Some(scope) = self.draft_scope.clone() else { return };
        let dir = crate::pick::save_dir();
        let base = crate::pick::safe_name(name);
        let mut path = dir.join(&base);
        for n in 2.. {
            if !path.exists() {
                break;
            }
            path = dir.join(format!("{n}-{base}"));
        }
        let _ = std::fs::create_dir_all(&dir);
        self.share_after = path.clone();
        self.notice = (!self.media.request_save(scope, event, path)).then(|| i18n::t("A save is already in progress").to_owned());
    }

    /// Takes a dropped file as the pending attachment (up to the server's upload limit, 50 MB until it is known).
    /// `thread`: for the open thread's composer instead of the room's.
    pub fn attach(&mut self, path: &std::path::Path, thread: bool) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let data = std::fs::metadata(path).ok().filter(|m| 0 < m.len() && m.len() <= self.upload_limit.unwrap_or(50 << 20)).and_then(|_| std::fs::read(path).ok());
        match (name, data) {
            (Some(name), Some(data)) => self.set_attachment(thread, name, data, None),
            _ => self.notice = Some(i18n::t("File cannot be attached").to_owned()),
        }
    }

    pub fn attach_image(&mut self, image: &egui::ColorImage, thread: bool) {
        match crate::composer::pasted_png(image).filter(|data| !data.is_empty() && data.len() as u64 <= self.upload_limit.unwrap_or(50 << 20)) {
            Some(data) => self.set_attachment(thread, "Immagine.png".to_owned(), data, None),
            None => self.notice = Some(i18n::t("Image cannot be attached").to_owned()),
        }
    }

    /// The pending attachment of the room's composer or of the thread's.
    fn set_attachment(&mut self, thread: bool, name: String, data: Vec<u8>, voice: Option<matrix_engine::sending::Voice>) {
        if !thread {
            (self.attachment, self.voice) = (Some((name, data)), voice);
        } else if let Some(th) = &mut self.thread {
            (th.attachment, th.voice) = (Some((name, data)), voice);
        }
        self.notice = None;
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

    /// Asks for an avatar as uploaded (not the thumbnail), to retouch it; `None` while the account is not connected.
    pub fn avatar_original(&self, mxc: &str) -> Option<std::sync::mpsc::Receiver<matrix_engine::avatar::AvatarPicture>> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.avatar_slot.try_lock().ok()?.as_ref()?.request_original(mxc.to_owned(), tx);
        Some(rx)
    }

    /// The account's own picture, once downloaded (the spaces rail and the tray draw it).
    pub fn own_avatar(&self) -> crate::avatar::Avatar {
        crate::avatar::lookup(&self.avatars, self.verification.as_ref().and_then(|v| v.profile_avatar()))
    }

    pub fn react(&mut self, key: &str) {
        self.ops.toggle_reaction(key, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    pub fn end_poll(&mut self) {
        self.ops.submit(Operation::EndPoll, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    pub fn delete(&mut self) {
        self.ops.submit(Operation::Redact, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    fn close_menu_keep_selection(&mut self) {
        self.menu = None;
        self.confirm_delete = false;
        self.confirm_end_poll = false;
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
            attachment: self.attachment.take().map(|(filename, data)| matrix_engine::sending::Attachment { filename, data, voice: self.voice.take() }),
            poll: None,
            mentions: std::mem::take(&mut self.mentions),
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

    /// The «Cronologia cifrata» step is open: signed in, recovery offered, being checked or failed.
    pub fn recovering(&self) -> bool {
        recovery_open(self.form.recovery())
    }

    pub fn defer_recovery(&mut self) {
        let Some(source) = self.form.source() else { return };
        let command = if self.form.recovery_key().is_some() {
            self.form.confirm_saved_source(source)
        } else {
            self.form.defer_source(source)
        };
        if let Some(command) = command && self.ui.commands.try_send(command).is_err() {
            self.form.apply(SignInEvent::Failed(signin::SignInError::NoSession));
        }
    }

    pub fn setup_recovery(&mut self) {
        if let Some(command) = self.form.source().and_then(|source| self.form.setup_source(source))
            && self.ui.commands.try_send(command).is_err()
        {
            self.form.apply_setup(signin::SetupEvent::Failed(signin::SetupFailure::Other));
        }
    }

    /// Sends the recovery key typed in `phrase`; it never stays in the form.
    pub fn recover(&mut self) {
        self.form.phrase.replace(None, &self.phrase);
        self.phrase.clear();
        if let Some(command) = self.form.source().and_then(|src| self.form.recover_source(src)) {
            let _ = self.ui.commands.try_send(command);
        }
    }

    /// Seconds recorded so far, for the composer the recording belongs to.
    pub fn recording(&self, thread: bool) -> Option<u64> {
        self.recorder.as_ref().filter(|_| self.record_thread == thread).map(|r| r.elapsed_secs())
    }

    /// One microphone: a second start (the other composer) while a note is being recorded does nothing.
    pub fn start_recording(&mut self, thread: bool) {
        if self.recorder.is_some() { return; }
        match matrix_desktop::voice_recorder::Recorder::start(self.upload_limit.unwrap_or(50 << 20)) {
            Ok(recorder) => {
                self.record_thread = thread;
                self.recorder = Some(recorder);
                self.notice = None;
            }
            Err(error) => self.notice = Some(error.to_owned()),
        }
    }

    /// Ends the recording; `send` puts the note out like «Invia nota», otherwise it is thrown away.
    pub fn stop_recording(&mut self, send: bool) {
        let Some(recorder) = self.recorder.take() else { return };
        let (samples, rate) = recorder.finish();
        if !send {
            return;
        }
        match matrix_desktop::voice_attachment(&samples, rate, self.upload_limit.unwrap_or(50 << 20)) {
            Some(note) => {
                let thread = self.record_thread;
                // A thread closed meanwhile: the note has nowhere to go and is dropped.
                self.set_attachment(thread, note.filename, note.data, note.voice);
                if thread { self.send_thread(String::new()) } else { self.send(String::new()) }
            }
            None => self.notice = Some(i18n::t("No audio recorded").to_owned()),
        }
    }

    /// Play / pause of a voice note; the first press downloads and decodes it, which starts it.
    pub fn play_audio(&mut self, id: EventId) {
        if let Some((current, out)) = &self.audio
            && *current == id
        {
            out.player().lock().unwrap_or_else(|e| e.into_inner()).toggle();
        } else if let Some(scope) = self.draft_scope.clone() {
            self.audio = None;
            self.media.request_audio(scope, id);
        }
    }

    /// (note, playing, progress) of the loaded note.
    pub fn audio_now(&self) -> Option<(EventId, bool, f32)> {
        let (id, out) = self.audio.as_ref()?;
        let p = out.player();
        let p = p.lock().unwrap_or_else(|e| e.into_inner());
        Some((id.clone(), p.is_playing(), p.progress()))
    }

    pub fn status(&self) -> FormStatus {
        self.form.status()
    }
}

fn recovery_open(state: Option<RecoveryState>) -> bool {
    matches!(state, Some(RecoveryState::Offered | RecoveryState::Pending | RecoveryState::Failed(_) | RecoveryState::Unsupported))
}

fn append_profile_mention(draft: &mut String, mentions: &mut Vec<matrix_engine::sending::Mention>, user_id: String, name: &str) {
    if !draft.is_empty() && !draft.ends_with(char::is_whitespace) { draft.push(' '); }
    let label = format!("@{}", name.trim_start_matches('@'));
    draft.push_str(&label);
    draft.push(' ');
    mentions.push(matrix_engine::sending::Mention { user_id, label });
}

#[test]
fn profile_mention_preserves_draft_and_user() {
    let mut draft = "Ciao".to_owned();
    let mut mentions = Vec::new();
    append_profile_mention(&mut draft, &mut mentions, "@alice:example.org".into(), "@Àlice");
    assert_eq!(draft, "Ciao @Àlice ");
    assert_eq!(mentions[0].user_id, "@alice:example.org");
    assert_eq!(mentions[0].label, "@Àlice");
}

/// Room-directory answers name their term (engine `SearchState`): one for a term no longer typed is stale.
fn accept_room_answer(answer: Option<&str>, typed: &str) -> bool {
    answer.is_none_or(|t| t == typed.trim())
}

/// User-search answers carry no term, so order decides: after a new request, anything but its
/// first Searching/Idle is an older query's answer. shortcut: a coalesced Searching would leave the gate shut until the next keystroke.
fn accept_user_answer(gate: &mut bool, state: &matrix_engine::operations::SearchState) -> bool {
    use matrix_engine::operations::SearchState as S;
    if matches!(state, S::Searching | S::Idle) {
        *gate = false;
        return true;
    }
    !*gate
}

/// A saved position whose row is not loaded: wait for the saved page and the full one (both can come after the first), page back up to a cap, give up when history ends or fails.
fn restore_gave_up(status: matrix_desktop::HistoryStatus, pages: u8, waited: std::time::Duration) -> bool {
    use matrix_desktop::HistoryStatus::*;
    pages >= 12 || matches!(status, Failed) || (matches!(status, Start) && (pages >= 2 || waited.as_secs() >= 3))
}

/// Start-up reopening of the saved room `want` ends when it is on screen, when the user opened a room themselves
/// (the list's own first pick, moved or not by later diffs, is not a choice), or after a while (a room that left the list must not pop up later).
fn restore_over(want: &str, now: Option<&str>, user_picked: bool, waited: std::time::Duration) -> bool {
    now == Some(want) || user_picked || waited > std::time::Duration::from_secs(20)
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    use matrix_engine::operations::SearchState as S;

    #[test]
    fn saved_room_is_reopened_unless_the_user_chose_or_time_passed() {
        let s = std::time::Duration::from_secs;
        assert!(!restore_over("!last", Some("!first"), false, s(1)));
        // the list's pick moved to another room by itself (a diff): still waiting
        assert!(!restore_over("!last", Some("!other"), false, s(1)));
        assert!(restore_over("!last", Some("!first"), true, s(1)));
        assert!(restore_over("!last", Some("!last"), false, s(1)));
        assert!(restore_over("!last", Some("!first"), false, s(21)));
    }

    #[test]
    fn saved_position_waits_for_both_emissions_then_gives_up() {
        use matrix_desktop::HistoryStatus::*;
        let s = std::time::Duration::from_secs;
        assert!(!restore_gave_up(Start, 1, s(0))); // saved page lacks the anchor, the full one is still to come
        assert!(!restore_gave_up(More, 5, s(30)));
        assert!(restore_gave_up(Start, 2, s(0)));
        assert!(restore_gave_up(Start, 1, s(3)));
        assert!(restore_gave_up(More, 12, s(0)));
        assert!(restore_gave_up(Failed, 1, s(0)));
    }

    #[test]
    fn recovery_screen_stays_open_for_first_setup_and_closes_after_confirmation() {
        assert!(recovery_open(Some(RecoveryState::Unsupported)));
        assert!(recovery_open(Some(RecoveryState::Offered)));
        assert!(!recovery_open(Some(RecoveryState::Deferred)));
        assert!(!recovery_open(Some(RecoveryState::Recovered)));
    }

    #[test]
    fn setup_key_requires_explicit_saved_confirmation() {
        use matrix_engine::signin::{SetupEvent, SignInCommand};
        let mut form = SignInForm::default();
        form.apply(SignInEvent::Recovery(RecoveryState::Unsupported));
        assert!(matches!(form.setup_source(form.source().unwrap()), Some(SignInCommand::SetupRecovery)));
        assert!(form.source().is_none());
        form.apply_setup(SetupEvent::Ready(matrix_engine::recovery::RecoverySecret::fixture("fixture key")));
        assert_eq!(form.recovery_key(), Some("fixture key"));
        let source = form.source().unwrap();
        assert!(form.defer_source(source).is_none());
        assert!(matches!(form.confirm_saved_source(source), Some(SignInCommand::DeferRecovery)));
        assert!(form.recovery_key().is_none());
    }

    #[test]
    fn older_answers_are_dropped() {
        assert!(accept_room_answer(Some("caf"), " caf "));
        assert!(!accept_room_answer(Some("ca"), "caf"));
        assert!(accept_room_answer(None, "caf"));
        let mut gate = true;
        assert!(!accept_user_answer(&mut gate, &S::NoResults));
        assert!(accept_user_answer(&mut gate, &S::Searching));
        assert!(accept_user_answer(&mut gate, &S::NoResults));
    }
}
