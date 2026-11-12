#![recursion_limit = "256"]
// Release on Windows: GUI subsystem, no console window (--help output is lost there).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod account_remove;
mod account_switch;
mod avatar_view;
mod compact_view;
mod composer_icons;
mod composer_view;
mod editing_command;
mod emoji_font;
mod directory_servers_view;
// Unused until the filter row card wires it in.
#[allow(dead_code)]
mod filter_view;
mod groups_edit_view;
mod invite_view;
mod link_view;
mod list_divider;
#[cfg(test)]
mod list_divider_test;
/// URLs the system handed to the app (macOS open-URL) before the shell could take them.
static OPENED_URLS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
#[cfg(target_os = "linux")]
mod install_offer_view;
mod login_field;
mod media_view;
mod poll_dialog;
mod poll_view;
mod about_view;
mod members_view;
mod input_modality;
mod menu_view;
mod name_line;
#[cfg(test)]
mod navigation_resize_test;
mod new_room_view;
mod quick_search_view;
mod search_list_view;
mod room_edit_view;
mod notification_view;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
mod operation_panel_view;
mod panel_opener_view;
mod rail_view;
mod ring_seam;
mod room_details_view;
mod recover_keys_view;
mod room_menu_view;
mod room_search_view;
mod scrollbar_view;
mod settings_view;
mod switch_row;
#[cfg(test)]
mod switch_rows_test;
#[cfg(any(test, feature = "gui-fixture"))]
mod rail_drag_test;
#[cfg(test)]
mod verification_popup_test;
#[cfg(test)]
mod member_profile_test;
#[cfg(test)]
mod link_jump_test;
#[cfg(test)]
mod drawer_active_test;
#[cfg(test)]
mod rail_ring_test;
#[cfg(test)]
mod details_panel_test;
#[cfg(test)]
mod dialog_typing_test;
#[cfg(test)]
mod room_topic_test;
#[cfg(test)]
mod quick_answer_test;
#[cfg(test)]
mod ink_test;
#[cfg(test)]
mod sidebar_navigation_test;
#[cfg(feature = "live")]
mod signin_view;
mod spaces_view;
mod picker_view;
mod thread_view;
mod timeline_view;
mod tray_view;
mod verification_view;
mod video_player;
mod audio_player;
mod voice_recorder;
use composer_view::{ComposerView, OpenPoll, Submitted};

use gpui::{
    App, Application, Bounds, ClipboardItem, Context, FocusHandle, KeyDownEvent, ObjectFit, RenderImage,
    ScrollStrategy,
    UniformListScrollHandle, Window, WindowBounds, WindowOptions, div, point, prelude::*, px, rgb,
    size, uniform_list,
};
use matrix_desktop::{
    DEMO_NOTICE, HistorySession, HistoryStatus, LIGHT, Navigation, Sidebar, Theme,
    opens_message_menu,
};
use matrix_domain::{AccountId, RoomScope};
use matrix_engine::{RoomListEvent, RoomSyncStatus};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use timeline_view::TimelineView;

