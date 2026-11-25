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
    pub attachment_revision: u64,
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
    reconnecting: bool,
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
    /// The message whose picture is being fetched for the sticker editor.
    pub sticker_from: Option<EventId>,
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
    /// Waveform of the decoded note in `audio`: what the bubble draws when the sender sent none.
    audio_wave: Vec<f32>,
    /// A voice note being recorded (the microphone is open); `voice` is the note waiting to be sent with the next send.
    pub recorder: Option<matrix_desktop::voice_recorder::Recorder>,
    /// The recording was started from the thread's composer: its bar shows there and the note goes into the thread.
    record_thread: bool,
    voice: Option<matrix_engine::sending::Voice>,
    /// Set by the app each frame: the window has focus, the newest message is in view, the «Conferme di lettura» setting.
    pub reading: (bool, bool, bool),
    read_dirty: bool,
    /// Debounce for the receipt after a new event: nothing goes out before this.
    read_after: Option<std::time::Instant>,
    /// Rooms seen with unread messages: when one reaches zero its delivered alerts are removed.
    alerted_unread: std::collections::HashSet<String>,
    /// The shown room was opened with unread messages: first-unread anchor and progressive reading.
    unread_open: Option<unread::UnreadOpen>,
    pub unread_notice: Option<std::time::Instant>,
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
    sharing: bool,
    /// Composer (thread or room) that asked for the microphone while the permission prompt was up.
    mic_pending: Option<bool>,
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
    /// The DM being started is the feedback one: its composer gets the diagnostic line, a failure is a list line.
    feedback_pending: bool,
    /// Joins asked from the quick search or «Unisciti», by the command's target; a successful one is removed.
    pub joins: std::collections::HashMap<String, crate::join::JoinState>,
    /// «Crea spazio» was sent and its answer is not in yet.
    pub new_busy: bool,
    pub pending_select: Option<matrix_domain::RoomScope>,
    /// `pending_select` just landed on a room: the one-column shell leaves the list for it (also when the room was already listed).
    pub opened: bool,
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
    /// Messages of every room that match the room list's search field, and the term already sent for them.
    pub all_search: matrix_engine::operations::MessageSearchState,
    all_sent: String,
    /// A message chosen in `all_search` of a room that is not open yet: the jump waits for the room.
    jump_after_open: Option<(matrix_domain::RoomScope, (EventId, bool))>,
    /// The message a jump landed on and when: drawn lit for a moment.
    pub flash: Option<(EventId, std::time::Instant)>,
    /// The «who reacted» popup: the pill it hangs from and the reaction it lists (a snapshot).
    pub who: Option<(eframe::egui::Rect, matrix_engine::history::Reaction)>,
    media: LiveMedia,
    pub viewer: Option<crate::viewer::Viewer>,
    /// The full picture of the avatar the viewer shows, on its way.
    avatar_full: Option<std::sync::mpsc::Receiver<matrix_engine::avatar::AvatarPicture>>,
    /// The member card was opened from a message, not from the info panel: it stands alone (the info panel is off) and holds whether the panel was open before, to give it back on closing.
    pub chat_profile: Option<bool>,
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
    /// The menu was opened by a long press: the message is lifted and the touch menu is shown instead (it outlives `menu` while it fades out).
    pub lift: Option<crate::touch_menu::Lift>,
    /// File dropped on the window, sent with the next Enter (the text is its caption).
    pub attachment: Option<(String, Vec<u8>)>,
    pub attachment_revision: u64,
    /// Short result line for the last save or refused attachment.
    pub notice: Option<String>,
    pub notice_shown: Option<(String, std::time::Instant)>,
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
    /// Since when the room list has been waiting for the first sync.
    pub list_wait: crate::loading::Clock,
    pub homeserver: String,
    pub user: String,
    pub password: String,
    pub phrase: String,
}

/// A new message in the open room is read after this quiet time, so a burst sends one receipt.
const READ_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

impl Session {
    /// Opens the default profile under the app data directory; a saved account unlocks by itself through the OS vault.
    /// Remote to end this account's worker with a server logout («Esci»).
    pub fn control(&self) -> matrix_engine::WorkerControl {
        self._worker.control()
    }

    pub fn start(dirs: &AppDirs, root: std::path::PathBuf, trust: Option<Vec<u8>>) -> Result<Session, String> {
        Self::start_mode(dirs, root, trust, false)
    }