// GPUI owns projections only. Optional live restore and sync run on a worker.
struct Shell {
    #[cfg(feature = "live")]
    signin: Option<gpui::Entity<signin_view::SignInView>>,
    rail_ui: rail_view::RailUi,
    opener_ui: panel_opener_view::OpenerUi,
    room_menu: Option<(RoomScope, gpui::Point<gpui::Pixels>)>,
    /// Clicks on the header magnifier («apri ricerca stanza»); read by the room-search task.
    room_search_requests: u32,
    /// The header field and results of the room message search, while open.
    room_search: Option<room_search_view::RoomSearch>,
    /// «Esci dalla stanza» was clicked once; the second click leaves.
    confirm_leave: Option<RoomScope>,
    menu_open: bool,
    menu_focus: FocusHandle,
    drawer_focus: FocusHandle,
    settings_open: bool,
    settings_error: Option<&'static str>,
    about_open: bool,
    about_row: usize,
    about_focus: FocusHandle,
    about_system: String,
    about_icon: Option<std::sync::Arc<RenderImage>>,
    #[cfg(target_os = "linux")]
    install_offer: matrix_desktop::install_offer::State,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_offer_bounds: [Option<accesskit::Rect>; 4],
    new_room: Option<new_room_view::NewRoom>,
    poll_dialog: Option<gpui::Entity<poll_dialog::PollDialog>>,
    quick: Option<quick_search_view::QuickSearch>,
    recents: matrix_desktop::quick_search::RecentsStore,
    room_edit: Option<room_edit_view::RoomEdit>,
    /// A just-created room, opened as soon as the sync lists it.
    pending_open: Option<RoomScope>,
    restore: matrix_desktop::last_room_restore::LastRoomRestore,
    groups_edit: Option<groups_edit_view::GroupsEdit>,
    /// Set when the new-room dialog closed from an engine result: the next render restores focus.
    refocus_menu: bool,
    refocus_list: bool,
    /// The «Unisciti» server-name field went away: the next render puts the focus back on the search field.
    refocus_join_field: bool,
    drawer_row: usize,
    /// Last «Aggiorna ora» failure, one line, shown in place of the update row.
    update_error: Option<String>,
    /// `confirm_update` runs once, at the first frame.
    update_confirmed: bool,
    settings_focus: FocusHandle,
    /// Switch the keyboard sits on in Settings: 0 debug, 1 hide deleted messages.
    settings_row: usize,
    prefs: matrix_desktop::Preferences,
    prefs_dirs: Option<matrix_desktop::AppDirs>,
    /// Divider drag in progress: pointer x and list width at mouse down.
    list_drag: Option<(f32, f32)>,
    list_divider_focus: FocusHandle,
    room_prefs: matrix_desktop::RoomPrefs,
    /// Composer focus as of the last frame (saved with the room on change/close, not per event).
    composer_focused: bool,
    /// The saved room was just reopened and its composer had focus: take it on the next frame.
    focus_composer_on_restore: bool,
    /// Catalog as last read; reloaded whenever the drawer opens or this window signs in.
    accounts: matrix_desktop::AccountCatalog,
    /// Profile folder and trust root this window was started with.
    account_root: Option<(std::path::PathBuf, Option<std::path::PathBuf>)>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    back_ui: compact_view::BackUi,
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    back_focus: FocusHandle,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    operation_ui: operation_panel_view::OperationUi,
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    actions_focus: FocusHandle,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    accessibility: matrix_desktop::accessibility::LinuxAccessibility,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_search_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_history_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    history_focus: FocusHandle,
    pending_sidebar_key: Option<filter_view::SidebarPress>,
    notification_focus: matrix_desktop::notifications::NotificationControl,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_notification_bounds: [Option<accesskit::Rect>; 4],
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_rooms: matrix_desktop::accessibility::RoomNodes,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_history: matrix_desktop::accessibility::HistoryNodes,
    room_details_open: bool,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    details_focus: matrix_desktop::accessibility::details::DetailsControl,
    /// Focus zone of the non-modal «Informazioni stanza» panel; the composer keeps the keyboard until F6 or a click moves it here.
    details_zone: FocusHandle,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_details_bounds: [Option<accesskit::Rect>; 8],
    spaces_ui: spaces_view::SpacesUi,
    spaces_open: bool,
    navigation_wide: bool,
    notifications: matrix_domain::notifications::NotificationPolicy,
    tray: Option<matrix_desktop::tray::Tray>,
    /// Not minimized to the tray; the menu entry reads «Riduci» or «Mostra».
    window_visible: bool,
    notification_handle: Option<matrix_desktop::notifications::NotificationHandle>,
    notifications_open: bool,
    notification_failed: bool,
    avatars: avatar_view::Avatars,
    media: matrix_desktop::LiveMedia,
    /// The in-app player of one voice note: its message and its cpal output. Dropped to stop it.
    audio: Option<(matrix_domain::EventId, crate::audio_player::AudioOutput)>,
    /// Channels to the shown account's worker.
    routes: matrix_desktop::AccountRoutes,
    /// Accounts running in the background, by catalog ID; only the shown one has its state in this struct.
    parked: HashMap<AccountId, matrix_desktop::AccountState>,
    /// Ends the shown account's worker.
    control: Option<matrix_engine::WorkerControl>,
    /// Account the drawer asks to remove, until confirmed or cancelled.
    confirm_removal: Option<AccountId>,
    /// Why the last click on this drawer account row could not show it.
    switch_error: Option<(AccountId, &'static str)>,
    swallow_menu_key_up: bool,
    /// The system link waiting for the accounts to be ready (memory only, see `link_open`).
    pending_link: matrix_desktop::link_open::LinkQueue,
    /// One line about a link that could not be opened; cleared after a few seconds.
    link_notice: Option<(&'static str, std::time::Instant)>,
    link_jump: Option<link_view::LinkJump>,
    removals: Vec<account_remove::Removal>,
    /// Catalog ID of the shown account (unknown until a new sign-in is recorded).
    active_id: Option<AccountId>,
    verification: matrix_desktop::DeviceVerification,
    verification_open: bool,
    delete_field: Option<gpui::Entity<login_field::LoginField>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    device_focus: matrix_desktop::accessibility::devices::DeviceControl,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_device_bounds: [Option<accesskit::Rect>; 2],
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    device_scroll: verification_view::DeviceScroll,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_device_nodes: std::cell::RefCell<matrix_desktop::accessibility::devices::DeviceNodes>,
    delivery: std::rc::Rc<std::cell::RefCell<matrix_desktop::LiveDelivery>>,
    operations: std::rc::Rc<std::cell::RefCell<matrix_desktop::LiveOperations>>,
    pending_operation_command: Option<matrix_engine::operations::OperationCommand>,
    /// Room whose members were asked for the @ completion.
    members_requested: Option<RoomScope>,
    /// Joined members of the shown room (room sheet) and the profile opened from them.
    room_members: Vec<matrix_engine::operations::Member>,
    profile: Option<members_view::ProfileState>,
    /// Focused control of the opened profile (index in Tab order) and the action whose copy just happened.
    profile_focus: usize,
    /// Scroll of the opened profile (short windows); follows the keyboard focus.
    profile_scroll: gpui::ScrollHandle,
    profile_copied: Option<matrix_desktop::ProfileAct>,
    /// «Invia messaggio» of a profile is waiting for the engine.
    profile_dm: bool,
    /// The action of the profile that just failed (verify, moderation); shown for a moment, like the copy confirmation.
    profile_failed: Option<matrix_desktop::ProfileAct>,
    /// What the room lets the current user do to the profile's user (id it was asked for, rights).
    profile_rights: Option<(String, matrix_engine::operations::MemberRights)>,
    /// Destructive moderation act waiting for its confirmation.
    profile_confirm: Option<matrix_desktop::ProfileAct>,
    /// The open room may hold unread messages in view; cleared once the read marker is queued.
    read_dirty: bool,
    confirm_redaction: Option<matrix_domain::ScopedTimelineId>,
    /// The pending confirmation closes the poll instead of deleting the message.
    confirm_end: bool,
    connected: bool,
    status: RoomSyncStatus,
    /// Start of the current outage, for the list's disconnection icon.
    outage: matrix_desktop::disconnect::Gate,
    sidebar: Sidebar,
    active_scope: RoomScope,
    composer: gpui::Entity<ComposerView>,
    composer_visit: std::rc::Rc<composer_view::SendVisit>,
    timeline: Option<TimelineView>,
    thread: Option<thread_view::ThreadPanel>,
    /// The panel closed last: reopening that thread shows its replies at once, then refreshes.
    thread_last: Option<thread_view::ThreadPanel>,
    /// A room change parked this thread's panel: reopen it when that room is active again.
    thread_resume: Option<(RoomScope, matrix_domain::EventId)>,
    saved_timelines: HashMap<RoomScope, TimelineView>,
    history: HistorySession,
    pending_history: Option<matrix_engine::history::HistoryRequest>,
    compact_chat: bool,
    theme: Theme,
    system_dark: bool,
    search_focus: FocusHandle,
    list_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    room_scrollbar: scrollbar_view::Scrollbar,
    members_scroll: UniformListScrollHandle,
    members_scrollbar: scrollbar_view::Scrollbar,
    topic_scroll: UniformListScrollHandle, // only its base ScrollHandle is used, to share the scrollbar
    topic_scrollbar: scrollbar_view::Scrollbar,
    // Links of the «Argomento» text: their own registry and confirmation, never the timeline's.
    topic_links: Rc<RefCell<matrix_desktop::MessageLinkRequests>>,
    topic_confirm: Rc<RefCell<matrix_desktop::LinkConfirmation>>,
    topic_pressed: Rc<Cell<Option<u64>>>,
    topic_hover: Rc<Cell<Option<u64>>>,
}

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn drain_navigation_actions(this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        for _ in 0..8 {
            let Some(action) = this.accessibility.take_action() else {
                break;
            };
            input_modality::set_keyboard(cx);
            #[cfg(feature = "live")]
            if this.dispatch_signin_action(&action, window, cx) {
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::CompactBack(action) = action {
                this.apply_semantic_back(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::OperationPanel(action) = action
            {
                this.apply_semantic_operation_panel(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Spaces(action) = action {
                this.apply_semantic_spaces(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::About(action) = action {
                this.apply_semantic_about(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Drawer(action) = action {
                this.apply_semantic_drawer(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Devices(action) = action {
                this.apply_semantic_devices(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Details(action) = action {
                this.apply_semantic_details(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::RoomSearch(action) = action {
                this.apply_semantic_room_search(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::NewRoom(action) = action {
                this.apply_semantic_new_room(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::NewChat(action) = action {
                this.apply_semantic_new_chat(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Notifications(action) = action {
                this.apply_semantic_notification(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::ChooseIcon(icon) = action {
                if this.settings_open {
                    this.choose_icon(icon, window, cx);
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Settings(action) = action {
                this.apply_semantic_settings(action, window, cx);
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::InstallOffer(choice) = action {
                this.choose_install(choice, cx);
                continue;
            }
            if this.notifications_open || this.verification_open {
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::LoadServerHistory(tag) = &action
            {
                if this.connected
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && let Some(request) = this.history.load_older_if_current(tag)
                {
                    this.pending_history = Some(request);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::FocusServerHistory(tag) =
                &action
            {
                if this.connected
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && !tag.is_cancelled()
                    && this.history.pagination_tag().is_some_and(|current| {
                        current.generation() == tag.generation() && current.scope() == tag.scope()
                    })
                {
                    this.history_focus.focus(window);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::LinkConfirmation(intent) =
                &action
            {
                if let Some(event) = intent.request.source().event()
                    && event.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == event.scope)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_confirmation(intent, window, cx);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::OfflineDelivery(intent) =
                &action
            {
                if !this.connected
                    && intent.target.event().scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == intent.target.event().scope)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_offline_delivery(intent, window);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::PlayVideo(id) = &action {
                if id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.play_video(id, window);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::OpenProfile(user_id) = &action {
                if (window.viewport_size().width >= px(800.) || this.compact_chat) && this.timeline.is_some() {
                    this.pending_profile(user_id.clone());
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::FocusPill(id, start) = &action {
                if id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_pill(id, *start, window);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::RecoverKeys(id) = &action {
                if this.connected
                    && id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this.timeline.as_ref().is_some_and(|timeline| {
                        timeline
                            .semantic_snapshot(&mut this.a11y_history, window)
                            .allows_recover_keys(id)
                            && timeline.focus_semantic_recover(id, window, cx)
                    })
                {
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::SaveAttachment(id) = &action {
                if id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.save_attachment(id);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::CopyEventId(intent) = &action {
                if intent.target.event().scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == intent.target.event().scope)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_event_id_copy(intent, window, cx);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::MessageLink(intent) = &action {
                if let Some(event) = intent.target.event()
                    && event.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == event.scope)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_link(intent, window);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::MessageSelection(intent) =
                &action
            {
                if intent.target.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == intent.target.scope)
                    && let Some(timeline) = &this.timeline
                {
                    timeline.apply_selection(intent, window, cx);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::FocusMessageRetry(id) = &action
            {
                if this.connected
                    && id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == id.scope)
                    && this.timeline.as_ref().is_some_and(|timeline| {
                        timeline
                            .semantic_snapshot(&mut this.a11y_history, window)
                            .allows_retry(id)
                            && timeline.focus_semantic_retry(id, window)
                    })
                {
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::OpenMessageActions(id)
            | matrix_desktop::accessibility::NavigationAction::FocusMessageActions(id) = &action
            {
                if this.connected
                    && id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == id.scope)
                    && this.timeline.as_ref().is_some_and(|timeline| {
                        timeline
                            .semantic_snapshot(&mut this.a11y_history, window)
                            .allows_actions(id)
                    })
                {
                    if matches!(
                        action,
                        matrix_desktop::accessibility::NavigationAction::OpenMessageActions(_)
                    ) {
                        this.operations.borrow_mut().select(id.clone());
                    } else if let Some(timeline) = &this.timeline {
                        timeline.focus_semantic_actions(id, window);
                    }
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::RetryMessage(id) = &action {
                if this.connected
                    && id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .sidebar
                        .selected()
                        .is_some_and(|room| room.scope == id.scope)
                    && this.timeline.as_ref().is_some_and(|timeline| {
                        timeline
                            .semantic_snapshot(&mut this.a11y_history, window)
                            .allows_retry(id)
                    })
                    && let matrix_domain::TimelineId::Pending(transaction) = &id.id
                    && this.delivery.borrow_mut().retry(&id.scope, transaction)
                {
                    this.refresh_delivery(false);
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::FocusPollOption(id, ix)
            | matrix_desktop::accessibility::NavigationAction::VotePoll(id, ix) = &action
            {
                if this.connected
                    && id.scope == this.active_scope
                    && (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this.sidebar.selected().is_some_and(|room| room.scope == id.scope)
                {
                    if matches!(action, matrix_desktop::accessibility::NavigationAction::VotePoll(..)) {
                        this.vote_poll(id, *ix);
                    } else if let Some(timeline) = &this.timeline {
                        timeline.focus_semantic_poll(id, *ix, window);
                    }
                    cx.notify();
                }
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::FocusMessage(id) = &action {
                if (window.viewport_size().width >= px(800.) || this.compact_chat)
                    && this
                        .timeline
                        .as_ref()
                        .is_some_and(|timeline| timeline.focus_semantic_message(id, window))
                {
                    cx.notify();
                    let entity = cx.entity().clone();
                    window.on_next_frame(move |window, _| {
                        window.on_next_frame(move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                    });
                }
                continue;
            }
            if this.dispatch_opener_action(&action, window, cx) {
                continue;
            }
            if this.dispatch_composer_action(&action, window, cx) {
                continue;
            }
            if let matrix_desktop::accessibility::NavigationAction::Scrollbar(action) = action {
                this.apply_scrollbar(action, window, cx);
                cx.notify();
                continue;
            }
            if window.viewport_size().width < px(800.) && this.compact_chat {
                continue;
            }
            match action {
                matrix_desktop::accessibility::NavigationAction::PanelOpener(_) => unreachable!(),
                #[cfg(feature = "live")]
                matrix_desktop::accessibility::NavigationAction::SignIn(_) => (),
                matrix_desktop::accessibility::NavigationAction::CopyEventId(_)
                | matrix_desktop::accessibility::NavigationAction::InstallOffer(_)
                | matrix_desktop::accessibility::NavigationAction::PlayVideo(_)
                | matrix_desktop::accessibility::NavigationAction::SaveAttachment(_)
                | matrix_desktop::accessibility::NavigationAction::OpenProfile(_)
                | matrix_desktop::accessibility::NavigationAction::RecoverKeys(_)
                | matrix_desktop::accessibility::NavigationAction::OfflineDelivery(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::MessageSelection(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::CompactBack(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::MessageLink(_)
                | matrix_desktop::accessibility::NavigationAction::LinkConfirmation(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::Scrollbar(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::OperationPanel(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::Spaces(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::Devices(_)
                | matrix_desktop::accessibility::NavigationAction::Drawer(_)
                | matrix_desktop::accessibility::NavigationAction::About(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::LoadServerHistory(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::FocusServerHistory(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::NewChat(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::RoomSearch(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::NewRoom(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::Notifications(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::ChooseIcon(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::Settings(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::Details(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::FocusComposer
                | matrix_desktop::accessibility::NavigationAction::FocusSend { .. }
                | matrix_desktop::accessibility::NavigationAction::SubmitComposer { .. } => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::FocusMessage(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::FocusMessageRetry(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::RetryMessage(_) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::FocusPill(..)
                | matrix_desktop::accessibility::NavigationAction::FocusPollOption(..)
                | matrix_desktop::accessibility::NavigationAction::VotePoll(..) => unreachable!(),
                matrix_desktop::accessibility::NavigationAction::FocusMessageActions(_)
                | matrix_desktop::accessibility::NavigationAction::OpenMessageActions(_) => {
                    unreachable!()
                }
                matrix_desktop::accessibility::NavigationAction::ToggleMenu => {
                    this.set_menu_open(!this.menu_open, window, cx)
                }
                matrix_desktop::accessibility::NavigationAction::FocusMenu => {
                    this.menu_focus.focus(window);
                    cx.notify();
                }
                matrix_desktop::accessibility::NavigationAction::FocusSearch => {
                    this.search_focus.focus(window);
                    cx.notify();
                }
                matrix_desktop::accessibility::NavigationAction::FocusRoom(intent) => {
                    this.apply_semantic_room(intent, true, window, cx)
                }
                matrix_desktop::accessibility::NavigationAction::ActivateRoom(intent) => {
                    this.apply_semantic_room(intent, false, window, cx)
                }
                matrix_desktop::accessibility::NavigationAction::AnswerInvite { scope, accept } => {
                    this.answer_invite(scope, accept, cx)
                }
                matrix_desktop::accessibility::NavigationAction::LeaveRoom(scope) => {
                    // Only while that row's menu is open, as the pointer path.
                    if this.room_menu.as_ref().is_some_and(|(open, _)| *open == scope) {
                        this.leave_room(scope, cx)
                    }
                }
            }
        }
    }

    #[cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
    fn dispatch_signin_action(
        &self,
        action: &matrix_desktop::accessibility::NavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use matrix_desktop::accessibility::NavigationAction;
        if let Some(signin) = &self.signin {
            if let NavigationAction::SignIn(intent) = action {
                signin.update(cx, |view, cx| {
                    view.apply_semantic(*intent, window, cx);
                });
            }
            // Consume queued chat actions while sign-in covers the chat.
            return true;
        }
        matches!(action, NavigationAction::SignIn(_))
    }
    fn search_observer(&self, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = _cx.entity();
            Some(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = f64::from(window.scale_factor());
                        let rect = accesskit::Rect::new(
                            f64::from(f32::from(bounds.left())) * scale,
                            f64::from(f32::from(bounds.top())) * scale,
                            f64::from(f32::from(bounds.right())) * scale,
                            f64::from(f32::from(bounds.bottom())) * scale,
                        );
                        if entity.read(cx).a11y_search_bounds != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_search_bounds = Some(rect);
                                    cx.notify();
                                });
                            });
                        }
                    },
                )
                .absolute()
                .size_full()
                .top_0()
                .left_0()
                .into_any_element(),
            )
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn dispatch_composer_action(
        &mut self,
        action: &matrix_desktop::accessibility::NavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use matrix_desktop::accessibility::NavigationAction as A;
        if !matches!(
            action,
            A::FocusComposer | A::FocusSend { .. } | A::SubmitComposer { .. }
        ) {
            return false;
        }
        if self.composer_visit.source().is_none() {
            return true;
        }
        match action {
            A::FocusSend {
                scope,
                text,
                source,
            }
            | A::SubmitComposer {
                scope,
                text,
                source,
            } => {
                let current = self
                    .composer
                    .read(cx)
                    .semantic_snapshot(self.active_scope.clone(), window);
                if current.matches_submission(scope, text, *source) {
                    self.composer.update(cx, |editor, cx| {
                        if matches!(action, A::FocusSend { .. }) {
                            editor.focus_send(window, cx);
                        } else {
                            editor.submit_from_accessibility(window, cx);
                        }
                    });
                }
            }
            A::FocusComposer => self
                .composer
                .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx)),
            _ => unreachable!(),
        }
        cx.notify();
        true
    }

    #[cfg(test)]
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.set_theme(
            if self.theme.is_dark() {
                Theme::Classic
            } else {
                Theme::Dark
            },
            cx,
        );
    }

    /// Re-evaluates the saved theme and «notte automatica» (clock or OS appearance).
    fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let theme = self.prefs.effective_theme(self.system_dark, matrix_desktop::local_hour());
        if theme != self.theme {
            self.set_theme(theme, cx);
        }
    }

    fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        self.composer
            .update(cx, |editor, cx| editor.set_theme(theme.tokens(), cx));
        if let Some(c) = self.thread.as_ref().and_then(|p| p.composer.clone()) {
            c.update(cx, |editor, cx| editor.set_theme(theme.tokens(), cx));
        }
        cx.notify();
    }

    fn navigation_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The verification popup is modal over every screen: it gets all keys.
        if self.verification.popup_open() {
            self.popup_key(event, cx);
            return;
        }
        let key = &event.keystroke;
        // Device sign-out and rename dialogs are modal: their field and buttons get the keys.
        if self.verification_open
            && (self.verification.delete_dialog().is_some() || self.verification.rename_dialog().is_some())
        {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                use matrix_desktop::accessibility::devices::{DeviceAction, DeviceControl, DeviceView};
                let view = self.semantic_devices(window);
                let DeviceView::Open(panel) = &view else { return };
                // Enter in the field is the dialog's own submit; buttons get Enter/Space here.
                let action = match key.key.as_str() {
                    "tab" => Some(DeviceAction::Focus {
                        account: panel.account,
                        control: panel.next_control(key.modifiers.shift),
                    }),
                    "escape" => Some(DeviceAction::CancelDelete(panel.account)),
                    "enter" | "space" if panel.focus != DeviceControl::DialogField => view.action(&accesskit::ActionRequest {
                        action: accesskit::Action::Click,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: panel.control_id(panel.focus),
                        data: None,
                    }),
                    _ => return,
                };
                if let Some(action) = action {
                    self.apply_semantic_devices(action, window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        if self.quick.is_some() || self.room_search_typing(&key.modifiers, window, cx) {
            return;
        }
        if let Some(timeline) = &self.timeline
            && timeline.viewer_key(&key.key, window)
        {
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.connected && !self.search_focus.is_focused(window) {
            let m = &key.modifiers;
            if opens_message_menu(&key.key, m.shift, m.control, m.alt, m.platform) {
                let target = self
                    .timeline
                    .as_ref()
                    .and_then(|t| t.keyboard_menu_target());
                if let Some(target) = target
                    && self.operations.borrow_mut().select(target)
                {
                    self.actions_focus().focus(window);
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            if key.key == "escape" && self.operations.borrow().selection().is_some() {
                self.close_message_actions(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        if key.key == "escape" && self.composer.update(cx, |c, cx| c.close_picker(cx)) {
            cx.stop_propagation();
            return;
        }
        if self.thread.is_some() && key.key == "escape" {
            self.close_thread();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.rail_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.ring_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.back_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && let Some(panel) = self.semantic_operation_panel(window)
            && let Some(action) = panel.key(
                &key.key,
                key.modifiers.shift,
                self.timeline.as_ref().is_some_and(|timeline| {
                    timeline
                        .semantic_snapshot(&mut self.a11y_history, window)
                        .messages
                        .iter()
                        .any(|message| message.event == panel.target && message.actions_focused)
                }),
            )
        {
            self.apply_semantic_operation_panel(action, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.connected && key.modifiers.control && key.modifiers.shift && key.key == "s" {
            self.set_spaces_open(!self.spaces_open);
            self.set_compact_chat(false);
            self.spaces_shortcut_focus(window, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.spaces_open && key.key == "escape" {
            self.set_spaces_open(false);
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            self.spaces_shortcut_focus(window, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.connected
            && !self.notifications_open
            && !self.verification_open
            && key.modifiers.control
            && key.key == "i"
        {
            if self.room_details_open && self.details_zone.contains_focused(window, cx) {
                self.list_focus.focus(window);
            }
            self.set_room_details_open(
                !self.room_details_open && self.sidebar.selected().is_some(),
            );
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.room_details_open && key.key == "f6" && !key.modifiers.modified() {
            if self.details_zone.contains_focused(window, cx) {
                self.composer.update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            } else {
                self.details_zone.focus(window);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // Non-modal: the panel only sees keys while it holds the focus; elsewhere they pass through.
        // The profile sheet covers the whole panel and a member click does not move the focus, so its ring keeps the keys also from outside.
        if self.room_details_open
            && (self.details_zone.contains_focused(window, cx) || self.profile.is_some())
        {
            if self.profile.is_some() && self.profile_key(key.key.as_str(), key.modifiers.shift, window, cx) {
                cx.stop_propagation();
                return;
            }
            // Without the a11y ring «Argomento» is the only scrollable part of the panel, so it owns the arrows.
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            let topic_focused = self.details_focus == matrix_desktop::accessibility::details::DetailsControl::Topic;
            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
            let topic_focused = true;
            if self.profile.is_none() && topic_focused && self.topic_scroll_key(&key.key) {
                cx.notify();
                cx.stop_propagation();
                return;
            }
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if matches!(key.key.as_str(), "tab" | "space" | "enter") {
                use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
                let view = self.semantic_details(window);
                if let DetailsView::Open(panel) = &view {
                    let action = if key.key == "tab" {
                        Some(DetailsAction::Focus {
                            scope: panel.scope.clone(),
                            control: panel.next_focus(key.modifiers.shift),
                        })
                    } else {
                        view.action(&accesskit::ActionRequest {
                            action: accesskit::Action::Click,
                            target_tree: accesskit::TreeId::ROOT,
                            target_node: panel.focus.id(),
                            data: None,
                        })
                    };
                    if let Some(action) = action {
                        self.apply_semantic_details(action, window, cx);
                    }
                }
                cx.stop_propagation();
                return;
            }
            if key.key == "escape" {
                self.set_room_details_open(false);
                self.list_focus.focus(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        if self.connected && key.modifiers.control && key.modifiers.shift && key.key == "n" {
            self.set_room_details_open(false);
            self.set_notifications_open(!self.notifications_open);
            self.set_verification_open(false);
            self.list_focus.focus(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.notifications_open {
            if matches!(key.key.as_str(), "tab" | "up" | "down") {
                let room = self.sidebar.selected().is_some();
                self.notification_focus =
                    self.notification_focus.next(room, key.modifiers.shift || key.key == "up");
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if matches!(key.key.as_str(), "space" | "enter") {
                use matrix_desktop::notifications::NotificationControl::*;
                match self.notification_focus {
                    Enabled => self.change_notification_preference(0),
                    Mute => self.change_notification_preference(1),
                    Preview => self.change_notification_preference(2),
                    Close => self.set_notifications_open(false),
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
            match key.key.as_str() {
                "escape" => self.set_notifications_open(false),
                "n" if !key.modifiers.control => self.change_notification_preference(0),
                "m" if !key.modifiers.control => self.change_notification_preference(1),
                "p" if !key.modifiers.control => self.change_notification_preference(2),
                _ => (),
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.connected && key.modifiers.control && key.modifiers.shift && key.key == "v" {
            self.set_notifications_open(false);
            self.set_room_details_open(false);
            self.set_verification_open(!self.verification_open);
            if self.verification_open {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    self.device_focus = Default::default();
                    self.a11y_device_bounds = [None; 2];
                }
                self.verification.refresh();
            }
            self.list_focus.focus(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.verification_open {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if matches!(key.key.as_str(), "tab" | "space" | "enter") {
                use matrix_desktop::accessibility::devices::{DeviceAction, DeviceView};
                let view = self.semantic_devices(window);
                if let DeviceView::Open(panel) = &view {
                    let action = if key.key == "tab" {
                        Some(DeviceAction::Focus {
                            account: panel.account,
                            control: panel.next_control(key.modifiers.shift),
                        })
                    } else {
                        view.action(&accesskit::ActionRequest {
                            action: accesskit::Action::Click,
                            target_tree: accesskit::TreeId::ROOT,
                            target_node: panel.control_id(panel.focus),
                            data: None,
                        })
                    };
                    if let Some(action) = action {
                        self.apply_semantic_devices(action, window, cx);
                    }
                }
                cx.stop_propagation();
                return;
            }
            if key.key == "escape" {
                self.set_verification_open(false);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.sidebar_tab(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.spaces_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if key.key == "tab"
            && !key.modifiers.shift
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && self.connected
            && self.list_focus.is_focused(window)
            && self.history.pagination_tag().is_some()
            && self.timeline.is_some()
        {
            self.set_compact_chat(true);
            self.history_focus.focus(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if (window.viewport_size().width >= px(800.) || self.compact_chat)
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && let Some(timeline) = &self.timeline
            && let Some(action) = {
                let snapshot = timeline.semantic_snapshot(&mut self.a11y_history, window);
                snapshot
                    .confirmation
                    .as_ref()
                    .and_then(|c| c.key(&key.key, key.modifiers.shift))
                    .map(matrix_desktop::accessibility::NavigationAction::LinkConfirmation)
                    .or_else(|| snapshot.event_id_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.link_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.selection_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.offline_delivery_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.video_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.poll_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.retry_key(&key.key, key.modifiers.shift))
                    .or_else(|| snapshot.actions_key(&key.key, key.modifiers.shift))
            }
        {
            use matrix_desktop::accessibility::NavigationAction;
            match action {
                NavigationAction::OfflineDelivery(intent) => {
                    timeline.apply_offline_delivery(&intent, window);
                }
                NavigationAction::CopyEventId(intent) => {
                    timeline.apply_event_id_copy(&intent, window, cx);
                }
                NavigationAction::PlayVideo(id) => {
                    timeline.play_video(&id, window);
                }
                NavigationAction::SaveAttachment(id) => {
                    timeline.save_attachment(&id);
                }
                NavigationAction::RecoverKeys(id) => {
                    timeline.focus_semantic_recover(&id, window, cx);
                }
                NavigationAction::LinkConfirmation(intent) => {
                    timeline.apply_confirmation(&intent, window, cx);
                }
                NavigationAction::MessageLink(intent) => {
                    timeline.apply_link(&intent, window);
                }
                NavigationAction::MessageSelection(intent) => {
                    timeline.apply_selection(&intent, window, cx);
                }
                NavigationAction::FocusPill(id, start) => {
                    timeline.apply_pill(&id, start, window);
                }
                NavigationAction::OpenProfile(user_id) => {
                    self.pending_profile(user_id);
                }
                NavigationAction::FocusMessageActions(id) => {
                    timeline.focus_semantic_actions(&id, window);
                }
                NavigationAction::OpenMessageActions(id) => {
                    if id.scope == self.active_scope
                        && self
                            .sidebar
                            .selected()
                            .is_some_and(|room| room.scope == id.scope)
                    {
                        self.operations.borrow_mut().select(id);
                    }
                }
                NavigationAction::FocusMessageRetry(id) => {
                    timeline.focus_semantic_retry(&id, window);
                }
                NavigationAction::FocusPollOption(id, ix) => {
                    timeline.focus_semantic_poll(&id, ix, window);
                }
                NavigationAction::VotePoll(id, ix) => {
                    if id.scope == self.active_scope
                        && self.sidebar.selected().is_some_and(|room| room.scope == id.scope)
                    {
                        self.vote_poll(&id, ix);
                    }
                }
                NavigationAction::FocusMessage(id) => {
                    timeline.focus_semantic_message(&id, window);
                }
                NavigationAction::FocusComposer => self
                    .composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx)),
                NavigationAction::RetryMessage(id) => {
                    if id.scope == self.active_scope
                        && self
                            .sidebar
                            .selected()
                            .is_some_and(|room| room.scope == id.scope)
                        && let matrix_domain::TimelineId::Pending(tx) = &id.id
                        && self.delivery.borrow_mut().retry(&id.scope, tx)
                    {
                        self.refresh_delivery(false);
                    }
                }
                _ => unreachable!(),
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.history_focus.is_focused(window)
            && self.connected
            && (window.viewport_size().width >= px(800.) || self.compact_chat)
        {
            if matches!(key.key.as_str(), "space" | "enter")
                && !key.modifiers.control
                && !key.modifiers.alt
                && !key.modifiers.platform
            {
                self.request_older();
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if key.key == "tab" {
                if key.modifiers.shift {
                    self.set_compact_chat(false);
                    self.search_focus.focus(window);
                } else {
                    self.composer
                        .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        if window.viewport_size().width < px(800.) && key.modifiers.alt && key.key == "left" {
            self.set_compact_chat(false);
            self.list_focus.focus(window);
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn sync_room(&mut self, cx: &mut Context<Self>) {
        // Arm before the first event: a slow sync must not open (and save) the first row.
        if self.connected {
            let (account, prefs) = (self.draft_scope().account_id, &self.room_prefs);
            self.restore
                .arm(account, || prefs.last_open_room(account));
        }
        // Waiting for the saved room: opening the auto-selected first row would mark it read.
        // Selection stays empty meanwhile; the empty scope below also closes a room that was shown before it vanished.
        if self.restore.waiting() {
            self.sidebar.clear_selection();
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if !self
            .sidebar
            .selected()
            .is_some_and(|r| r.scope == self.active_scope)
        {
            self.back_ui.nodes.borrow_mut().project(None);
        }
        if self.connected {
            let scope = self.sidebar.selected().map(|room| room.scope.clone());
            if let Some(request) = self.history.select(scope.clone()) {
                self.set_room_details_open(false);
                self.media.clear();
                // The note belongs to the room being left: stop the sound with it.
                self.audio = None;
                self.operations.borrow_mut().clear_selection();
                self.confirm_redaction = None;
                self.revoke_thread_read();
                self.active_scope = request.tag.scope().clone();
                self.remember_last_room();
                self.note_seen();
                let draft_scope = self.draft_scope();
                self.composer
                    .update(cx, |editor, cx| editor.switch_room(draft_scope, cx));
                let timeline =
                    TimelineView::connected(self.active_scope.clone(), cx.focus_handle());
                if let Some(room) = self.sidebar.selected() {
                    timeline.set_member_count(room.info.joined_members);
                }
                self.timeline = Some(timeline);
                self.pending_history = Some(request);
                self.read_dirty = true;
                self.refresh_delivery(false);
            } else if scope.is_none() {
                self.set_room_details_open(false);
                self.media.clear();
                // The note belongs to the room being left: stop the sound with it.
                self.audio = None;
                self.operations.borrow_mut().clear_selection();
                self.timeline = None;
                self.pending_history = None;
            }
            return;
        }
        let Some(scope) = self.sidebar.selected().map(|room| room.scope.clone()) else {
            return;
        };
        if scope != self.active_scope {
            self.composer
                .update(cx, |editor, cx| editor.switch_room(scope.clone(), cx));
            let next = self
                .saved_timelines
                .remove(&scope)
                .unwrap_or_else(|| TimelineView::new(scope.clone(), cx.focus_handle()));
            let previous = self.timeline.replace(next).expect("offline timeline");
            previous.retire_selection();
            self.saved_timelines
                .insert(self.active_scope.clone(), previous);
            self.revoke_thread_read();
            self.active_scope = scope;
            self.remember_last_room();
        }
    }

    /// Reopens the saved room of the active account once listed. Compact layout: the list stays the
    /// screen (no `set_compact_chat`), the room is only selected and scrolled into view.
    fn restore_last_room(&mut self, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        let account = self.active_scope.account_id;
        let saved = self.draft_scope().account_id;
        let (prefs, sidebar) = (&self.room_prefs, &mut self.sidebar);
        let opened = self.restore.poll(
            saved,
            || prefs.last_open_room(saved),
            |room| {
                let scope = RoomScope { account_id: account, room_id: room.clone() };
                sidebar.is_joined(&scope) && sidebar.select(&scope)
            },
        );
        if opened.is_none() && self.restore.waiting() {
            self.sidebar.clear_selection();
        }
        if let Some(room) = opened {
            // Read before sync_room: changing room rewrites the saved flag from the (still unfocused) composer.
            self.composer_focused = self.room_prefs.composer_focused(saved);
            self.focus_composer_on_restore = self.composer_focused;
            self.sync_room(cx);
            let scope = RoomScope { account_id: account, room_id: room };
            if let Some(position) = self.sidebar.position_of(&scope) {
                self.scroll.scroll_to_item(position, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    /// Called only when the user changes room; writes the file only if the value changed.
    fn remember_last_room(&mut self) {
        let account = self.draft_scope().account_id;
        let focus = self.composer_focused;
        if !self.restore.waiting()
            && (self.room_prefs.set_last_open_room(&self.draft_scope()) | self.room_prefs.set_composer_focused(account, focus))
            && let Some(dirs) = &self.prefs_dirs
            && let Err(error) = self.room_prefs.save(dirs)
        {
            eprintln!("Gnobbi: cannot save room preferences: {error}");
        }
    }

    fn actions_focus(&self) -> &FocusHandle {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            &self.operation_ui.focus
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            &self.actions_focus
        }
    }

    /// Dismiss the message actions and hand focus back to the message they came from.
    fn close_message_actions(&mut self, window: &mut Window) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let target = self.operations.borrow().selection().cloned();
        self.operations.borrow_mut().clear_selection();
        self.confirm_redaction = None;
        if let Some(timeline) = &self.timeline {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if let Some(target) = target {
                timeline.focus_semantic_actions(&target, window);
                return;
            }
            timeline.focus_timeline(window);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick.is_some() {
            return;
        }
        let key = &event.keystroke;
        if self.room_search_typing(&key.modifiers, window, cx) {
            return;
        }
        if self.connected {
            let account = self.draft_scope().account_id;
            self.restore.arm(account, || self.room_prefs.last_open_room(account));
        }
        let searching = self.search_focus.is_focused(window);
        if self.list_focus.is_focused(window)
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && !key.modifiers.shift
            && matches!(key.key.as_str(), "enter" | "space")
        {
            let target = self.sidebar.focused_room()
                .or_else(|| (!self.restore.waiting()).then(|| self.sidebar.selected()).flatten())
                .map(|room| (room.scope.clone(), room.info.invited));
            if let Some((scope, invited)) = target {
                if invited {
                    if key.key == "enter" { self.answer_invite(scope, true, cx); }
                } else if self.sidebar.select(&scope) {
                    self.restore.cancel();
                    self.sync_room(cx);
                    if window.viewport_size().width < px(800.) { self.set_compact_chat(true); }
                    cx.notify();
                }
            }
            return;
        }
        if !searching
            && let Some(scope) = self.sidebar.selected_invite().map(|room| room.scope.clone())
            && matches!(key.key.as_str(), "enter" | "delete" | "backspace")
        {
            self.answer_invite(scope, key.key == "enter", cx);
            return;
        }
        if window.viewport_size().width < px(800.) && !self.compact_chat && key.key == "enter" {
            if searching
                && !self.sidebar.visible_indices().iter().any(|&index| {
                    self.sidebar
                        .selected()
                        .is_some_and(|selected| selected.scope == self.sidebar.rooms()[index].scope)
                })
            {
                self.restore.cancel();
                self.sidebar.navigate(Navigation::First);
            }
            if !self.sidebar.visible_indices().is_empty() {
                self.sync_room(cx);
                self.set_compact_chat(true);
                self.list_focus.focus(window);
                cx.notify();
            }
            return;
        }
        let in_thread = self.thread.as_ref().and_then(|p| p.composer.as_ref()).is_some_and(|c| c.read(cx).is_focused(window));
        if (in_thread || self.composer.read(cx).is_focused(window)) && !(key.modifiers.control && key.key == "f") {
            return;
        }
        if key.modifiers.control && matches!(key.key.as_str(), "home" | "end") {
            if let Some(timeline) = &self.timeline {
                timeline.jump_to_boundary(key.key == "end");
            }
            cx.notify();
            return;
        }
        if key.modifiers.control && key.key == "pageup" {
            if self.connected {
                self.request_older();
            } else {
                self.timeline.as_ref().unwrap().prepend_older();
            }
            cx.notify();
            return;
        }
        if !self.connected && key.modifiers.control && key.key == "pagedown" {
            self.timeline.as_ref().unwrap().append_live();
            cx.notify();
            return;
        }
        if key.modifiers.control && key.key == "f" {
            self.set_compact_chat(false);
            self.search_focus.focus(window);
            cx.notify();
            return;
        }
        if key.key == "tab" || (searching && key.key == "enter") {
            if searching {
                self.list_focus.focus(window);
            } else {
                self.search_focus.focus(window);
            }
            cx.notify();
            return;
        }
        let direction = match key.key.as_str() {
            "down" => Some(Navigation::Next),
            "up" => Some(Navigation::Previous),
            "home" if !searching => Some(Navigation::First),
            "end" if !searching => Some(Navigation::Last),
            _ => None,
        };
        if let Some(direction) = direction {
            let waiting = self.restore.waiting();
            let position = if waiting {
                self.sidebar.navigate_focus(direction)
            } else {
                self.sidebar.navigate(direction)
            };
            if let Some(position) = position {
                self.scroll.scroll_to_item(position, ScrollStrategy::Top);
            }
            if !waiting {
                self.sync_room(cx);
            }
            cx.notify();
            return;
        }
        if !searching {
            return;
        }
        let mut query = self.sidebar.query().to_owned();
        match key.key.as_str() {
            "escape" => query.clear(),
            "backspace" => {
                query.pop();
            }
            _ if !key.modifiers.control && !key.modifiers.platform && !key.modifiers.alt => {
                if let Some(text) = &key.key_char {
                    query.extend(text.chars().filter(|c| !c.is_control()));
                }
            }
            _ => return,
        }
        self.sidebar.set_query(&query);
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Room at the top of the viewport and its position, when the user has scrolled down.
    fn reading_anchor(&self) -> Option<(RoomScope, usize)> {
        let state = self.scroll.0.borrow();
        let height = f32::from(state.last_item_size?.item.height);
        let offset = -f32::from(state.base_handle.offset().y);
        if offset <= 0. || height <= 0. {
            return None;
        }
        let position = (offset / height) as usize;
        let index = *self.sidebar.visible_indices().get(position)?;
        Some((self.sidebar.rooms()[index].scope.clone(), position))
    }

    /// Re-aim the scroll so the room being read stays under the pointer after a reorder.
    fn keep_reading_position(&mut self, anchor: Option<(RoomScope, usize)>) {
        let Some((scope, old)) = anchor else { return };
        let state = self.scroll.0.borrow();
        let Some(size) = state.last_item_size else {
            return;
        };
        let offset = -f32::from(state.base_handle.offset().y);
        let shifted = matrix_desktop::reading_offset(
            offset,
            f32::from(size.item.height),
            Some(old),
            self.sidebar.position_of(&scope),
        );
        state.base_handle.set_offset(point(px(0.), px(-shifted)));
    }

    fn request_older(&mut self) {
        if let Some(request) = self.history.load_older() {
            self.pending_history = Some(request);
        }
    }

    fn refresh_delivery(&self, reveal: bool) {
        if let Some(timeline) = &self.timeline {
            let delivery = self.delivery.borrow();
            let items = delivery.items(&self.active_scope);
            if !items.is_empty() {
                timeline.project_delivery(items, reveal);
            }
        }
    }

    /// Vote on option `ix`: the state comes from the Timeline, we only send the new answers.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn vote_poll(&mut self, id: &matrix_domain::ScopedTimelineId, ix: usize) {
        #[cfg(feature = "live")]
        if let matrix_domain::TimelineId::Server(target) = &id.id
            && let Some(poll) = self.timeline.as_ref().and_then(|t| t.poll_of(id))
        {
            self.operations.borrow_mut().vote(id.scope.clone(), target.clone(), &poll, ix, matrix_engine::sending::fresh_transaction_id());
        }
        #[cfg(not(feature = "live"))]
        let _ = (id, ix);
    }
    fn open_poll_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected || self.sidebar.selected().is_none() || self.poll_dialog.is_some() {
            return;
        }
        let theme = self.theme.tokens();
        let dialog = cx.new(|cx| poll_dialog::PollDialog::new(theme, cx));
        cx.subscribe_in(&dialog, window, |this: &mut Shell, _, event: &poll_dialog::PollDialogEvent, window, cx| {
            this.close_poll_dialog(window, cx);
            #[cfg(feature = "live")]
            if let poll_dialog::PollDialogEvent::Create(draft) = event
                && this.routes.send.is_some()
            {
                let accepted = this.delivery.borrow_mut().submit_poll(
                    this.active_scope.clone(),
                    matrix_engine::sending::fresh_transaction_id(),
                    draft.clone(),
                );
                if accepted {
                    this.refresh_delivery(true);
                }
            }
            #[cfg(not(feature = "live"))]
            let _ = event;
        })
        .detach();
        dialog.read(cx).focus_question(window, cx);
        self.poll_dialog = Some(dialog);
        cx.notify();
    }

    fn close_poll_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_dialog = None;
        self.composer.update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn semantic_poll(&self, window: &Window, cx: &gpui::App) -> Option<matrix_desktop::accessibility::new_room::Snapshot> {
        self.poll_dialog.as_ref().map(|dialog| dialog.read(cx).semantic(window, cx))
    }

    fn flush_operations(&mut self) {
        use matrix_engine::{
            TrySendError,
            operations::{OperationCommand, OperationError, OperationResult},
        };
        if self.members_requested.as_ref() != Some(&self.active_scope)
            && self.sidebar.selected().is_some()
            && let Some((sender, _)) = &mut self.routes.operation
            && sender
                .try_send(OperationCommand::Members {
                    scope: self.active_scope.clone(),
                })
                .is_ok()
        {
            self.members_requested = Some(self.active_scope.clone());
        }
        self.flush_user_search();
        self.flush_room_search();
        self.flush_directory();
        self.flush_quick_users();
        self.flush_quick_rooms();
        if self.pending_operation_command.is_none() {
            self.pending_operation_command = self.operations.borrow_mut().take_command();
        }
        if let Some(command) = self.pending_operation_command.take()
            && let Some((sender, _)) = &mut self.routes.operation
        {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(TrySendError::Full(command) | TrySendError::Busy(command)) => {
                    self.pending_operation_command = Some(command)
                }
                Err(TrySendError::Closed(command)) => {
                    let result = match command {
                        OperationCommand::Inspect { tag, target } => OperationResult::Inspected {
                            tag,
                            target,
                            details: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Apply(request) => OperationResult::Applied {
                            request,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::MarkRoom { scope, .. } => OperationResult::RoomMarked {
                            scope,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Profile { user_id } => OperationResult::Profile {
                            user_id,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Ignore { user_id, ignore } => OperationResult::Ignored {
                            user_id,
                            ignore,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::MemberRights { scope, user_id } => OperationResult::MemberRights {
                            scope,
                            user_id,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Moderate { scope, user_id, action } => OperationResult::Moderated {
                            scope,
                            user_id,
                            action,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Members { scope } => OperationResult::Members {
                            scope,
                            members: Err(OperationError::Unavailable),
                            upload_limit: None,
                        },
                        OperationCommand::ThreadSend { scope, root, .. } => OperationResult::ThreadSent {
                            scope,
                            root,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::Thread { tag, root, .. } => OperationResult::Thread {
                            tag,
                            root,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::ThreadRead { tag, root, target, .. } => OperationResult::ThreadRead {
                            tag,
                            root,
                            target,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::SetMuted { scope, .. } => OperationResult::RoomMarked {
                            scope,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::EditRoom { .. } => OperationResult::RoomEdited {
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::AnswerInvite { scope, accept } => {
                            OperationResult::InviteAnswered {
                                scope,
                                accept,
                                result: Err(OperationError::Unavailable),
                            }
                        }
                        OperationCommand::EditTags(_) => OperationResult::TagsEdited {
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::SearchMessages { scope, term } => OperationResult::MessageSearch {
                            scope,
                            state: matrix_engine::operations::MessageSearchState::Error { term },
                        },
                        // Nothing is open on the engine side: there is no page to ask for.
                        OperationCommand::MoreMessages => return,
                        OperationCommand::SearchUsers { .. } => OperationResult::UserSearch {
                            state: matrix_engine::operations::SearchState::Error,
                            all_available: false,
                        },
                        OperationCommand::SearchRooms { term, .. } => OperationResult::RoomSearch {
                            state: matrix_engine::search::SearchState::Error {
                                term,
                                error: matrix_engine::search::SearchError::Network,
                            },
                        },
                        OperationCommand::MoreRooms => OperationResult::RoomSearch {
                            state: matrix_engine::search::SearchState::Idle,
                        },
                        OperationCommand::ProbeServer { server } => OperationResult::ServerProbed {
                            server,
                            result: Err(matrix_engine::search::SearchError::Network),
                        },
                        OperationCommand::LeaveRoom { scope } => OperationResult::RoomLeft {
                            scope,
                            result: Err(OperationError::Unavailable),
                        },
                        OperationCommand::StartDm { .. }
                        | OperationCommand::CreateRoom { .. }
                        | OperationCommand::JoinRoom { .. } => {
                            OperationResult::RoomStarted {
                                result: Err(OperationError::Unavailable),
                            }
                        }
                    };
                    self.operations.borrow_mut().apply(result);
                }
            }
        }
    }

    fn render_operations(&mut self, _window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        use matrix_engine::operations::OperationError;
        let actions = self.operations.borrow();
        let selected = actions.selection().is_some();
        let busy = actions.is_pending();
        let permissions = actions.details().map(|d| d.permissions).unwrap_or_default();
        let error = actions.error();
        if self.confirm_redaction.as_ref() != actions.selection() {
            self.confirm_redaction = None;
        }
        let confirming = self.confirm_redaction.is_some();
        let ending = confirming && self.confirm_end;
        let can_end = actions.can_end_poll(actions.selection().and_then(|id| self.timeline.as_ref()?.poll_of(id)).as_ref());
        let notice = actions.notice(confirming && !ending);
        drop(actions);
        if !selected && !busy && error.is_none() {
            return div().into_any_element();
        }
        let t = self.theme.tokens();
        let probe = std::env::var("MATRIX_DESKTOP_TIMELINE_PROBE").as_deref() == Ok("1");
        let controls = if ending {
            vec![("confirm-redact", "Chiudi", can_end), ("cancel-redact", "Annulla", true)]
        } else if confirming {
            vec![
                (
                    "confirm-redact",
                    "Conferma elimina",
                    permissions.redact && !busy,
                ),
                ("cancel-redact", "Annulla", true),
            ]
        } else {
            vec![
                ("reply", "Rispondi", permissions.reply && !busy),
                ("thread", "Rispondi nel thread", permissions.reply && !busy),
                ("edit", "Modifica", permissions.edit && !busy),
                ("redact", "Elimina", permissions.redact && !busy),
                ("end-poll", "Chiudi sondaggio", can_end),
                ("react", "+1 / togli", permissions.react && !busy),
                ("retry", "Riprova", error == Some(OperationError::Transport)),
                ("close", "Chiudi", true),
            ]
        };
        div().flex_shrink_0().px_4().py_2().bg(rgb(t.surface)).border_t_1().border_color(rgb(t.selection))
            .when(ending, |d| d.child(div().text_size(px(12.)).text_color(rgb(t.accent)).child("Chiudere il sondaggio?")))
            .when(notice != matrix_desktop::LiveOperations::IDLE_NOTICE, |d| d.child(div().text_size(px(11.)).text_color(rgb(t.secondary)).whitespace_normal().child(notice)))
            .child(div().flex().flex_wrap().gap_2().children(controls.into_iter().map(|(name,label,enabled)| {
                let d = div().id(name).relative().px_2().h(px(28.)).flex().items_center().text_size(px(12.))
                    .text_color(rgb(if enabled { t.accent } else { t.secondary }))
                    .when(enabled, |d| d.cursor_pointer())
                    .child(if enabled { label.to_owned() } else { format!("{label} · n/d") })
                    .on_click(cx.listener(move |this, event, _window, cx| {
                        if !enabled { return; }
                        // Semantic key-down owns these controls. GPUI synthesizes
                        // a key-up click on the *new* focus after Begin/Cancel;
                        // never let it advance or undo the confirmation stage.
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        if matches!(event, gpui::ClickEvent::Keyboard(_)) { return; }
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        let _ = event;
                        match name {
                            "close" => this.close_message_actions(_window),
                            "redact" | "end-poll" => {
                                this.confirm_end = name == "end-poll";
                                let target = this.operations.borrow().selection().cloned();
                                if let Some(target) = target {
                                    this.operations.borrow_mut().select(target.clone());
                                    this.confirm_redaction = Some(target);
                                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                    this.operation_ui.focus.focus(_window);
                                }
                            },
                            "cancel-redact" => { this.confirm_redaction = None; this.confirm_end = false; },
                            "retry" => { this.operations.borrow_mut().retry(); },
                            "thread" => {
                                let target = this.operations.borrow().selection().map(|s| s.id.clone());
                                if let Some(matrix_domain::TimelineId::Server(id)) = target {
                                    this.operations.borrow_mut().clear_selection();
                                    this.open_thread(&id, Some(_window), cx);
                                }
                            },
                            _ => {
                                #[cfg(feature="live")]
                                {
                                    let transaction = matrix_engine::sending::fresh_transaction_id();
                                    match name {
                                        "reply" | "edit" => this.composer.update(cx, |editor,cx| editor.accept_operation(&mut this.operations.borrow_mut(), name == "edit", transaction, cx)),
                                        "react" => { this.operations.borrow_mut().toggle_reaction("+1", transaction); },
                                        "confirm-redact" => {
                                            let ending = std::mem::take(&mut this.confirm_end);
                                            if this.confirm_redaction.as_ref() == this.operations.borrow().selection() { this.operations.borrow_mut().submit(if ending { matrix_engine::operations::Operation::EndPoll } else { matrix_engine::operations::Operation::Redact }, transaction); }
                                            this.confirm_redaction = None;
                                        },
                                        _ => (),
                                    }
                                }
                            }
                        }
                        cx.notify();
                    }))
                    .when(probe, |d| d.child(gpui::canvas(move |bounds,_,_| {
                        eprintln!("Matrix Desktop operation {{\"control\":\"{name}\",\"enabled\":{enabled},\"bounds\":[{},{},{},{}]}}", f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height));
                    }, |_,_,_,_| {}).absolute().inset_0()));
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                let d = d.when(name == "close" || name == "cancel-redact", |d| d.track_focus(&self.actions_focus)
                    .when(input_modality::ring(&self.actions_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent))));
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let d = d.when(name == "close" || name == "cancel-redact", |d| d.track_focus(&self.operation_ui.focus)
                    .when(input_modality::ring(&self.operation_ui.focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                    .children(self.operation_control_observer("close", cx)))
                    .when(name == "reply" && enabled, |d| d.track_focus(&self.operation_ui.reply_focus)
                        .when(input_modality::ring(&self.operation_ui.reply_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("reply", cx)))
                    .when(name == "edit" && enabled, |d| d.track_focus(&self.operation_ui.edit_focus)
                        .when(input_modality::ring(&self.operation_ui.edit_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("edit", cx)))
                    .when(name == "react" && enabled, |d| d.track_focus(&self.operation_ui.react_focus)
                        .when(input_modality::ring(&self.operation_ui.react_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("react", cx)))
                    .when((name == "redact" || name == "confirm-redact") && enabled, |d| d.track_focus(&self.operation_ui.redact_focus)
                        .when(input_modality::ring(&self.operation_ui.redact_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer(name, cx)))
                    .when(name == "end-poll" && enabled, |d| d.track_focus(&self.operation_ui.end_focus)
                        .when(input_modality::ring(&self.operation_ui.end_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("end-poll", cx)))
                    .when(name == "retry" && enabled, |d| d.track_focus(&self.operation_ui.retry_focus)
                        .when(input_modality::ring(&self.operation_ui.retry_focus, _window, cx), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("retry", cx)));
                d
            }))).into_any_element()
    }

    fn flush_history(&mut self) {
        if let Some(request) = self.pending_history.take()
            && let Some((sender, _)) = &mut self.routes.history
        {
            match sender.try_send(request) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(request)
                    | matrix_engine::TrySendError::Busy(request),
                ) => self.pending_history = Some(request),
                Err(matrix_engine::TrySendError::Closed(request)) => {
                    self.history.apply(matrix_engine::history::HistoryResult {
                        tag: request.tag,
                        page: Err(matrix_engine::history::HistoryError::Unavailable),
                    });
                }
            }
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_update_watch(cx);
        let down = self.connected && matches!(self.status, RoomSyncStatus::AuthenticationFailed | RoomSyncStatus::SyncFailed);
        if self.outage.observe(down, std::time::Instant::now()) {
            // Redraw once the grace has passed so the icon appears without waiting for another event.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(matrix_desktop::disconnect::GRACE).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
        // Common wrapper: sign-in, notifications and verification return early from `render_screen` and must update the input modality too.
        div().size_full().relative()
            .capture_key_down(|_, _, cx| input_modality::set_keyboard(cx))
            // Window-level listener: a click under an `.occlude()` overlay never reaches the root's own mouse handlers.
            .child(gpui::canvas(|_, _, _| (), |_, _, window, _| {
                window.on_mouse_event(|_: &gpui::MouseDownEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Capture { input_modality::set_pointer(cx); }
                });
            }).absolute().size_0())
            .child(self.render_screen(window, cx))
            .children(self.render_verification_popup(cx))
    }
}

impl Shell {
    /// The reason to show, once the outage has outlived the grace; None while connected.
    fn disconnected_shown(&self) -> Option<&'static str> {
        self.outage.visible(std::time::Instant::now()).then(|| matrix_desktop::disconnect::reason(&self.status)).flatten()
    }

    fn render_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Whatever selected a row meanwhile (diff fallback), a still-awaited saved room means no row is selected.
        if self.restore.waiting() {
            self.sidebar.clear_selection();
        }
        let pinned_picker = self.composer.read(cx).pinned_picker();
        if std::mem::take(&mut self.refocus_menu) {
            self.menu_focus.focus(window);
        }
        // Nothing focused (fresh window): keys never reach the elements, so Tab, Enter and Ctrl+Shift+M would all be dead
        // without a mouse click. The Menu button is the first stop and opens the drawer.
        if self.connected && window.focused(cx).is_none() {
            self.menu_focus.focus(window);
        }
        // The sign-in inputs are gone after login: without this nothing has focus and shortcuts stay dead.
        if std::mem::take(&mut self.refocus_list) {
            self.list_focus.focus(window);
        }
        if std::mem::take(&mut self.refocus_join_field) {
            self.focus_dialog_field(window, cx);
        }
        #[cfg(feature = "live")]
        if let Some(signin) = &self.signin {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            self.accessibility.update(
                matrix_desktop::accessibility::NavigationSnapshot::for_signin(
                    signin.read(cx).semantic_snapshot(window, cx),
                ),
                window.is_window_active(),
            );
            signin.update(cx, |view, _| view.theme = self.theme.tokens());
            let t = self.theme.tokens();
            return div().size_full().bg(rgb(t.surface)).pt(matrix_desktop::window::titlebar_inset(window.is_fullscreen())).child(signin.clone()).into_any_element();
        }
        window.set_window_title(&matrix_desktop::window::account_title(self.verification.profile()));
        let compact = window.viewport_size().width < px(800.);
        let list_width = self.list_width(window);
        if let (Some(timeline), Some(room)) = (&self.timeline, self.sidebar.selected()) {
            timeline.set_peer_receipts(&room.info.public_read_receipts);
        }
        if let Some(timeline) = &self.timeline {
            timeline.set_hide_redacted(self.room_prefs.hide_redacted(&self.active_scope).unwrap_or(self.prefs.hide_redacted));
            timeline.set_hide_status(self.room_prefs.hide_status(&self.active_scope));
        }
        #[cfg(test)]
        if navigation_resize_test::FROZEN.load(std::sync::atomic::Ordering::SeqCst) {
            return div().size_full().into_any_element();
        }
        self.navigation_wide = !compact;
        self.refresh_navigation_visibility();
        self.reconcile_sidebar_focus(window, cx);
        if self.focus_composer_on_restore {
            self.focus_composer_on_restore = false;
            if !compact {
                self.composer.update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            }
        } else if !self.focus_composer_on_restore {
            self.composer_focused = self.composer.read(cx).is_focused(window);
        }
        // Publish the same page that will actually be painted after resize.
        if compact && self.composer.read(cx).is_focused(window) {
            self.set_compact_chat(true);
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if !compact && self.back_ui.focus.is_focused(window) {
            self.composer
                .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let rooms = self.semantic_rooms();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        self.accessibility.update(
            matrix_desktop::accessibility::NavigationSnapshot {
                title: matrix_desktop::window::account_title(self.verification.profile()),
                #[cfg(feature = "live")]
                signin: None,
                openers: self.semantic_openers(window),
                back: self.semantic_back(window),
                drawer: self.semantic_drawer(),
                scrollbars: self.semantic_scrollbars(window),
                operation_panel: self.semantic_operation_panel(window),
                spaces: self.semantic_spaces(window),
                navigation_revision: self.sidebar.navigation_revision(),
                icon_choice: self.settings_open.then_some(self.prefs.icon),
                disconnected: self.disconnected_shown(),
                new_room: self.semantic_new_room(window, cx).or_else(|| self.semantic_poll(window, cx)).or_else(|| self.semantic_room_edit(window, cx)).or_else(|| self.semantic_quick_search(window, cx)),
                devices: self.semantic_devices(window),
                details: self.semantic_details(window),
                notifications: self.semantic_notifications(window),
                settings: self.semantic_settings(),
                about: self.semantic_about(),
                new_chat: self.semantic_new_chat(cx),
                room_search: self.semantic_room_search(window, cx),
                dark: self.theme.is_dark(),
                visible: !self.notifications_open
                    && !self.verification_open
                    && (window.viewport_size().width >= px(800.) || !self.compact_chat),
                query: self.sidebar.query().to_owned(),
                search_focused: self.search_focus.is_focused(window),
                search_bounds: self.a11y_search_bounds,
                rooms,
                list_focused: self.list_focus.is_focused(window),
                history: (!self.notifications_open
                    && !self.verification_open
                    && (window.viewport_size().width >= px(800.) || self.compact_chat))
                    .then(|| {
                        self.timeline.as_ref().map(|timeline| {
                            let mut snapshot =
                                timeline.semantic_snapshot(&mut self.a11y_history, window);
                            if self.connected {
                                if let Some(room) = self.sidebar.selected() {
                                    snapshot.apply_public_receipts(
                                        &room.scope,
                                        &room.info.public_read_receipts,
                                    );
                                }
                                snapshot.server = self.history.pagination_tag().map(|tag| {
                                    matrix_desktop::accessibility::pagination::ServerPagination {
                                        tag,
                                        status: self.history.status(),
                                        bounds: self.a11y_history_bounds,
                                        focused: self.history_focus.is_focused(window),
                                    }
                                });
                            }
                            snapshot
                        })
                    })
                    .flatten(),
                composer: (!self.notifications_open
                    && !self.verification_open
                    && (window.viewport_size().width >= px(800.) || self.compact_chat)
                    && (!self.connected || self.sidebar.selected().is_some()))
                .then(|| {
                    self.composer
                        .read(cx)
                        .semantic_snapshot(self.active_scope.clone(), window)
                }),
            },
            window.is_window_active(),
        );
        #[cfg(target_os = "linux")]
        {
            self.poll_install_offer(window, cx);
            #[cfg(feature = "linux-a11y")]
            self.publish_install_offer();
        }
        if self.notifications_open {
            return self.render_notifications(cx);
        }
        if self.verification_open {
            return self.render_verification(cx);
        }
        let t = self.theme.tokens();
        let theme = self.theme;
        let selected = self.sidebar.selected();
        let header_name = selected.map_or("Nessuna stanza".to_owned(), |room| room.name.clone());
        let header_initials = selected.map_or(String::new(), |room| room.initials.clone());
        let (list_name, list_initials, list_color) = self.sidebar.list_header();
        let list_picture = self.avatars.get(self.sidebar.list_header_avatar());
        let header_color = selected.map_or(t.disabled, |room| room.color);
        let header_picture = self
            .avatars
            .get(selected.and_then(|room| room.info.avatar.as_deref()));
        let connection_label = if !self.connected {
            "demo offline"
        } else {
            match self.status {
                RoomSyncStatus::Connecting => "connessione",
                RoomSyncStatus::Ready => "connesso",
                RoomSyncStatus::AuthenticationFailed => "sblocco fallito",
                RoomSyncStatus::SyncFailed => "sync interrotta",
            }
        };
        div()
            .track_focus(&self.list_focus)
            .relative()
            .capture_key_down(cx.listener({
                let opener_keys=self.opener_key_callback();
                move |this,event,window,cx| {
                    if this.verification.popup_open() {this.navigation_key(event,window,cx);}
                    else if this.menu_key(event,window,cx) || this.sidebar_activation_down(event,window,cx) || opener_keys(this,event,window,cx) {cx.stop_propagation();}
                    else {this.navigation_key(event,window,cx);}
                }
            }))
            .capture_key_up(cx.listener(Self::sidebar_activation_up))
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .pt(matrix_desktop::window::titlebar_inset(window.is_fullscreen()))
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .font_family(matrix_desktop::ui_font())
            .text_size(px(14.))
            // inner box: absolute overlays (top_0/inset_0) start below the titlebar inset, which stays draggable
            .child(div().relative().size_full().flex()
            .when(!compact, |d| d.child(self.render_rail(window, cx)))
            .when(!compact || !self.compact_chat, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .w(if compact {
                            window.viewport_size().width
                        } else {
                            px(list_width)
                        })
                        .h_full()
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(rgb(t.border))
                        .child(
                            div()
                                .h(px(t.header))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .px_4()
                                .gap_2()
                                .when(compact, |d| d.child(self.render_menu_button(t, window, cx)))
                                .child(
                                    div()
                                        .relative()
                                        .children({
                                            #[cfg(test)] { sidebar_navigation_test::observe("list-avatar".into()) }
                                            #[cfg(not(test))] { None::<gpui::AnyElement> }
                                        })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(32.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(rgb(list_color.unwrap_or(t.input)))
                                        .text_color(rgb(if list_color.is_some() { t.avatar_text } else { t.secondary }))
                                        .overflow_hidden()
                                        .child(match list_picture {
                                            Some(image) => avatar_view::round(image, 32.),
                                            None => gpui::div().child(list_initials).into_any_element(),
                                        }),
                                )
                                .child(
                                    div()
                                        .relative()
                                        .children({
                                            #[cfg(test)] { sidebar_navigation_test::observe("list-name".into()) }
                                            #[cfg(not(test))] { None::<gpui::AnyElement> }
                                        })
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(18.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(list_name),
                                )
                                // Fixed spot, absent (not hidden) while connected: nothing in the Tab order, no help text.
                                .when_some(self.disconnected_shown(), |d, reason| {
                                    d.child(
                                        div()
                                            .id("disconnected")
                                            .flex_shrink_0()
                                            .child(composer_icons::icon(composer_icons::Icon::Alert, t.error))
                                            .tooltip(move |_, cx| cx.new(|_| rail_view::RailTooltip { label: reason.into(), theme }).into()),
                                    )
                                }),
                        )
                        // The room menu closes on click, so its failed «Fissa in alto» action reports here (the drawer shows the same line).
                        .children(self.sidebar.tag_error().map(|e| div().mx_4().mb_2().text_size(px(12.)).text_color(rgb(t.error)).child(e)))
                        .child(
                            div()
                                .mx_4()
                                .mb_3()
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .bg(rgb(t.input))
                                .text_color(rgb(t.secondary))
                                .text_size(px(12.))
                                .id("room-search")
                                .relative()
                                .children(self.search_observer(cx))
                                .children({
                                    #[cfg(test)] { sidebar_navigation_test::observe("search".into()) }
                                    #[cfg(not(test))] { None::<gpui::AnyElement> }
                                })
                                .track_focus(&self.search_focus)
                                .border_1()
                                .border_color(rgb(if input_modality::only_keys(self.search_focus.is_focused(window), cx) {
                                    t.accent
                                } else {
                                    t.border
                                }))
                                .cursor(gpui::CursorStyle::IBeam)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search_focus.focus(window);
                                    cx.notify();
                                }))
                                .child(if self.sidebar.query().is_empty() {
                                    "Cerca conversazioni".to_owned()
                                } else {
                                    format!("{}│", self.sidebar.query())
                                }),
                        )
                        .when(compact && self.connected && self.sidebar.spaces().next().is_some(), |d| d.child(self.render_spaces(window, cx)))
                        .child(
                            div().relative().flex_1().min_h_0().children({
                                #[cfg(test)] { sidebar_navigation_test::observe("list".into()) }
                                #[cfg(not(test))] { None::<gpui::AnyElement> }
                            }).child(uniform_list(
                                // Spaces remain a filter within the existing sidebar.
                                "rooms",
                                self.sidebar.visible_indices().len(),
                                cx.processor(
                                    move |this, range: std::ops::Range<usize>, window, cx| {
                                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                        let semantic_rooms = this.a11y_rooms.project(&this.sidebar);
                                        range
                                            .map(|position| {
                                                let i = this.sidebar.visible_indices()[position];
                                                let room = &this.sidebar.rooms()[i];
                                                let scope = room.scope.clone();
                                                let invite = room.info.invited.then(|| this.sidebar.invite_state(&scope));
                                                let invite_buttons = invite.map(|state| this.invite_buttons(&scope, state, t, cx));
                                                let source_visit = this.sidebar.navigation_callback();
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                let callback = {
                                                    let source = &semantic_rooms[position];
                                                    compact_view::room_callback(matrix_desktop::accessibility::RoomIntent {
                                                        id: source.id, scope: source.scope.clone(), revision: source.revision,
                                                    })
                                                };
                                                let selected = this
                                                    .sidebar
                                                    .selected()
                                                    .is_some_and(|r| r.scope == scope);
                                                let background =
                                                    if selected { t.selected } else { t.surface };
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                let room_observer = {
                                                    let source = &semantic_rooms[position];
                                                    this.room_observer(matrix_desktop::accessibility::RoomIntent { id: source.id, scope: source.scope.clone(), revision: source.revision }, cx)
                                                };
                                                let focused = this.sidebar.focused_room().map_or(selected, |target| target.scope == scope);
                                                div()
                                            .id(i)
                                            .relative()
                                            .when(cfg!(all(feature = "linux-a11y", target_os = "linux")), |d| {
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                { d.child(room_observer) }
                                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                                { d }
                                            })
                                            .w_full()
                                            .border_2()
                                            .border_color(rgb(
                                                if focused && input_modality::ring(&this.list_focus, window, cx) {
                                                    t.accent
                                                } else {
                                                    background
                                                },
                                            ))
                                            .cursor_pointer()
                                            .when(this.connected, |d| {
                                                let scope = scope.clone();
                                                d.on_mouse_down(gpui::MouseButton::Right, cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                                                    this.sidebar.clear_leave_error();
                                                    this.room_menu = Some((scope.clone(), e.position));
                                                    cx.notify();
                                                }))
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.navigation_wide = window.viewport_size().width >= px(800.);
                                                this.refresh_navigation_visibility();
                                                if !source_visit(&this.sidebar) { return; }
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                let Some(scope) = callback(&this.a11y_rooms, &this.sidebar) else { return; };
                                                if !this.sidebar.select(&scope) { return; }
                                                this.restore.cancel();
                                                this.sync_room(cx);
                                                this.set_compact_chat(true);
                                                this.list_focus.focus(window);
                                                cx.notify();
                                            }))
                                            .flex()
                                            .items_center()
                                            .h(px(matrix_desktop::row_text::ROW_HEIGHT_PX))
                                            .flex_shrink_0()
                                            .px(px(matrix_desktop::row_text::LIST_PAD_X_PX))
                                            .gap(px(matrix_desktop::row_text::AVATAR_GAP_PX))
                                            .bg(rgb(background))
                                            .child(match this.avatars.get(room.info.avatar.as_deref()) {
                                                Some(image) => avatar_view::round(image, matrix_desktop::row_text::AVATAR_PX),
                                                None => div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .size(px(matrix_desktop::row_text::AVATAR_PX))
                                                    .flex_shrink_0()
                                                    .rounded_full()
                                                    .bg(rgb(room.color))
                                                    .text_color(rgb(t.avatar_text))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(room.initials.clone())
                                                    .into_any_element(),
                                            })
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap(px(matrix_desktop::row_text::TEXT_GAP_PX))
                                                    .child(name_line::name_line(
                                                        gpui::StyledText::new(matrix_desktop::row_text::one_line(&room.name)),
                                                        room.stamp().map(gpui::StyledText::new),
                                                        t.secondary,
                                                    ))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .h(px(matrix_desktop::row_text::PREVIEW_LINE_PX))
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .truncate()
                                                                    .text_size(px(matrix_desktop::row_text::PREVIEW_PX))
                                                                    .line_height(px(matrix_desktop::row_text::PREVIEW_LINE_PX))
                                                                    .text_color(rgb(t.secondary))
                                                                    .child(match invite {
                                                                        Some(state) => invite_view::preview(state).to_owned(),
                                                                        None => this.sidebar.leave_error(&room.scope).map_or_else(|| matrix_desktop::row_text::one_line(&room.preview), str::to_owned),
                                                                    }),
                                                            )
                                                            // Badge under the time, in the preview line (Telegram).
                                                            .when(this.connected && (room.unread > 0 || room.info.marked_unread), |d| {
                                                                d.child(
                                                                    div()
                                                                        .flex_shrink_0()
                                                                        .px_2()
                                                                        .rounded_full()
                                                                        .bg(rgb(t.accent))
                                                                        .text_color(rgb(t.on_accent))
                                                                        .text_size(px(11.))
                                                                        .line_height(px(matrix_desktop::row_text::PREVIEW_LINE_PX))
                                                                        .child(if room.unread > 0 { room.unread.to_string() } else { "•".to_owned() }),
                                                                )
                                                            })
                                                            // Same place, when there is no badge: the pin of a pinned room or the arrow of a low-priority one.
                                                            // Name for screen readers («Fissata» / «Bassa priorità») is in the row label (accessibility.rs).
                                                            .when(!(this.connected && (room.unread > 0 || room.info.marked_unread)) && (room.pinned() || room.low_priority()), |d| {
                                                                let kind = if room.pinned() { composer_icons::Icon::PinFilled } else { composer_icons::Icon::LowPriority };
                                                                d.child(div().flex_shrink_0().flex().items_center().h(px(matrix_desktop::row_text::PREVIEW_LINE_PX)).child(composer_icons::icon_sized(kind, t.secondary, 16.)))
                                                            }),
                                                    ),
                                            )
                                            .children(invite_buttons)
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                ),
                            )
                            .track_scroll(self.scroll.clone())
                            .flex_1()
                            .min_h_0().h_full()).child(self.room_scrollbar.render("room-scrollbar", scrollbar_view::ScrollTarget::Rooms(self.scroll.clone()), t)),
                        )
                        .when(self.sidebar.visible_indices().is_empty(), |d| {
                            d.child(
                                div()
                                    .px_4()
                                    .py_3()
                                    .text_color(rgb(t.secondary))
                                    .child("Nessuna conversazione trovata"),
                            )
                        })
                )
            })
            // Tab stop of the divider: between the list and the chat, like its place on screen.
            .when(!compact, |d| d.child(self.render_list_divider_focus(cx)))
            .when(!compact || self.compact_chat, |d| {
                d.child(
                    div()
                        .id("chat-column")
                        .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                            let paths = paths.paths().to_vec();
                            this.composer.update(cx, |editor, cx| editor.attach_paths(paths, cx));
                        }))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(t.header))
                                .flex_shrink_0()
                                .px_6()
                                .gap_3()
                                .border_b_1()
                                .border_color(rgb(t.border))
                                .when(compact, |d| {
                                    d.child(self.render_back(window, cx))
                                })
                                .when(!compact, |d| {
                                    d.child(
                                        match header_picture {
                                            Some(image) => avatar_view::round(image, 40.),
                                            None => div()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .size(px(40.))
                                                .rounded_full()
                                                .bg(rgb(header_color))
                                                .text_color(rgb(t.avatar_text))
                                                .child(header_initials)
                                                .into_any_element(),
                                        },
                                    )
                                })
                                .children(self.render_room_search_field(t, cx))
                                .when(self.room_search.is_none(), |d| d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .flex_1().min_w_0().overflow_hidden()
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(matrix_desktop::row_text::one_line(&header_name)),
                                        )
                                        .when(!self.connected, |d| d.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(t.secondary))
                                                .child(DEMO_NOTICE),
                                        )),
                                ))
                                .when(self.connected && self.sidebar.selected().is_some(), |d| d.child(self.render_root_opener(matrix_desktop::panel_openers::Control::Search, window, cx)).child(self.render_root_opener(matrix_desktop::panel_openers::Control::Info, window, cx))),
                        )
                        .children(self.render_room_search_results(t, cx))
                        // a11y: the strip is also the history Tab stop, so it must stay in the focus tree (a hairline when it has no text).
                        .when(self.connected && (cfg!(all(feature = "linux-a11y", target_os = "linux")) || matches!(self.history.status(), HistoryStatus::Loading | HistoryStatus::Failed)), |d| d.child(div().id("load-server-history").relative().h(px(if self.history.status().strip_label().is_some() { 24. } else { 0. })).flex_shrink_0().px_6().flex().items_center().bg(rgb(t.surface)).text_size(px(12.)).text_color(rgb(t.accent)).cursor_pointer()
                            .map(|d| {
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                {
                                    let entity = cx.entity();
                                    d.track_focus(&self.history_focus).border_1().border_color(rgb(if input_modality::only_keys(self.history_focus.is_focused(window), cx) { t.accent } else { t.surface })).child(gpui::canvas(|_,_,_| (), move |bounds,_,window,cx| {
                                        let scale = f64::from(window.scale_factor());
                                        let rect = accesskit::Rect::new(f64::from(f32::from(bounds.left())) * scale, f64::from(f32::from(bounds.top())) * scale, f64::from(f32::from(bounds.right())) * scale, f64::from(f32::from(bounds.bottom())) * scale);
                                        if entity.read(cx).a11y_history_bounds != Some(rect) {
                                            let entity = entity.clone();
                                            window.on_next_frame(move |_,cx| entity.update(cx, |this,cx| { this.a11y_history_bounds = Some(rect); cx.notify(); }));
                                        }
                                    }).absolute().inset_0())
                                }
                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                d
                            })
                            .child(self.history.status().strip_label().unwrap_or_default()).on_click(cx.listener(|this, _, _, cx| { this.request_older(); cx.notify(); }))))
                        .when(self.prefs.debug && self.connected && self.sidebar.selected().is_some(), |d| d.child(
                            div().flex_shrink_0().px_4().py_1().text_size(px(11.)).text_color(rgb(t.secondary))
                                .child(matrix_desktop::debug_room_line(&self.active_scope, connection_label))))
                        .when_some(self.timeline.as_ref(), |d, timeline| { timeline.set_debug(self.prefs.debug); { let (offered, pending, message) = self.key_recovery_state(); timeline.set_key_recovery(offered, pending, message); } d.child(timeline.render(t, window, self.connected.then(|| self.delivery.clone()), self.connected.then(|| self.operations.clone()), cx))})
                        .when(self.connected && self.timeline.is_none(), |d| d.child(div().flex_1().bg(rgb(t.timeline)).p_6().child("Nessuna stanza selezionata. Nessun messaggio demo.")))
                        .when(self.connected, |d| d.child(self.render_operations(window, cx)))
                        .when(!self.connected || self.sidebar.selected().is_some(), |d| d.child(self.composer.clone())),
                )
            })
            .when(!compact, |d| d.children(self.render_list_divider(window, cx)))
            .when(self.room_details_open, |d| d.child(self.render_room_details(window, cx)))
            .when_some(pinned_picker.clone(), |d, p| d.child(thread_view::side_panel("emoji-drawer", window, t, list_width).child(p)))
            .when(self.thread.is_some() && pinned_picker.is_none(), |d| d.child(self.render_thread(window, cx)))
            .when(self.rail_edit_open(), |d| d.child(self.render_edit_panel(window, cx)))
            .when(self.menu_open, |d| d.child(self.render_drawer(t, window, cx)))
            .when(self.room_menu.is_some(), |d| d.child(self.render_room_menu(t, cx)))
            .children(self.render_new_room(t, window, cx))
            .children(self.render_link_notice(t))
            .children(self.poll_dialog.clone().inspect(|dialog| {
                dialog.update(cx, |d, _| d.theme = t);
            }))
            .children(self.render_quick_search(t, cx))
            .children(self.render_room_edit(t, window, cx))
            .children(self.render_groups_edit(t, window, cx))
            .children(self.timeline.as_ref().and_then(|timeline| timeline.render_viewer(t)))
            .children({
                #[cfg(target_os = "linux")]
                {
                    self.render_install_offer(t, cx)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    None::<gpui::AnyElement>
                }
            }))
            .into_any_element()
    }
}

/// First instance: listen for later launches. Otherwise deliver our link (or "raise") and exit.
/// Any socket trouble only costs the single-instance behaviour: the app starts normally.
#[cfg(all(target_os = "linux", feature = "live"))]
fn single_instance_gate(argv: &[String]) {
    use matrix_desktop::single_instance::{Claim, claim, default_socket_path, serve};
    let (rest, link) = matrix_desktop::take_matrix_link(argv.to_vec());
    // --self-test prints the build and must touch nothing (tests/self_test_cli.rs): no socket dir.
    if rest.first().is_some_and(|a| a == "--install-desktop-entry" || a == "--self-test")
        || matrix_desktop::is_account_window(&rest)
    {
        return;
    }
    let link = matrix_desktop::single_instance::outgoing(link);
    let Some(path) = default_socket_path() else {
        return eprintln!("Gnobbi: single instance off (no private state directory)");
    };
    match claim(&path, &link) {
        Ok(Claim::First(listener)) => serve(listener),
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

/// Windows twin of the gate above: named mutex + pipe, same behaviour.
#[cfg(all(windows, feature = "live"))]
fn single_instance_gate(argv: &[String]) {
    use matrix_desktop::windows_link::{Claim, MAX_LINK, claim, register_for_current_user};
    let (rest, link) = matrix_desktop::take_matrix_link(argv.to_vec());
    if matrix_desktop::hostile_argv(&rest, &link) {
        eprintln!("Gnobbi: link with extra arguments refused");
        std::process::exit(2);
    }
    if rest.first().is_some_and(|a| a == "--self-test") || matrix_desktop::is_account_window(&rest) {
        return;
    }
    let link = link.filter(|l| l.len() <= MAX_LINK && !l.contains('\n')).unwrap_or_default();
    match claim(&link) {
        Ok(Claim::First) => register_for_current_user(),
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

fn main() {
    // Before the marker and the UI: the update helper is this same exe in another mode.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = matrix_desktop::update::helper::dispatch(&argv) {
        std::process::exit(code);
    }
    // A second launch hands its link to the live instance and exits before anything else starts.
    #[cfg(all(any(target_os = "linux", windows), feature = "live"))]
    single_instance_gate(&argv);
    matrix_desktop::update::startup::run_at_start();
    #[cfg(all(feature = "perf-probe", feature = "live", target_os = "macos"))]
    if std::env::var_os("P115B_NATIVE").is_some() {
        return login_field::mac_clipboard_probe::run();
    }

    #[cfg(feature = "live")]
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }
    // Background update check; a no-op (no thread, no network) outside AppImage/Windows portable.
    #[cfg(feature = "live")]
    if let Some(dirs) = matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok()) {
        matrix_desktop::update::scheduler::start(dirs, matrix_desktop::update::shared_state(), Default::default());
    }
    let (args, link) = matrix_desktop::take_matrix_link(std::env::args().skip(1).collect());
    // Kept in memory for the link handler; an invalid one is dropped (only the error kind is logged).
    #[cfg(feature = "live")]
    let mut start_link = link.and_then(|raw| {
        matrix_engine::matrix_link::parse_matrix_link(&raw)
            .map_err(|error| eprintln!("Gnobbi: ignored matrix link ({error:?})"))
            .ok()
    });
    #[cfg(not(feature = "live"))]
    let mut start_link: Option<matrix_domain::MatrixLink> = link.and(None);
    if args.first().is_some_and(|a| a == "--install-desktop-entry") {
        // Saved icon choice; missing or invalid preferences fall back to the default.
        let icon = matrix_desktop::app_dirs(matrix_desktop::Platform::Linux, |k| {
            std::env::var(k).ok()
        })
        .map(|d| matrix_desktop::Preferences::load(&d).icon)
        .unwrap_or_default();
        // An explicit command is its own confirmation: replace a different copy.
        let done = matrix_desktop::desktop_entry::install_for_current_user(icon, true);
        if let Err(e) = done {
            eprintln!("install-desktop-entry: {e}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    let args = {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned));
        let home = std::env::var("HOME").ok();
        exe.as_deref()
            .and_then(|exe| matrix_desktop::bundle_launch_args(exe, home.as_deref(), &args))
            .unwrap_or(args)
    };
    // Without `live` there is no connected sign-in to open: the bare launch stays the offline fixture.
    // Windows tester builds launch like Linux: a double-click opens the sign-in, not the offline demo.
    #[cfg(all(
        any(target_os = "linux", target_os = "windows"),
        feature = "live",
        not(test)
    ))]
    let args = matrix_desktop::linux_launch_args(&args);
    let args = match matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| {
        std::env::var(k).ok()
    }) {
        Some(dirs)
            if args.first().is_some_and(|a| a == "--signin")
                && args.get(1).is_none_or(|a| a == "--ca-file") =>
        {
            if let Err(error) = dirs.ensure() {
                eprintln!("Gnobbi: cannot create {}: {error}", dirs.data.display());
                std::process::exit(2);
            }
            let _ = matrix_desktop::create_private_dir(&dirs.data.join("profile"));
            // A pre-catalog install is listed in place; the profile folder does not move.
            let mut catalog = matrix_desktop::AccountCatalog::load(&dirs);
            if catalog.migrate_legacy(&dirs) {
                let _ = catalog.save(&dirs);
            }
            matrix_desktop::default_signin_args(&args, &dirs)
        }
        _ => args,
    };
    #[cfg(test)]
    let args = if navigation_resize_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || panel_opener_view::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || rail_drag_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || list_divider_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || verification_popup_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || drawer_active_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || rail_ring_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || switch_rows_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
    {
        vec![]
    } else {
        args
    };
    let signin_options = matrix_desktop::SignInOptions::parse(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let options =
        matrix_desktop::RestoreOptions::parse(if signin_options.is_some() { &[] } else { &args })
            .unwrap_or_else(|error| {
                eprintln!("{error}");
                std::process::exit(2);
            });
    let connected = options.is_some() || signin_options.is_some();
    let account_root = signin_options
        .as_ref()
        .map(|o| (o.root.clone(), o.ca_file.clone()))
        .or_else(|| {
            options
                .as_ref()
                .map(|o| (o.root.clone(), o.ca_file.clone()))
        });
    #[cfg(feature = "live")]
    let mut signin_ui = None;
    #[cfg(feature = "live")]
    let mut signin_vault = false;
    let account_id = options
        .as_ref()
        .map_or(AccountId::new(0), |options| options.account_id);
    let mut routes = matrix_desktop::AccountRoutes::default();
    #[cfg(feature = "live")]
    let mut signin_options = signin_options;
    #[cfg(feature = "live")]
    let worker = connected.then(|| {
        use std::io::{IsTerminal, Read};
        let (mut worker, route) = if let Some(options) = signin_options.take() {
            let trust = options
                .ca_file
                .map(|path| {
                    let mut pem = Vec::new();
                    std::fs::File::open(path)?
                        .take(1048577)
                        .read_to_end(&mut pem)?;
                    if pem.len() > 1048576 {
                        return Err(std::io::Error::other("trust root too large"));
                    }
                    Ok(pem)
                })
                .transpose()
                .unwrap_or_else(|_: std::io::Error| {
                    eprintln!("Could not read explicit trust root");
                    std::process::exit(2);
                });
            let vault = matrix_engine::signin::platform_source();
            signin_vault = vault.is_some();
            let (ui, pending) = match vault {
                Some(source) => {
                    matrix_engine::signin::interactive_with_source(options.root, trust, source)
                }
                None => matrix_engine::signin::interactive(options.root, trust),
            };
            signin_ui = Some(ui);
            matrix_engine::room_sync::RoomListWorker::spawn(move || async move {
                pending
                    .run()
                    .await
                    .map_err(|_| matrix_engine::auth::AuthError::Storage)
            })
        } else {
            let options = options.expect("restore options");
            // Explicit pipe-only unlock before the UI starts; no terminal echo or argv secret.
            let mut phrase = String::new();
            if std::io::stdin().is_terminal()
                || std::io::stdin()
                    .lock()
                    .take(4097)
                    .read_to_string(&mut phrase)
                    .is_err()
                || phrase.len() > 4096
            {
                eprintln!(
                    "Provide the store passphrase on a closed stdin pipe (maximum 4096 bytes)"
                );
                std::process::exit(2);
            }
            if phrase.ends_with('\n') {
                phrase.pop();
            }
            matrix_engine::room_sync::RoomListWorker::spawn(move || async move {
                use matrix_engine::{auth::AuthError, persistence::AccountStore};
                let store = AccountStore::open(&options.root, options.account_id)
                    .map_err(|_| AuthError::Storage)?;
                if let Some(path) = options.ca_file {
                    let mut pem = Vec::new();
                    std::fs::File::open(path)
                        .map_err(|_| AuthError::Transport)?
                        .take(1048577)
                        .read_to_end(&mut pem)
                        .map_err(|_| AuthError::Transport)?;
                    if pem.len() > 1048576 {
                        return Err(AuthError::Transport);
                    }
                    store.restore_with_trusted_root(&phrase, &pem).await
                } else {
                    store.restore(&phrase).await
                }
            })
        }
        .unwrap_or_else(|_| {
            eprintln!("Could not start Matrix backend worker");
            std::process::exit(1);
        });
        routes.events = Some(route);
        routes.media = worker.take_media_route();
        routes.verification = worker.take_verification_route();
        routes.operation = worker.take_operation_route();
        routes.send = worker.take_send_route();
        routes.send_control = worker.send_control();
        routes.history = worker.take_history_route();
        routes.avatars = worker.avatar_slot();
        worker
    });
    #[cfg(not(feature = "live"))]
    if connected {
        eprintln!("Live restore requires a Unix build with --features live");
        std::process::exit(2);
    }
    #[cfg(feature = "live")]
    let (mut background_workers, mut background, active_id) =
        account_switch::start_background(account_root.as_ref());
    #[cfg(not(feature = "live"))]
    let (mut background, active_id): (
        Vec<(AccountId, matrix_desktop::AccountState)>,
        Option<AccountId>,
    ) = (vec![], None);
    #[cfg(feature = "live")]
    let mut active_control = worker.as_ref().map(|w| w.control());
    #[cfg(not(feature = "live"))]
    let mut active_control: Option<matrix_engine::WorkerControl> = None;
    let notification_worker = connected
        .then(matrix_desktop::notifications::NotificationWorker::spawn)
        .and_then(Result::ok);
    let mut notification_handle = notification_worker.as_ref().map(|worker| worker.handle());
    // Registered on the Application, before it runs and so before any window: GPUI drops URLs that arrive
    // with no callback. The shell does not exist yet, so the URLs wait in OPENED_URLS for the 200 ms tick.
    let app = Application::new();
    app.on_open_urls(|urls| OPENED_URLS.lock().unwrap().extend(urls));
    app.run(move |cx: &mut App| {
        emoji_font::register(cx);
        #[cfg(target_os = "macos")]
        cx.set_menus(vec![
            gpui::Menu { name: "Gnobbi".into(), items: vec![] },
            editing_command::edit_menu(),
        ]);
        let bounds = Bounds::centered(None, size(px(LIGHT.width), px(LIGHT.height)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                window_min_size: Some(size(px(360.), px(480.))),
                ..matrix_desktop::window::window_options()
            },
            |window, cx| {
                matrix_desktop::window::refresh_drawable(window);
                let shell = cx.new(|cx| {
                    let search_focus = cx.focus_handle();
                    let mut rail_ui = rail_view::RailUi::new(cx);
                    let spaces_ui = spaces_view::SpacesUi::new(cx);
                    for focus in [&rail_ui.focus, &spaces_ui.focus] {
                        cx.on_blur(focus, window, |this: &mut Shell, _, _| {
                            if let Some(press) = &mut this.pending_sidebar_key { press.cancelled = true; }
                        }).detach();
                    }
                    search_focus.focus(window);
                    #[cfg(feature = "live")]
                    let signin = signin_ui.take().map(|route| cx.new(|cx| {
                        signin_view::SignInView::new(route, cx).with_vault(signin_vault)
                    }));
                    #[cfg(feature = "live")]
                    if let Some(signin) = &signin {
                        cx.observe(signin, |_, _, cx| cx.notify()).detach();
                    }
                    let sidebar = if connected {
                        Sidebar::connected(account_id)
                    } else {
                        Sidebar::demo(account_id)
                    };
                    let active_scope = sidebar
                        .selected()
                        .map(|room| room.scope.clone())
                        .unwrap_or_else(|| RoomScope {
                            account_id,
                            room_id: matrix_domain::RoomId::new("!not-selected:local.invalid"),
                        });
                    let prefs_dirs = matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok());
                    let prefs = prefs_dirs.as_ref().map(matrix_desktop::Preferences::load).unwrap_or_default();
                    rail_ui.layouts = prefs_dirs.as_ref().map(matrix_desktop::RailLayouts::load).unwrap_or_default();
                    matrix_desktop::set_accent(prefs.effective_accent());
                    matrix_desktop::set_font(prefs.font);
                    matrix_desktop::app_icon::apply(prefs.icon, window);
                    let system_dark = matches!(
                        window.appearance(),
                        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                    );
                    let theme = prefs.effective_theme(system_dark, matrix_desktop::local_hour());
                    let composer = cx.new(|cx| {
                        let mut editor = ComposerView::new(active_scope.clone(), cx);
                        editor.set_theme(theme.tokens(), cx);
                        editor.set_live(connected);
                        editor
                    });
                    cx.subscribe(
                        &composer,
                        |this: &mut Shell, _, submitted: &Submitted, cx| {
                            if this.connected {
                                #[cfg(feature = "live")]
                                if this.sidebar.selected().is_some() && this.routes.send.is_some()
                                {
                                    let transaction_id =
                                        matrix_engine::sending::fresh_transaction_id();
                                    this.composer.update(cx, |editor, cx| {
                                        editor.accept_live(
                                            &mut this.delivery.borrow_mut(),
                                            this.active_scope.clone(),
                                            transaction_id,
                                            cx,
                                        )
                                    });
                                    this.refresh_delivery(true);
                                }
                            } else if let Some(timeline) = &this.timeline {
                                timeline.submit_text(submitted.0.clone());
                            }
                            cx.notify();
                        },
                    )
                    .detach();
                    cx.subscribe_in(&composer, window, |this: &mut Shell, _, _: &OpenPoll, window, cx| {
                        this.open_poll_dialog(window, cx)
                    })
                    .detach();
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    cx.observe(&composer, |_, _, cx| cx.notify()).detach();
                    if routes.events.is_some() {
                        let executor = cx.background_executor().clone();
                        cx.spawn(async move |this, cx| {
                            loop {
                                executor.timer(std::time::Duration::from_millis(16)).await;
                                if this
                                    .update(cx, |this: &mut Shell, cx| {
                                        this.pump_parked();
                                        this.finish_removals(cx);
                                        #[cfg(feature = "live")]
                                        if let Some(signin) = &this.signin
                                            && let Some(account) = signin.update(cx, |view, cx| view.poll(cx)) {
                                            this.signin = None;
                                            this.refocus_list = true;
                                            this.sidebar = Sidebar::connected(account);
                                            this.active_scope.account_id = account;
                                            this.notifications = matrix_domain::notifications::NotificationPolicy::new(account);
                                            this.verification = matrix_desktop::DeviceVerification::new(account);
                                            this.composer.update(cx, |editor, cx| editor.switch_room(this.active_scope.clone(), cx));
                                            cx.notify();
                                        }
                                        let matrix_desktop::Drained { events, media, verifications, operations, deliveries, pages, closed } = this.routes.drain();
                                        let failed = this
                                            .notification_handle
                                            .as_ref()
                                            .is_none_or(|handle| handle.failed());
                                        let notification_changed =
                                            failed != this.notification_failed;
                                        this.notification_failed = failed;
                                        let changed = notification_changed
                                            || !events.is_empty()
                                            || !media.is_empty()
                                            || !verifications.is_empty()
                                            || !pages.is_empty()
                                            || !deliveries.is_empty()
                                            || !operations.is_empty()
                                            || closed;
                                        for result in operations {
                                            if let matrix_engine::operations::OperationResult::Members { scope, members, upload_limit } = result {
                                                if scope == this.active_scope {
                                                    this.room_members = members.as_ref().map_or_else(|_| Vec::new(), Clone::clone);
                                                    this.composer.update(cx, |editor, cx| {
                                                        editor.set_upload_limit(upload_limit);
                                                        if let Ok(members) = members {
                                                            editor.set_members(members, cx);
                                                        }
                                                    });
                                                }
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::ThreadSent { scope, root, result } = result {
                                                this.thread_sent(scope, root, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::Thread { tag, root, result } = result {
                                                this.thread_found(tag, root, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::ThreadRead { tag, root, result, .. } = result {
                                                this.thread_read_result(tag, root, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::Profile { user_id, result } = result {
                                                this.profile_found(user_id, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::Ignored { user_id, ignore, result } = result {
                                                this.ignored_set(user_id, ignore, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::MemberRights { scope, user_id, result } = result {
                                                if scope == this.active_scope {
                                                    this.rights_found(user_id, result, cx);
                                                }
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::Moderated { scope, user_id, action, result } = result {
                                                if scope == this.active_scope {
                                                    this.moderated(user_id, action, result, cx);
                                                }
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::TagsEdited { result } = result {
                                                this.groups_edited(result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::RoomEdited { result } = result {
                                                this.room_edited(result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::RoomSearch { state } = result {
                                                this.directory_result(state, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::ServerProbed { server, result } = result {
                                                this.server_probed(server, result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::MessageSearch { scope, state } = result {
                                                this.room_search_result(scope, state, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::UserSearch { state, all_available } = result {
                                                this.user_search_result(state, all_available, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::RoomStarted { result } = result {
                                                this.room_started(result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::InviteAnswered { .. } = result {
                                                this.invite_answered(result, cx);
                                                continue;
                                            }
                                            if let matrix_engine::operations::OperationResult::RoomLeft { scope, result } = result {
                                                this.sidebar.room_left(&scope, result);
                                                continue;
                                            }
                                            this.operations.borrow_mut().apply(result);
                                        }
                                        for event in media {
                                            if this.media.apply(event) {
                                                this.update_media_image(cx);
                                            }
                                        }
                                        for event in verifications {
                                            // Recovery imports the keys; the SDK re-decrypts the open room without any new event: read it again.
                                            if matches!(event, matrix_engine::verification::VerificationEvent::KeysComplete { .. })
                                                && let Some(scope) = this.sidebar.selected().map(|r| r.scope.clone())
                                            {
                                                this.history.activity(&scope);
                                            }
                                            this.verification.apply(event);
                                            this.record_profile();
                                        }
                                        for event in deliveries {
                                            this.delivery.borrow_mut().handle(event);
                                        }
                                        for event in events {
                                            match event {
                                                RoomListEvent::Alert(scope) => {
                                                    if let Some(alert) = this.notifications.alert(&scope)
                                                        && let Some(handle) = &this.notification_handle
                                                    {
                                                        handle.submit(Shell::alert_from(&this.accounts, this.active_id, alert));
                                                    }
                                                }
                                                RoomListEvent::Diff(diff) => {
                                                    if let matrix_engine::RoomListDiff::Remove(_) = &diff
                                                        && let Some(handle) = &this.notification_handle
                                                    {
                                                        handle.cancel_pending();
                                                    }
                                                    let anchor = this.reading_anchor();
                                                    let gone = matches!(&diff, matrix_engine::RoomListDiff::Remove(s) if s.account_id == this.active_scope.account_id && this.restore.lost(&s.room_id));
                                                    this.sidebar.apply_diff(diff);
                                                    if gone { this.sidebar.clear_selection(); }
                                                    this.reconcile_topic_links();
                                                    this.open_started_room(cx);
                                                    this.keep_reading_position(anchor);
                                                }
                                                RoomListEvent::TimelineActivity(scope) => {
                                                    this.read_dirty |= this.sidebar.selected().is_some_and(|r| r.scope == scope);
                                                    if let Some(p) = &mut this.thread { p.activity(&scope); }
                                                    this.history.activity(&scope)
                                                }
                                                RoomListEvent::Order(order) => {
                                                    let anchor = this.reading_anchor();
                                                    this.sidebar.set_order(order);
                                                    this.keep_reading_position(anchor);
                                                }
                                                RoomListEvent::Status(status) => {
                                                    if status
                                                        == RoomSyncStatus::AuthenticationFailed
                                                    {
                                                        this.notifications.set_enabled(false);
                                                        if let Some(handle) =
                                                            &this.notification_handle
                                                        {
                                                            handle.cancel_pending();
                                                        }
                                                    }
                                                    this.status = status
                                                }
                                            }
                                        }
                                        if closed
                                            && matches!(
                                                this.status,
                                                RoomSyncStatus::Connecting | RoomSyncStatus::Ready
                                            )
                                        {
                                            this.status = RoomSyncStatus::SyncFailed;
                                        }
                                        // After the diffs: the saved room is picked before any other opens.
                                        this.restore_last_room(cx);
                                        if changed {
                                            this.sync_room(cx);
                                            this.read_dirty |= !pages.is_empty();
                                            for result in pages {
                                                let live = this.history.is_live(&result.tag);
                                                let events = if live { this.history.apply_live(result) } else { this.history.apply(result) };
                                                if let Some(events) = events && let Some(timeline) = &this.timeline {
                                                    if live { timeline.append_server(events); } else { timeline.prepend_server(events); }
                                                }
                                            }
                                            this.refresh_delivery(false);
                                            if let Some(timeline) = &this.timeline {
                                                timeline
                                                    .project_operations(&this.operations.borrow());
                                            }
                                            cx.notify();
                                        }
                                        if this.pending_history.is_none() {
                                            this.pending_history = this.history.refresh();
                                        }
                                        if this.connected && this.pending_history.is_none()
                                            && let Some(timeline) = &this.timeline {
                                            this.pending_history = this.history.load_older_near_top(timeline.first_visible_row());
                                        }
                                        this.flush_history();
                                        if this.drive_inline_images() {
                                            cx.notify();
                                        }
                                        this.drive_saves(cx);
                                        if this.drive_audio() {
                                            cx.notify();
                                        }
                                        this.drive_recover_keys(cx);
                                        if this.drive_avatars(cx) {
                                            cx.notify();
                                        }
                                        if let Some((sender, _)) = &mut this.routes.media {
                                            this.media.flush(sender);
                                        }
                                        this.mark_open_room_read(cx);
                                        this.flush_operations();
                                        this.drive_thread(cx);
                                        // Repaint when a jump's flash runs out, with or without a search open.
                                        if this.timeline.as_ref().is_some_and(|t| t.flash_ended()) {
                                            cx.notify();
                                        }
                                        this.drive_profile(cx);
                                        this.drive_room_search(cx);
                                        this.drive_link_jump(cx);
                                        if let Some((sender, _)) = &mut this.routes.verification {
                                            this.verification.flush(sender);
                                        }
                                        if let Some((sender, _)) = &mut this.routes.send {
                                            this.delivery.borrow_mut().flush(sender);
                                        }
                                        closed
                                    })
                                    .unwrap_or(true)
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    #[cfg(feature = "perf-probe")]
                    if !connected
                        && let Ok(seconds) = std::env::var("MATRIX_DESKTOP_PERF_SCROLL")
                        && let Some(ticks) = match seconds.as_str() {
                            "3" => Some(180),
                            "12" => Some(720),
                            _ => None,
                        }
                    {
                        let executor = cx.background_executor().clone();
                        cx.spawn(async move |this, cx| {
                            let _ = this.update(cx, |this: &mut Shell, cx| {
                                if let Some(timeline) = &this.timeline {
                                    timeline.jump_to_boundary(false);
                                    cx.notify();
                                }
                            });
                            executor.timer(std::time::Duration::from_secs(3)).await;
                            eprintln!("Matrix performance scroll begin");
                            for _ in 0..ticks {
                                executor
                                    .timer(std::time::Duration::from_nanos(16_666_667))
                                    .await;
                                if this
                                    .update(cx, |this: &mut Shell, cx| {
                                        if let Some(timeline) = &this.timeline {
                                            timeline.scroll_performance_fixture();
                                            cx.notify();
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            eprintln!("Matrix performance scroll end");
                        })
                        .detach();
                    }
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    {
                        let executor = cx.background_executor().clone();
                        cx.spawn_in(window, async move |this, cx| {
                            loop {
                                executor.timer(std::time::Duration::from_millis(16)).await;
                                if this.update_in(cx, |this, window, cx| {
                                    Shell::drain_navigation_actions(this, window, cx);
                                }).is_err() {
                                    break;
                                }
                            }
                        }).detach();
                    }
                    cx.spawn_in(window, async move |this, cx| {
                        loop {
                            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                            if this.update_in(cx, |this, window, cx| this.sync_tray(window, cx)).is_err() {
                                break;
                            }
                        }
                    }).detach();
                    cx.observe_window_activation(window, |this: &mut Shell, window, _| {
                        if window.is_window_active() {
                            this.window_visible = true;
                        }
                    }).detach();
                    cx.spawn_in(window, async move |this, cx| {
                        loop {
                            cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                            if this.update_in(cx, |this, window, cx| {
                                if this.update_audio_progress() {
                                    cx.notify();
                                }
                                this.open_clicked_alert(window, cx);
                                #[cfg(all(any(target_os = "linux", windows), feature = "live"))]
                                for link in matrix_desktop::single_instance::take_incoming() {
                                    window.activate_window();
                                    if !link.is_empty() {
                                        this.open_matrix_link(&link, cx);
                                    }
                                }
                                #[cfg(feature = "live")]
                                for url in std::mem::take(&mut *OPENED_URLS.lock().unwrap()) {
                                    this.open_matrix_link(&url, cx);
                                }
                                this.drain_pending_link(window, cx);
                            }).is_err() {
                                break;
                            }
                        }
                    }).detach();
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(std::time::Duration::from_secs(60)).await;
                            if this.update(cx, |this, cx| this.apply_theme(cx)).is_err() {
                                break;
                            }
                        }
                    }).detach();
                    // Stamps in the room list change at local midnight: repaint then, no restart needed.
                    cx.spawn(async move |this, cx| {
                        loop {
                            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                            let wait = matrix_desktop::time_fmt::next_local_midnight_ms().map_or(3_600_000, |m| m.saturating_sub(now) + 1_000);
                            cx.background_executor().timer(std::time::Duration::from_millis(wait.min(3_600_000))).await;
                            if this.update(cx, |_, cx| cx.notify()).is_err() {
                                break;
                            }
                        }
                    }).detach();
                    cx.observe_window_appearance(window, |this: &mut Shell, window, cx| {
                        this.system_dark = matches!(
                            window.appearance(),
                            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                        );
                        this.apply_theme(cx);
                    }).detach();
                    cx.observe_window_bounds(window, |this: &mut Shell, window, cx| {
                        this.navigation_wide = window.viewport_size().width >= px(800.);
                        this.refresh_navigation_visibility();
                        this.reconcile_sidebar_focus(window, cx);
                    }).detach();
                    #[cfg(test)]
                    navigation_resize_test::install(window, cx);
                    #[cfg(test)]
                    panel_opener_view::install(window, cx);
                    #[cfg(test)]
                    sidebar_navigation_test::install(window, cx);
                    #[cfg(test)]
                    verification_popup_test::install(window, cx);
                    #[cfg(test)]
                    drawer_active_test::install(window, cx);
                    #[cfg(test)]
                    rail_ring_test::install(window, cx);
                    #[cfg(test)]
                    switch_rows_test::install(window, cx);
                    #[cfg(any(test, feature = "gui-fixture"))]
                    rail_drag_test::install(window, cx);
                    #[cfg(test)]
                    list_divider_test::install(window, cx);
                    Shell {
                        #[cfg(feature = "live")]
                        signin: signin.clone(),
                        pending_sidebar_key: None,
                        rail_ui,
                        opener_ui: panel_opener_view::OpenerUi::new(),
                        room_menu: None,
                        room_search_requests: 0,
                        room_search: None,
                        confirm_leave: None,
                        menu_open: false,
                        menu_focus: cx.focus_handle(),
                        drawer_focus: cx.focus_handle(),
                        settings_open: false,
                        settings_error: None,
                        about_open: false,
                        about_row: 0,
                        about_focus: cx.focus_handle(),
                        about_system: String::new(),
                        about_icon: None,
                        #[cfg(target_os = "linux")]
                        install_offer: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_offer_bounds: [None; 4],
                        new_room: None,
                        poll_dialog: None,
                        quick: None,
                        recents: prefs_dirs.as_ref().map(matrix_desktop::quick_search::RecentsStore::load).unwrap_or_default(),
                        room_edit: None,
                        pending_open: None,
                        restore: Default::default(),
                        groups_edit: None,
                        refocus_menu: false,
                        refocus_list: false,
                        refocus_join_field: false,
                        drawer_row: 0,
                        update_error: None,
                        update_confirmed: false,
                        settings_focus: cx.focus_handle(),
                        settings_row: 0,
                        accounts: prefs_dirs.as_ref().map(matrix_desktop::AccountCatalog::load).unwrap_or_default(),
                        account_root: account_root.clone(),
                        room_prefs: prefs_dirs.as_ref().map(matrix_desktop::RoomPrefs::load).unwrap_or_default(),
                        composer_focused: false,
                        focus_composer_on_restore: false,
                        prefs_dirs,
                        list_drag: None,
                        list_divider_focus: cx.focus_handle().tab_stop(true),
                        prefs,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        back_ui: compact_view::BackUi::new(cx),
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        back_focus: cx.focus_handle(),
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        actions_focus: cx.focus_handle(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        operation_ui: operation_panel_view::OperationUi::new(cx),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        accessibility: matrix_desktop::accessibility::LinuxAccessibility::new(
                            matrix_desktop::accessibility::NavigationSnapshot {
                                title: matrix_desktop::window::TITLE.into(),
                                new_chat: Default::default(),
                                room_search: Default::default(),
                                #[cfg(feature = "live")]
                                signin: signin.as_ref().map(|view| view.read(cx).semantic_snapshot(window, cx)),
                                openers: vec![],
                                back: None,
                                drawer: None,
        scrollbars: vec![],
        operation_panel: None,
                                spaces: None,
                navigation_revision: 0,
                                icon_choice: None,
                                disconnected: None,
                                new_room: None,
                                devices: Default::default(),
                                settings: None,
                                about: None,
                                details: Default::default(),
                                notifications: Default::default(),
                                dark: false,
                                visible: true,
                                query: String::new(),
                                search_focused: true,
                                search_bounds: None,
                                rooms: vec![],
                                history: None,
                                list_focused: false,
                                composer: None,
                            },
                        ),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_search_bounds: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_history_bounds: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        history_focus: cx.focus_handle(),
                        notification_focus: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_notification_bounds: [None; 4],
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_rooms: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_history: Default::default(),
                        notifications: matrix_domain::notifications::NotificationPolicy::new(
                            account_id,
                        ),
                        // Fixture and test windows must not register a tray on the desktop.
                        tray: if cfg!(any(test, feature = "gui-fixture", feature = "perf-probe")) {
                            None
                        } else {
                            matrix_desktop::tray::Tray::start()
                        },
                        window_visible: true,
                        notification_handle: notification_handle.take(),
                        notifications_open: false,
                        room_details_open: false,
                        room_members: Vec::new(),
                        profile: None,
                        profile_focus: 0,
                        profile_scroll: gpui::ScrollHandle::new(),
                        profile_copied: None,
                        profile_dm: false,
                        profile_failed: None,
                        profile_rights: None,
                        profile_confirm: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        details_focus: Default::default(),
                        details_zone: cx.focus_handle(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_details_bounds: [None; 8],
                        spaces_ui,
                        spaces_open: false,
                        navigation_wide: true,
                        notification_failed: false,
                        avatars: Default::default(),
                        media: matrix_desktop::LiveMedia::default(),
                        audio: None,
                        // Before `routes` is taken: the queue control belongs to the delivery too.
                        delivery: std::rc::Rc::new(std::cell::RefCell::new(
                            matrix_desktop::LiveDelivery::with_control(routes.send_control.clone()),
                        )),
                        routes: std::mem::take(&mut routes),
                        parked: background.drain(..).collect(),
                        control: active_control.take(),
                        confirm_removal: None,
                        switch_error: None,
                        pending_link: {
                            let mut queue = matrix_desktop::link_open::LinkQueue::default();
                            queue.extend(start_link.take());
                            queue
                        },
                        link_notice: None,
                        link_jump: None,
                        swallow_menu_key_up: false,
                        removals: vec![],
                        active_id,
                        verification: matrix_desktop::DeviceVerification::new(account_id),
                        verification_open: false,
                        delete_field: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        device_focus: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_device_bounds: [None; 2],
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        device_scroll: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_device_nodes: Default::default(),
                        operations: std::rc::Rc::default(),
                        pending_operation_command: None,
                        members_requested: None,
                        read_dirty: false,
                        confirm_redaction: None,
                        confirm_end: false,
                        connected,
                        status: RoomSyncStatus::Connecting,
                        outage: Default::default(),
                        sidebar,
                        composer_visit: composer.read(cx).visit.clone(),
                        composer,
                        timeline: (!connected)
                            .then(|| TimelineView::new(active_scope.clone(), cx.focus_handle())),
                        active_scope,
                        thread: None,
                        thread_last: None,
                        thread_resume: None,
                        saved_timelines: HashMap::new(),
                        history: HistorySession::new(),
                        pending_history: None,
                        compact_chat: false,
                        theme,
                        system_dark,
                        search_focus,
                        list_focus: cx.focus_handle(),
                        scroll: UniformListScrollHandle::new(),
                        room_scrollbar: Default::default(),
                        members_scroll: UniformListScrollHandle::new(),
                        members_scrollbar: Default::default(),
                        topic_scroll: UniformListScrollHandle::new(),
                        topic_scrollbar: Default::default(),
                        topic_links: Default::default(),
                        topic_confirm: Default::default(),
                        topic_pressed: Default::default(),
                        topic_hover: Default::default(),
                    }
                });
                let weak = shell.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.composer_focused = this.composer.read(cx).is_focused(window);
                        this.remember_last_room();
                    });
                    let hide = weak
                        .update(cx, |this, _| this.close_hides_to_tray())
                        .unwrap_or(false);
                    if hide {
                        window.hide_window();
                        let _ = weak.update(cx, |this, _| this.window_visible = false);
                    }
                    !hide
                });
                shell
            },
        )
        .expect("could not open desktop window");
        cx.on_window_closed(|cx| {
            // GPUI X11 holds its client borrow during WM_DELETE_WINDOW.
            // Quit on the next executor turn, after that callback has returned.
            cx.spawn(async move |cx| {
                let _ = cx.update(|cx| {
                    if cx.windows().is_empty() {
                        cx.quit();
                    }
                });
            })
            .detach();
        })
        .detach();
    });
    if let Some(worker) = notification_worker {
        let _ = worker.shutdown();
    }
    #[cfg(feature = "live")]
    for worker in background_workers.drain(..) {
        let _ = worker.shutdown();
    }
    #[cfg(feature = "live")]
    if let Some(worker) = worker
        && worker.shutdown().is_err()
    {
        eprintln!("Matrix backend shutdown failed");
        std::process::exit(1);
    }
}