    pub fn start_mode(dirs: &AppDirs, root: std::path::PathBuf, trust: Option<Vec<u8>>, password_login: bool) -> Result<Session, String> {
        matrix_desktop::create_private_dir(&root).map_err(|e| e.to_string())?;
        let status_dir = root.clone();
        let last_file = root.join("last_room");
        let sticker_file = root.join("stickers.json");
        let scroll_file = root.join("scroll_pos.json");
        let last_room = std::fs::read_to_string(&last_file).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let vault = signin::platform_source();
        let with_vault = vault.is_some();
        let (ui, pending) = match vault {
            Some(source) => signin::interactive_with_source(root, trust, source),
            None => signin::interactive(root, trust),
        };
        let pending = if password_login { pending.with_password_login() } else { pending };
        let (mut worker, rooms) = RoomListWorker::spawn(move || async move {
            pending.run().await.map_err(|_| matrix_engine::auth::AuthError::Storage)
        })
        .map_err(|e| e.to_string())?;
        crate::suspend::register(worker.control());
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
            reconnecting: false,
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
            audio_wave: Vec::new(),
            recorder: None,
            record_thread: false,
            voice: None,
            reading: (false, false, true),
            read_dirty: true,
            read_after: None,
            alerted_unread: Default::default(),
            unread_open: None,
            unread_notice: None,
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
            sharing: false,
            mic_pending: None,
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
            feedback_pending: false,
            joins: Default::default(),
            new_busy: false,
            pending_space: None,
            pending_select: None,
            opened: false,
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
            all_search: matrix_engine::operations::MessageSearchState::Idle,
            all_sent: String::new(),
            jump_after_open: None,
            flash: None,
            who: None,
            media: LiveMedia::default(),
            viewer: None,
            avatar_full: None,
            chat_profile: None,
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
            lift: None,
            attachment: None,
            attachment_revision: 0,
            notice: None,
            notice_shown: None,
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
            stickers: crate::stickers::Panel::load(sticker_file),
            sticker_from: None,
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
            list_wait: Default::default(),
            homeserver: SignInForm::default().homeserver,
            user: String::new(),
            password: String::new(),
            phrase: String::new(),
        })
    }

    /// Reconnect the transport without replacing the visible room, drafts or scroll state.
    pub fn reconnect(&mut self, dirs: &AppDirs, root: std::path::PathBuf, trust: Option<Vec<u8>>) -> Result<(), String> {
        let mut new = Self::start(dirs, root, trust)?;
        if self.account.is_none() {
            *self = new;
            return Ok(());
        }
        std::mem::swap(&mut self.form, &mut new.form);
        std::mem::swap(&mut self.ui, &mut new.ui);
        std::mem::swap(&mut self._worker, &mut new._worker);
        std::mem::swap(&mut self.rooms, &mut new.rooms);
        std::mem::swap(&mut self.route, &mut new.route);
        std::mem::swap(&mut self.send, &mut new.send);
        std::mem::swap(&mut self.op_route, &mut new.op_route);
        std::mem::swap(&mut self.ver_route, &mut new.ver_route);
        std::mem::swap(&mut self.media_route, &mut new.media_route);
        std::mem::swap(&mut self.avatar_slot, &mut new.avatar_slot);
        self.history = HistorySession::new();
        self.pending = self.history.select(self.sidebar.selected().map(|r| r.scope.clone()));
        self.reconnecting = true;
        Ok(())
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
                    let reconnecting = self.reconnecting && self.account == Some(account);
                    self.account = Some(account);
                    if reconnecting {
                        if let Some(v) = &mut self.verification { v.refresh(); }
                    } else {
                        let mut v = matrix_desktop::DeviceVerification::new(account);
                        v.refresh();
                        self.verification = Some(v);
                        self.notifications = matrix_domain::notifications::NotificationPolicy::new(account);
                        self.sidebar = matrix_desktop::Sidebar::connected(account);
                    }
                }
                SignInEvent::Recovery(RecoveryState::Offered) if self.reconnecting => self.defer_recovery(),
                _ => {}
            }
        }
        while let Ok(event) = self.rooms.try_recv() {
            changed = true;
            match event {
                RoomListEvent::Status(s) => {
                    if s == RoomSyncStatus::Ready { self.reconnecting = false; }
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
                    if self.sidebar.selected().is_some_and(|r| r.scope == scope) {
                        self.read_dirty = true;
                        self.read_after = Some(std::time::Instant::now() + READ_DEBOUNCE);
                    }
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
        if let Some(thread) = self.mic_pending && matrix_desktop::voice_recorder::permission_granted() {
            self.mic_pending = None;
            self.start_recording(thread, ctx);
            changed = true;
        }
        changed |= self.drive_media(ctx);
        changed |= self.drive_stickers();
        changed |= self.drive_avatars(ctx);
        changed
    }

    /// While the Sticker tab is shown: asks for the packs of the open room (again after 30 s) and queues their pictures as avatars.
    fn drive_stickers(&mut self) -> bool {
        let Some(scope) = self.draft_scope.clone() else { return false };
        if self.stickers.scope.as_ref() != Some(&scope) {
            // The account's packs are the same in every room: keep them on screen until the new room's answer.
            (self.stickers.scope, self.stickers.fetched) = (Some(scope.clone()), None);
        }
        let open = self.stickers.wants_packs();
        if cfg!(debug_assertions) && matrix_desktop::hooks::var_os("GNOBBI_DEBUG_STICKER_FIXTURE").is_some() {
            self.stickers.edits.clear(); // painted packs, nothing to ask or change on a server
            self.stickers.busy = false;
            return false;
        }
        for edit in self.stickers.edits.drain(..) {
            self.op_queue.push(OperationCommand::StickerEdit { edit });
        }
        if open && self.stickers.fetched.is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(30)) {
            self.stickers.fetched = Some(std::time::Instant::now());
            self.op_queue.push(OperationCommand::StickerPacks { scope });
        }
        false
    }

    /// «Add to my stickers» on a picture or sticker message: its original, then the editor.
    /// An `animated` one (GIF, WebP) is saved as it is, with its own mxc, never through the editor's PNG.
    pub fn sticker_from_message(&mut self, id: EventId, animated: bool) {
        let Some(scope) = self.draft_scope.clone() else { return };
        if animated {
            self.stickers.edit(matrix_engine::stickers::PackEdit::AddMessage { pack: None, default_name: i18n::t("Stickers").to_owned(), room: scope.room_id.as_str().to_owned(), event: id.as_str().to_owned() });
            return;
        }
        self.media.cancel_original();
        self.media.request_original(scope, id.clone());
        self.sticker_from = Some(id);
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
            if let Some(frames) = p.anim { crate::animation::insert(p.mxc.clone(), frames); }
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
        let mut wanted: Vec<String> = ctx.data_mut(|d| d.remove_temp::<Vec<String>>(egui::Id::new(crate::avatar::MENTION_AVATARS))).unwrap_or_default();
        wanted.extend(self.sidebar.rooms().iter().filter_map(|r| r.info.avatar.clone()));
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
        if let Some((_, Some(p))) = &self.profile && let Some(mxc) = &p.avatar && self.avatar_wanted(&matrix_engine::avatar::large_key(mxc)) {
            self.avatars.insert(matrix_engine::avatar::large_key(mxc), Avatar::Loading);
            source.request_large(mxc.clone(), self.avatar_chan.0.clone());
        }
        if let Some(mxc) = crate::animation::take_wanted() { source.request_animated(mxc, self.avatar_chan.0.clone()); }
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
        if let Some(ok) = self.media.take_saved().map(|(_, r)| r.is_ok()) {
            if ok && cfg!(target_os = "ios") {
                // iOS: the file sits in a private folder until the share sheet takes it; «Saved» waits for the sheet's answer.
                crate::pick::share(&crate::pick::with_extension(&std::mem::take(&mut self.share_after)));
                self.sharing = true;
            } else {
                    self.notice_shown = None;
                self.notice = Some(i18n::t(if ok { "Saved" } else { "Saving failed" }).to_owned());
            }
            changed = true;
        }
        if self.sharing {
            // The sheet's answer arrives from UIKit, which does not wake egui: poll for it.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            if let Some(result) = crate::pick::shared() {
                self.sharing = false;
                if let Some(ok) = result {
                    self.notice_shown = None;
                    self.notice = Some(i18n::t(if ok { "Saved" } else { "Saving failed" }).to_owned());
                }
                changed = true;
            }
        }
        if let Some((id, result)) = self.media.take_audio() {
            match result {
                Ok(opened) => {
                    self.audio_wave = opened.waveform.clone();
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
                        if let Some(frames) = p.anim { crate::animation::insert(id.as_str().to_owned(), frames); }
                    }
                }
            }
        }
        if let Some((id, result)) = self.media.take_original() {
            if self.sticker_from.as_ref() == Some(&id) {
                self.sticker_from = None;
                match result {
                    Ok(p) => self.stickers.add_rgba(p.width, p.height, p.rgba, ""),
                    Err(_) => self.stickers.error = Some(i18n::t("Image cannot be used")),
                }
            } else if let Some(viewer) = &mut self.viewer && viewer.id == id { viewer.set_original(ctx, result); }
            changed = true;
        }
        if let Some(rx) = &self.avatar_full && let Ok(p) = rx.try_recv() {
            self.avatar_full = None;
            if let (Some(viewer), Some(rgba)) = (&mut self.viewer, p.rgba) { viewer.set_avatar(ctx, rgba); }
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
            crate::animation::clear();
            self.members = None;
            self.thread = None;
            self.upload_limit = None;
            self.search_sent.clear();
            self.search_jump = self.jump_after_open.take().filter(|(room, _)| Some(room) == scope.as_ref()).map(|(_, jump)| jump);
            self.search = matrix_engine::operations::MessageSearchState::Idle;
            self.heights = Default::default();
            self.heights.restore = scope.as_ref().and_then(|s| self.scroll.get(s.room_id.as_str()));
            self.start_unread();
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
        if !crate::loading::debug_hold(false) && let Some((_, rx)) = &mut self.route {
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
            if !live { self.heights.restore_wait.0 = self.heights.restore_wait.0.saturating_add(1); }
        }
        if !self.unread_pending() && let (Some((id, _)), Some(messages)) = (&self.heights.restore, &self.messages) {
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
            self.pending = if self.unread_pending() { self.unread_history_step() } else { self.history.load_older_near_top(if self.search_jump.is_some() { 0 } else { self.first_visible }) };
        }
        if let (Some(request), Some((tx, _))) = (self.pending.take(), &mut self.route) {
            if let Err(e) = tx.try_send(request) {
                match e {
                    matrix_engine::TrySendError::Full(r) | matrix_engine::TrySendError::Busy(r) => self.pending = Some(r),
                    matrix_engine::TrySendError::Closed(_) => {}
                }
            } else {
                crate::loading::debug_hold(true);
            }
        }
        changed
    }

    /// Where the open room's history request stands, for the loading feedback.
    pub fn fetch(&self) -> crate::loading::Fetch {
        use matrix_desktop::HistoryStatus::*;
        match self.history.status() {
            Loading => crate::loading::Fetch::Busy,
            // The first-unread search tries again by itself: still loading, not a failure to retry by hand.
            Failed if self.unread_pending() => crate::loading::Fetch::Busy,
            Failed => crate::loading::Fetch::Failed,
            // A first page without rows but with more behind it: the older one is asked for at once.
            More if self.messages.as_ref().is_some_and(|m| m.events().is_empty()) => crate::loading::Fetch::Busy,
            _ => crate::loading::Fetch::Done,
        }
    }

    /// «Retry» on a failed history load: the same request again (a failed live read is taken up by the next refresh).
    pub fn retry_history(&mut self) {
        if self.pending.is_none() {
            self.pending = self.history.load_older();
        }
    }

    fn drive_operations(&mut self) -> bool {
        let mut changed = self.follow_pending_open();
        changed |= self.restore_or_remember_room();
        // Opening or receiving in the room on screen: the read marker goes out once the newest message is in view (main only, so threads stay unread).
        changed |= self.drive_unread();
        self.queue_read_marker();
        self.clear_read_alerts();
        self.queue_lookups();
        self.queue_thread_requests();
        self.queue_aliases_load();
        self.send_next_command();
        for result in self.take_results() {
            changed |= self.handle_result(result);
        }
        changed
    }

    /// Selects the room just started or joined as soon as the list has it; opens a space just created.
    fn follow_pending_open(&mut self) -> bool {
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
            self.opened = true;
            self.pending_click = None;
            self.profile = None;
            self.chat_profile = None;
            self.info_open = false;
            changed = true;
        }
        if let Some(scope) = self.pending_space.clone()
            && self.sidebar.set_space(Some(scope))
        {
            self.pending_space = None;
            changed = true;
        }
        changed
    }

    /// Restores the room open at the last run once the list is there; afterwards remembers the room on screen.
    fn restore_or_remember_room(&mut self) -> bool {
        let mut changed = false;
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
        changed
    }

    /// Queues the read marker for the room on screen once it is read.
    fn queue_read_marker(&mut self) {
        let (focused, at_bottom, public) = self.reading;
        if self.read_dirty
            && !self.unread_pending()
            && focused
            && at_bottom
            && self.op_pending.is_none()
            && self.account.is_some()
            && let Some(scope) = self.sidebar.selected().map(|r| r.scope.clone())
            && self.messages.is_some()
        {
            // A burst of messages sends one receipt: wait until it has been quiet for the debounce (the next frame is asked for).
            if self.read_after.is_some_and(|at| std::time::Instant::now() < at) {
                self.heights.watching = true;
                return;
            }
            self.read_dirty = false;
            self.read_after = None;
            // The tracker of an open with unread rows counts from its own read id: move it, or the badge lights up again next frame.
            self.unread_follow_read(&scope);
            self.op_pending = Some(OperationCommand::MarkRoom { scope, read: true, public, main_only: true });
        }
    }

    /// Queues the members read and the message, user and room searches that follow the UI state.
    fn queue_lookups(&mut self) {
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
        // The room list's field also searches the messages of every room (the engine ignores a term that is too short).
        let wanted = self.query.trim().to_owned();
        if self.op_pending.is_none() && wanted != self.all_sent {
            self.op_queue.retain(|op| !matches!(op, OperationCommand::MoreAllMessages));
            self.op_pending = Some(OperationCommand::SearchAllMessages { term: wanted.clone() });
            self.all_sent = wanted;
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
    }

    /// Queues the page and the read receipt of the open thread panel.
    fn queue_thread_requests(&mut self) {
        let (focused, _, public) = self.reading;
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
    }

    /// Queues the read of the addresses shown in the Info panel.
    fn queue_aliases_load(&mut self) {
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
    }

    /// Hands the next command to the engine; a full route keeps it for the next turn.
    fn send_next_command(&mut self) {
        if self.op_pending.is_none() && !self.op_queue.is_empty() {
            self.op_pending = Some(self.op_queue.remove(0));
        }
        for scope in self.ops.tick(std::time::Instant::now()) {
            self.history.activity(&scope);
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
    }

    /// Collects the results the engine has sent back.
    fn take_results(&mut self) -> Vec<OperationResult> {
        let mut results = Vec::new();
        if let Some((_, rx)) = &mut self.op_route {
            while let Ok(r) = rx.try_recv() {
                results.push(r);
            }
        }
        results
    }

    /// Tag, mode, leave and space results; `Some` (changed) when settled here.
    fn result_room_and_space(&mut self, result: &OperationResult) -> Option<bool> {
        match result {
            OperationResult::TagsEdited { result } => { self.sidebar.tag_done(result.clone()); self.room_error = self.sidebar.tag_error().map(str::to_owned); return Some(true); }
            OperationResult::NotificationModeSet { result, .. } => { self.sidebar.mode_done(result.clone()); self.room_error = self.sidebar.mode_error().map(str::to_owned); return Some(true); }
            OperationResult::RoomLeft { scope, result } => { self.sidebar.room_left(scope, result.clone()); self.room_error = self.sidebar.leave_error(scope).map(str::to_owned); return Some(true); }
            OperationResult::SpaceLeft { scope, result, .. } => { self.sidebar.space_left(scope, result.clone().map(drop)); self.room_error = self.sidebar.leave_error(scope).map(str::to_owned); return Some(true); }
            OperationResult::SpaceDeleted { scope, result } => {
                let name = self.sidebar.spaces().find(|r| &r.scope == scope).map_or_else(String::new, |r| r.name.clone());
                self.space_deleting = false;
                // Left for good: the space drops out of the rail; otherwise it stays and the report says why.
                if result.as_ref().is_ok_and(|d| matches!(d.left, Some(Ok(_)))) { self.sidebar.space_left(scope, Ok(())); }
                self.space_report = Some((name, result.clone()));
                self.space_facts_for(scope.clone());
                return Some(true);
            }
            OperationResult::SpaceChoices { scope, result } => {
                if let (Some((asked, slot)), Ok(choices)) = (&mut self.space_choices, result) && asked == scope { *slot = Some(choices.clone()); }
                return Some(true);
            }
            OperationResult::SpaceChildren { scope, result } => {
                if let Some(v) = &mut self.space_view && &v.scope == scope { v.children = Some(result.clone().map_err(|_| ())); }
                if let (Some((asked, slot)), Ok(c)) = (&mut self.space_facts, result) && asked == scope { *slot = Some(c.clone()); }
                return Some(true);
            }
            OperationResult::SpacePlaced { result, .. } => {
                self.space_placed = Some(*result);
                return Some(true);
            }
            OperationResult::SpaceChanged { result, .. } => {
                self.space_choices = None;
                // The open space window reads its rooms again.
                if let Some(v) = &self.space_view { self.op_queue.push(OperationCommand::SpaceChildren { scope: v.scope.clone() }); }
                if let Err(e) = result { self.room_error = Some(i18n::t(matrix_engine::operations::space_error_line(*e)).into()); }
                return Some(true);
            }
            OperationResult::RoomMarked { result: Err(_), .. } => { self.room_error = Some(i18n::t("Operation failed").into()); return Some(true); }
            _ => {}
        }
        None
    }

    /// Address results; `Some` (changed) when settled here.
    fn result_aliases(&mut self, result: &OperationResult) -> Option<bool> {
        let mut changed = false;
        if let OperationResult::Aliases { scope, result, aliases, .. } = result {
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
            return Some(changed);
        }
        None
    }

    /// Member rights, ban list, permissions and moderation results; `Some` (changed) when settled here.
    fn result_members_and_moderation(&mut self, result: &OperationResult) -> Option<bool> {
        let mut changed = false;
        match result {
            OperationResult::MemberRights { user_id, result, .. } => {
                if self.profile.as_ref().is_some_and(|(open, _)| open == user_id) {
                    self.rights = result.clone().ok();
                    changed = true;
                }
                return Some(changed);
            }
            OperationResult::BannedMembers { scope, result } => {
                if let Some(a) = &mut self.admin && &a.scope == scope {
                    a.banned = result.as_ref().map(|list| list.clone()).map_err(|_| ());
                    changed = true;
                }
                return Some(changed);
            }
            OperationResult::MemberPermissions { scope, result } => {
                if let Some(a) = &mut self.admin && &a.scope == scope {
                    a.perms = result.as_ref().ok().copied().unwrap_or_default();
                    changed = true;
                }
                return Some(changed);
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
                return Some(changed);
            }
            _ => {}
        }
        None
    }

    /// Room edit result; `Some` (changed) when settled here.
    fn result_room_edited(&mut self, result: &OperationResult) -> Option<bool> {
        if let OperationResult::RoomEdited { result } = result {
            match (result, &mut self.edit) {
                (Ok(()), _) => self.edit = None,
                (Err(_), Some(e)) => {
                    e.failed = true;
                    e.busy = false;
                }
                _ => {}
            }
            return Some(true);
        }
        None
    }

    /// Sticker results; `Some` (changed) when settled here.
    fn result_stickers(&mut self, result: &OperationResult) -> Option<bool> {
        match result {
            OperationResult::StickerPacks { scope, result } => {
                if self.draft_scope.as_ref() == Some(scope) { self.stickers.packs = Some(result.clone().map_err(|_| ())); }
                return Some(true);
            }
            OperationResult::StickerEdited { result } => {
                self.stickers.edited(result.is_ok());
                return Some(true);
            }
            OperationResult::StickerSent { result, .. } => {
                if result.is_err() { self.room_error = Some(i18n::t("Sticker not sent").into()); }
                return Some(true);
            }
            _ => {}
        }
        None
    }

    /// User and room search, creation, profile, ignore and join results; `Some` (changed) when settled here.
    fn result_searches_and_joins(&mut self, result: &OperationResult) -> Option<bool> {
        let mut changed = false;
        match result {
            OperationResult::UserSearch { state, .. } => {
                if accept_user_answer(&mut self.users_gate, state) {
                    self.users = state.clone();
                    changed = true;
                }
                return Some(changed);
            }
            OperationResult::RoomSearch { state } => {
                if accept_room_answer(state.term(), &self.explore.as_ref().map_or_else(|| self.new_term.clone(), |e| e.request().map(|r| r.0).unwrap_or_default())) {
                    self.public_rooms = state.clone();
                    changed = true;
                }
                return Some(changed);
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
                return Some(true);
            }
            OperationResult::RoomStarted { result } => {
                match result {
                    Ok(scope) => {
                        if std::mem::take(&mut self.feedback_pending) {
                            // A line the user already typed there is theirs: keep it.
                            let slot = if self.draft_scope.as_ref() == Some(scope) { &mut self.draft } else { self.drafts.entry(scope.clone()).or_default() };
                            if slot.is_empty() { *slot = matrix_desktop::feedback::diagnostic(); }
                        }
                        self.pending_select = Some(scope.clone());
                        self.new_open = false;
                        self.new_failed = false;
                    }
                    Err(e) if std::mem::take(&mut self.feedback_pending) => {
                        use matrix_engine::operations::OperationError as E;
                        self.room_error = Some(i18n::t(match e { E::Unavailable | E::WrongScope | E::InvalidInput => "Feedback address not found", e => matrix_engine::operations::space_error_line(*e) }).into());
                    }
                    Err(_) => self.new_failed = true,
                }
                return Some(true);
            }
            OperationResult::Profile { user_id, result } => {
                if let Some((open, slot)) = &mut self.profile
                    && open == user_id
                {
                    *slot = result.clone().ok();
                    changed = true;
                }
                return Some(changed);
            }
            OperationResult::Ignored { user_id, ignore, result: Ok(_) } => {
                if let Some((open, Some(p))) = &mut self.profile
                    && open == user_id
                {
                    p.ignored = *ignore;
                    changed = true;
                }
                return Some(changed);
            }
            OperationResult::Ignored { .. } => return Some(changed),
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
                return Some(true);
            }
            _ => {}
        }
        None
    }

    /// Applies one engine result to the UI state; `true` when something on screen changed.
    fn handle_result(&mut self, result: OperationResult) -> bool {
        if let Some(changed) = self.result_room_and_space(&result) {
            return changed;
        }
        if let Some(changed) = self.result_aliases(&result) {
            return changed;
        }
        if let Some(changed) = self.result_members_and_moderation(&result) {
            return changed;
        }
        if let Some(changed) = self.result_room_edited(&result) {
            return changed;
        }
        let mut changed = false;
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
            return changed;
        }
        if let OperationResult::ReadMarkers { scope, markers } = result {
            self.unread_markers(&scope, markers);
            changed = true;
            return changed;
        }
        if let OperationResult::ThreadRead { tag, root, target, result } = &result {
            if let Some(th) = &mut self.thread && th.root == *root && th.read_views.accepts(tag) {
                th.read_pending = false;
                if matches!(result, Ok(true)) { th.read_target = Some(target.clone()); }
            }
            return changed;
        }
        if let Some(changed) = self.result_stickers(&result) {
            return changed;
        }
        if matches!(result, OperationResult::ThreadSent { .. }) {
            if let Some(th) = &mut self.thread { th.next_read = std::time::Instant::now(); }
            return changed;
        }
        if let Some(changed) = self.result_searches_and_joins(&result) {
            return changed;
        }
        if let OperationResult::AllMessageSearch { state } = result {
            use matrix_engine::operations::MessageSearchState as S;
            let term = match &state {
                S::Idle => None,
                S::Searching { term } | S::Results { term, .. } | S::NoResults { term, .. } | S::Error { term } => Some(term.as_str()),
            };
            if term.is_none_or(|t| t == self.query.trim()) {
                self.all_search = state;
                changed = true;
            }
            return changed;
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
            return changed;
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
            return changed;
        }
        let done = matches!(&result, OperationResult::Applied { result: Ok(_), .. });
        if let OperationResult::Applied { request, .. } = &result
            && done
        {
            self.history.activity(&request.scope);
        }
        self.ops.apply(result) | done
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

    /// «Send feedback»: opens the DM with the feedback user (the SDK reuses or creates it), diagnostic line in its composer.
    pub fn send_feedback(&mut self) {
        self.room_error = None;
        self.feedback_pending = true;
        self.op_queue.push(OperationCommand::StartDm { user_id: matrix_desktop::feedback::target() });
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

    /// The next page of the messages under the room list's field.
    pub fn more_all_messages(&mut self) {
        use matrix_engine::operations::{MessageSearchState as S, MoreHits};
        if let S::Results { term, more: more @ (MoreHits::Available | MoreHits::Failed), .. } = &mut self.all_search
            && term == self.query.trim()
        {
            *more = MoreHits::Loading;
            self.op_queue.push(OperationCommand::MoreAllMessages);
        }
    }

    /// Opens the room of a message found by the room list's search on that message (as a reply's jump does).
    /// The room may be hidden by the field's own filter: it is opened anyway. False when the room is gone.
    pub fn open_message_hit(&mut self, hit: &matrix_engine::operations::MessageHit) -> bool {
        let Some(scope) = self.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == hit.room_id && !r.info.invited).map(|r| r.scope.clone()) else {
            return false;
        };
        let jump = (EventId::new(hit.thread_root.as_deref().unwrap_or(&hit.event_id)), hit.thread_root.is_some());
        if self.sidebar.selected().is_some_and(|r| r.scope == scope) {
            self.search_jump = Some(jump);
        } else {
            self.jump_after_open = Some((scope.clone(), jump));
            self.sidebar.select_outside_view(&scope);
        }
        self.notice = None;
        true
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
        self.thread = Some(ThreadState { root, views: Default::default(), history: None, at_start: false, heights: Default::default(), failed: false, draft: String::new(), attachment: None, attachment_revision: 0, voice: None, reply_to: None, menu: None, want: 50, growing: None, stalled: false, tail_visible: false, read_views: Default::default(), read_target: None, read_pending: false, read_public: None, read_at: None, next_read: std::time::Instant::now() });
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

    /// A sender's name or picture in a message was clicked: the member card opens alone and replaces the info panel while it is on screen.
    pub fn open_profile_from_chat(&mut self, user_id: String) {
        (self.chat_profile, self.info_open) = open_from_chat(self.chat_profile, self.info_open);
        self.open_profile(user_id);
    }

    /// Gives the info panel back once the chat card is closed; drops the chat card when the user opened the panel, search or a thread over it.
    pub fn settle_chat_profile(&mut self) {
        let other = self.search_open || self.thread.is_some();
        (self.chat_profile, self.profile, self.info_open) = settle_chat_card(self.chat_profile, self.profile.take(), self.info_open, other);
    }

    /// The member card is on screen: from the info panel, or alone from a message.
    pub fn card_open(&self) -> bool {
        self.profile.is_some() && (self.info_open || self.chat_profile.is_some())
    }

    /// The card's picture, big (falls back to the small one while the big one comes).
    pub fn profile_picture(&self, mxc: Option<&str>) -> crate::avatar::Avatar {
        let big = mxc.map(matrix_engine::avatar::large_key);
        match big.as_deref().map(|k| crate::avatar::lookup(&self.avatars, Some(k))) {
            Some(Avatar::Ready(t)) => Avatar::Ready(t),
            _ => crate::avatar::lookup(&self.avatars, mxc),
        }
    }

    /// Opens the picture full size in the viewer.
    pub fn open_avatar_viewer(&mut self, mxc: &str, name: String) {
        let Avatar::Ready(tex) = self.profile_picture(Some(mxc)) else { return };
        self.avatar_full = self.avatar_original(mxc);
        self.viewer = Some(crate::viewer::Viewer::for_avatar(name, tex));
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
            crate::viewer::Action::Close => { self.media.cancel_original(); self.avatar_full = None; self.viewer = None; }
            crate::viewer::Action::Save(path) => {
                if path.exists() {
                    self.notice = Some(i18n::t("Choose a file that does not exist yet").to_owned());
                    return;
                }
                if let Some(scope) = self.draft_scope.clone() {
                    let queued = self.media.request_save(scope, viewer.id.clone(), path.clone());
                    if queued { self.share_after = path; }
                    self.notice = (!queued).then(|| i18n::t("A save is already in progress").to_owned());
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
        let queued = self.media.request_save(scope, event, path.clone());
        if queued { self.share_after = path; }
        self.notice = (!queued).then(|| i18n::t("A save is already in progress").to_owned());
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

    /// A pasted picture goes out as a PNG; a pasted GIF as its original bytes, so it stays animated.
    pub fn attach_image(&mut self, pasted: crate::composer::Pasted, thread: bool) {
        use crate::composer::Pasted;
        let occupied = if thread { self.thread.as_ref().is_some_and(|th| th.attachment.is_some()) } else { self.attachment.is_some() };
        if occupied {
            self.notice = Some(i18n::t("Only one attachment is supported").to_owned());
            return;
        }
        let limit = self.upload_limit.unwrap_or(50 << 20);
        match pasted {
            Pasted::Picture(image) => match matrix_desktop::composer_image::prepare(&image, limit) {
                Ok(data) => self.set_attachment(thread, i18n::t("Image.png").to_owned(), data, None),
                Err(_) => self.notice = Some(i18n::t("Image cannot be attached").to_owned()),
            },
            Pasted::Gif(data) if data.len() as u64 <= limit => self.set_attachment(thread, "Image.gif".to_owned(), data, None),
            Pasted::Gif(_) => self.notice = Some(i18n::t("Image cannot be attached").to_owned()),
        }
    }

    /// The pending attachment of the room's composer or of the thread's.
    fn set_attachment(&mut self, thread: bool, name: String, data: Vec<u8>, voice: Option<matrix_engine::sending::Voice>) {
        static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let revision = REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !thread {
            (self.attachment, self.voice) = (Some((name, data)), voice);
            self.attachment_revision = revision;
        } else if let Some(th) = &mut self.thread {
            (th.attachment, th.voice) = (Some((name, data)), voice);
            th.attachment_revision = revision;
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

    /// The menu chose `key` on the selected message: adds it, or takes ours back when the row already shows it.
    pub fn react(&mut self, key: &str) {
        let mine = self.reacted(key);
        self.ops.toggle_reaction(key, mine, matrix_engine::sending::fresh_transaction_id());
        self.close_menu_keep_selection();
    }

    /// The reactions of the selected message as drawn: the server's with the taps on their way.
    pub fn selected_reactions(&self) -> Vec<matrix_engine::history::Reaction> {
        #[cfg(debug_assertions)]
        if let Some(fake) = crate::reactions::fixture() {
            return fake;
        }
        let Some(row) = crate::touch_menu::selected(self) else { return Vec::new() };
        match &row.id.id {
            TimelineId::Server(id) => matrix_desktop::overlay_reactions(self.ops.wishes(), id, &row.reactions),
            _ => row.reactions.clone(),
        }
    }

    /// The selected message carries our reaction `key`.
    pub fn reacted(&self, key: &str) -> bool {
        self.selected_reactions().iter().any(|r| r.key == key && r.mine)
    }

    /// What the pills asked for in this frame: a tap adds or takes back ours (shown at once, rolled back if the send fails), a long
    /// press or right click opens the list of who reacted.
    pub fn reaction_acts(&mut self, acts: crate::reactions::Acts, ctx: &eframe::egui::Context, t: &matrix_desktop::DesignTokens) {
        if let (Some((target, key, mine)), Some(scope)) = (acts.toggle, self.draft_scope.clone()) {
            self.ops.react_direct(scope, target, &key, mine, matrix_engine::sending::fresh_transaction_id());
        }
        let fresh = acts.who.is_some();
        if acts.who.is_some() {
            self.who = acts.who;
        }
        if let Some((at, r)) = &self.who && !crate::reactions::popup(ctx, *at, r, fresh, t) {
            self.who = None;
        }
        if !self.ops.wishes().is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
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
        !self.reconnecting && recovery_open(self.form.recovery())
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
    pub fn start_recording(&mut self, thread: bool, ctx: &egui::Context) {
        if self.recorder.is_some() { return; }
        match matrix_desktop::voice_recorder::Recorder::start(self.upload_limit.unwrap_or(50 << 20), { let ctx = ctx.clone(); move || ctx.request_repaint() }) {
            Ok(recorder) => {
                self.record_thread = thread;
                self.recorder = Some(recorder);
                self.notice = None;
            }
            // First tap: the permission prompt is up; poll() restarts after «Consenti».
            Err(matrix_desktop::voice_recorder::PERMISSION_PENDING) => self.mic_pending = Some(thread),
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

    /// (note, playing, progress, waveform) of the loaded note.
    pub fn audio_now(&self) -> Option<(EventId, bool, f32, Vec<f32>)> {
        let (id, out) = self.audio.as_ref()?;
        let p = out.player();
        let p = p.lock().unwrap_or_else(|e| e.into_inner());
        Some((id.clone(), p.is_playing(), p.progress(), self.audio_wave.clone()))
    }

    pub fn status(&self) -> FormStatus {
        let status = self.form.status();
        if self.reconnecting && !matches!(status, FormStatus::Failed(_) | FormStatus::Prompt(matrix_engine::signin::SignInPrompt { error: Some(_), .. })) {
            if let Some(account) = self.account { return FormStatus::Authenticated(account); }
        }
        status
    }
}

fn recovery_open(state: Option<RecoveryState>) -> bool {
    matches!(state, Some(RecoveryState::Offered | RecoveryState::Pending | RecoveryState::Failed(_) | RecoveryState::Unsupported))
}

/// A click on a sender: the card stands alone and the info panel goes off, remembered only by the first click.
pub fn open_from_chat(chat: Option<bool>, info: bool) -> (Option<bool>, bool) {
    (Some(chat.unwrap_or(info)), false)
}

/// State of a card opened from a message: (chat card, card, info panel) after one settle.
fn settle_chat_card<T>(chat: Option<bool>, card: Option<T>, info: bool, other_panel: bool) -> (Option<bool>, Option<T>, bool) {
    match chat {
        None => (None, card, info),
        Some(_) if other_panel => (None, None, info),
        Some(_) if info => (None, card, info), // the user opened the info panel: the card now belongs to it
        Some(was) if card.is_none() => (None, None, was),
        Some(_) => (chat, card, info),
    }
}

#[test]
fn a_chat_card_never_opens_the_info_panel() {
    assert_eq!(open_from_chat(None, false), (Some(false), false));
    assert_eq!(open_from_chat(None, true), (Some(true), false), "replaces the open panel");
    assert_eq!(open_from_chat(Some(true), false), (Some(true), false), "a second click keeps what the first remembered");
}

#[test]
fn chat_card_gives_the_info_panel_back() {
    assert_eq!(settle_chat_card(Some(false), Some(1), false, false), (Some(false), Some(1), false), "card on screen, alone");
    assert_eq!(settle_chat_card(Some(false), None::<u8>, false, false), (None, None, false), "closed: no panel as before");
    assert_eq!(settle_chat_card(Some(true), None::<u8>, false, false), (None, None, true), "closed: info panel back");
    assert_eq!(settle_chat_card(Some(true), Some(1), true, false), (None, Some(1), true), "panel opened over the card: the card is the panel's");
    assert_eq!(settle_chat_card(Some(true), Some(1), false, true), (None, None, false), "search or thread opened: the card goes");
    assert_eq!(settle_chat_card(None, None::<u8>, true, false), (None, None, true), "info panel opened by the user stays");
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

mod unread;

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
