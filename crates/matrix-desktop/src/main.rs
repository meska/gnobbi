mod compact_view;
mod composer_view;
mod editing_command;
mod filter_view;
#[cfg(all(feature = "live", unix))]
mod login_field;
mod media_view;
#[cfg(test)]
mod navigation_resize_test;
mod notification_view;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
mod operation_panel_view;
mod panel_opener_view;
mod rail_view;
mod ring_seam;
mod room_details_view;
mod scrollbar_view;
#[cfg(test)]
mod sidebar_navigation_test;
#[cfg(all(feature = "live", unix))]
mod signin_view;
mod spaces_view;
mod timeline_view;
mod verification_view;
use composer_view::{ComposerView, Submitted};

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyDownEvent, ScrollStrategy, TitlebarOptions,
    UniformListScrollHandle, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
    uniform_list,
};
use matrix_desktop::{
    DARK, DEMO_NOTICE, HistorySession, HistoryStatus, LIGHT, Navigation, Sidebar,
};
use matrix_domain::{AccountId, RoomScope};
use matrix_engine::{RoomListEvent, RoomSyncStatus};
use std::collections::HashMap;
use timeline_view::TimelineView;

// GPUI owns projections only. Optional live restore and sync run on a worker.
struct Shell {
    #[cfg(all(feature = "live", unix))]
    signin: Option<gpui::Entity<signin_view::SignInView>>,
    rail_ui: rail_view::RailUi,
    opener_ui: panel_opener_view::OpenerUi,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    back_ui: compact_view::BackUi,
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    back_focus: FocusHandle,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    operation_ui: operation_panel_view::OperationUi,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    accessibility: matrix_desktop::accessibility::LinuxAccessibility,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_search_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_history_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    history_focus: FocusHandle,
    pending_sidebar_key: Option<filter_view::SidebarPress>,
    unread_focus: FocusHandle,
    theme_focus: FocusHandle,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_theme_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_unread_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    notification_focus: matrix_desktop::accessibility::notifications::NotificationControl,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_notification_bounds: [Option<accesskit::Rect>; 4],
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_rooms: matrix_desktop::accessibility::RoomNodes,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_history: matrix_desktop::accessibility::HistoryNodes,
    room_details_open: bool,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    details_focus: matrix_desktop::accessibility::details::DetailsControl,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_details_bounds: [Option<accesskit::Rect>; 2],
    spaces_ui: spaces_view::SpacesUi,
    spaces_open: bool,
    navigation_wide: bool,
    notifications: matrix_domain::notifications::NotificationPolicy,
    notification_handle: Option<matrix_desktop::notifications::NotificationHandle>,
    notifications_open: bool,
    notification_failed: bool,
    media: matrix_desktop::LiveMedia,
    media_target: Option<matrix_domain::ScopedTimelineId>,
    media_image: Option<std::sync::Arc<gpui::RenderImage>>,
    media_commands: Option<matrix_engine::Sender<matrix_engine::media::MediaCommand>>,
    verification: matrix_desktop::DeviceVerification,
    verification_open: bool,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    device_focus: matrix_desktop::accessibility::devices::DeviceControl,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_device_bounds: [Option<accesskit::Rect>; 6],
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    device_scroll: verification_view::DeviceScroll,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    a11y_device_nodes: std::cell::RefCell<matrix_desktop::accessibility::devices::DeviceNodes>,
    verification_commands:
        Option<matrix_engine::Sender<matrix_engine::verification::VerificationCommand>>,
    delivery: std::rc::Rc<std::cell::RefCell<matrix_desktop::LiveDelivery>>,
    send_commands: Option<matrix_engine::Sender<matrix_engine::sending::SendRequest>>,
    operations: std::rc::Rc<std::cell::RefCell<matrix_desktop::LiveOperations>>,
    operation_commands: Option<matrix_engine::Sender<matrix_engine::operations::OperationCommand>>,
    pending_operation_command: Option<matrix_engine::operations::OperationCommand>,
    confirm_redaction: Option<matrix_domain::ScopedTimelineId>,
    connected: bool,
    status: RoomSyncStatus,
    sidebar: Sidebar,
    active_scope: RoomScope,
    composer: gpui::Entity<ComposerView>,
    composer_visit: std::rc::Rc<composer_view::SendVisit>,
    timeline: Option<TimelineView>,
    saved_timelines: HashMap<RoomScope, TimelineView>,
    history: HistorySession,
    history_commands: Option<matrix_engine::Sender<matrix_engine::history::HistoryRequest>>,
    pending_history: Option<matrix_engine::history::HistoryRequest>,
    compact_chat: bool,
    dark: bool,
    search_focus: FocusHandle,
    list_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    room_scrollbar: scrollbar_view::Scrollbar,
}

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn drain_navigation_actions(this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
                                    for _ in 0..8 {
                                        let Some(action) = this.accessibility.take_action() else {
                                            break;
                                        };
                                        #[cfg(all(feature = "live", unix))]
                                        if this.dispatch_signin_action(&action, window, cx) {
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::CompactBack(action) = action {
                                            this.apply_semantic_back(action, window, cx);
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::OperationPanel(action) = action {
                                            this.apply_semantic_operation_panel(action, window, cx);
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::Spaces(action) = action {
                                            this.apply_semantic_spaces(action,window,cx);
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::Filter(action) = action {
                                            this.apply_semantic_filter(action, window, cx);
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
                                        if let matrix_desktop::accessibility::NavigationAction::Notifications(action) = action {
                                            this.apply_semantic_notification(action, window, cx);
                                            continue;
                                        }
                                        if this.notifications_open || this.verification_open || this.room_details_open {
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::LoadServerHistory(tag) = &action {
                                            if this.connected && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && let Some(request) = this.history.load_older_if_current(tag) {
                                                this.pending_history = Some(request);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::FocusServerHistory(tag) = &action {
                                            if this.connected && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && !tag.is_cancelled()
                                                && this.history.pagination_tag().is_some_and(|current| current.generation() == tag.generation() && current.scope() == tag.scope()) {
                                                this.history_focus.focus(window);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::LinkConfirmation(intent) = &action {
                                            if intent.request.source().event().scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == intent.request.source().event().scope)
                                                && let Some(timeline) = &this.timeline {
                                                timeline.apply_confirmation(intent, window, cx);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::OfflineDelivery(intent) = &action {
                                            if !this.connected && intent.target.event().scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == intent.target.event().scope)
                                                && let Some(timeline) = &this.timeline {
                                                timeline.apply_offline_delivery(intent, window);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::CopyEventId(intent) = &action {
                                            if intent.target.event().scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == intent.target.event().scope)
                                                && let Some(timeline) = &this.timeline {
                                                timeline.apply_event_id_copy(intent, window, cx);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::MessageLink(intent) = &action {
                                            if intent.target.event().scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == intent.target.event().scope)
                                                && let Some(timeline) = &this.timeline {
                                                timeline.apply_link(intent, window);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::MessageSelection(intent) = &action {
                                            if intent.target.scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == intent.target.scope)
                                                && let Some(timeline) = &this.timeline {
                                                timeline.apply_selection(intent, window, cx);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::FocusMessageRetry(id) = &action {

                                            if this.connected && id.scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == id.scope)
                                                && this.timeline.as_ref().is_some_and(|timeline| timeline.semantic_snapshot(&mut this.a11y_history, window).allows_retry(id) && timeline.focus_semantic_retry(id,window)) {
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::OpenMessageActions(id) | matrix_desktop::accessibility::NavigationAction::FocusMessageActions(id) = &action {
                                            if this.connected && id.scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == id.scope)
                                                && this.timeline.as_ref().is_some_and(|timeline| timeline.semantic_snapshot(&mut this.a11y_history, window).allows_actions(id)) {
                                                if matches!(action, matrix_desktop::accessibility::NavigationAction::OpenMessageActions(_)) {
                                                    this.operations.borrow_mut().select(id.clone());
                                                } else if let Some(timeline) = &this.timeline {
                                                    timeline.focus_semantic_actions(id, window);
                                                }
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::RetryMessage(id) = &action {
                                            if this.connected && id.scope == this.active_scope
                                                && (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.sidebar.selected().is_some_and(|room| room.scope == id.scope)
                                                && this.timeline.as_ref().is_some_and(|timeline| timeline.semantic_snapshot(&mut this.a11y_history, window).allows_retry(id))
                                                && let matrix_domain::TimelineId::Pending(transaction) = &id.id
                                                && this.delivery.borrow_mut().retry(&id.scope, transaction) {
                                                this.refresh_delivery(false);
                                                cx.notify();
                                            }
                                            continue;
                                        }
                                        if let matrix_desktop::accessibility::NavigationAction::FocusMessage(id) = &action {
                                            if (window.viewport_size().width >= px(800.) || this.compact_chat)
                                                && this.timeline.as_ref().is_some_and(|timeline| timeline.focus_semantic_message(id, window)) {
                                                cx.notify();
                                                let entity = cx.entity().clone();
                                                window.on_next_frame(move |window, _| {
                                                    window.on_next_frame(move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                                                });
                                            }
                                            continue;
                                        }
                                        if this.dispatch_sidebar_action(&action, window, cx) { continue; }
                                        if this.dispatch_opener_action(&action, window, cx) { continue; }
                                        if this.dispatch_composer_action(&action, window, cx) { continue; }
                                        if let matrix_desktop::accessibility::NavigationAction::Scrollbar(action) = action {
                                            this.apply_scrollbar(action, window);
                                            cx.notify();
                                            continue;
                                        }
                                        if window.viewport_size().width < px(800.) && this.compact_chat {
                                            continue;
                                        }
                                        match action {
                                            matrix_desktop::accessibility::NavigationAction::PanelOpener(_) => unreachable!(),
                                            #[cfg(all(feature = "live", unix))]
                                            matrix_desktop::accessibility::NavigationAction::SignIn(_) => (),
                                            matrix_desktop::accessibility::NavigationAction::CopyEventId(_) | matrix_desktop::accessibility::NavigationAction::OfflineDelivery(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::MessageSelection(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::CompactBack(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::MessageLink(_) | matrix_desktop::accessibility::NavigationAction::LinkConfirmation(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Scrollbar(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::OperationPanel(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Spaces(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Filter(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Devices(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::LoadServerHistory(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusServerHistory(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Notifications(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::Details(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusComposer | matrix_desktop::accessibility::NavigationAction::FocusSend { .. } | matrix_desktop::accessibility::NavigationAction::SubmitComposer { .. } => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusMessage(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusMessageRetry(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::RetryMessage(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusMessageActions(_) | matrix_desktop::accessibility::NavigationAction::OpenMessageActions(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::ToggleTheme(_) | matrix_desktop::accessibility::NavigationAction::FocusTheme(_) => unreachable!(),
                                            matrix_desktop::accessibility::NavigationAction::FocusSearch => {
                                                this.search_focus.focus(window);
                                                cx.notify();
                                            }
                                            matrix_desktop::accessibility::NavigationAction::FocusRoom(intent) => this.apply_semantic_room(intent, true, window, cx),
                                            matrix_desktop::accessibility::NavigationAction::ActivateRoom(intent) => this.apply_semantic_room(intent, false, window, cx),
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

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[0].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.dark = !self.dark;
        let theme = if self.dark { DARK } else { LIGHT };
        self.composer
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        cx.notify();
    }

    fn navigation_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
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
            self.set_room_details_open(
                !self.room_details_open && self.sidebar.selected().is_some(),
            );
            self.list_focus.focus(window);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.room_details_open {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if matches!(key.key.as_str(), "tab" | "space" | "enter") {
                use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
                let view = self.semantic_details(window);
                if let DetailsView::Open(panel) = &view {
                    let action = if key.key == "tab" {
                        Some(DetailsAction::Focus {
                            scope: panel.scope.clone(),
                            control: panel.next_focus(),
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
            }
            if key.modifiers.control && key.modifiers.shift && key.key == "d" {
                self.toggle_theme(cx);
            }
            cx.notify();
            cx.stop_propagation();
            return;
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
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if key.key == "tab" {
                use matrix_desktop::accessibility::notifications::{
                    NotificationAction, NotificationView,
                };
                let view = self.semantic_notifications(window);
                if let NotificationView::Open(snapshot) = &view {
                    self.apply_semantic_notification(
                        NotificationAction::Focus(snapshot.next_focus(key.modifiers.shift)),
                        window,
                        cx,
                    );
                }
                cx.stop_propagation();
                return;
            }
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if matches!(key.key.as_str(), "space" | "enter") {
                let view = self.semantic_notifications(window);
                if let matrix_desktop::accessibility::notifications::NotificationView::Open(
                    snapshot,
                ) = &view
                    && let Some(action) = view.action(&accesskit::ActionRequest {
                        action: accesskit::Action::Click,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: snapshot.focus.id(),
                        data: None,
                    })
                {
                    self.apply_semantic_notification(action, window, cx);
                }
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
            if key.modifiers.control && key.modifiers.shift && key.key == "d" {
                self.toggle_theme(cx);
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
                    self.a11y_device_bounds = [None; 6];
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
            if key.modifiers.control && key.modifiers.shift && key.key == "d" {
                self.toggle_theme(cx);
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if key.modifiers.control && key.modifiers.shift && key.key == "d" {
            self.toggle_theme(cx);
            cx.stop_propagation();
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
        if !key.modifiers.control && !key.modifiers.alt && !key.modifiers.platform {
            if self.theme_focus.is_focused(window) {
                if key.key == "tab" {
                    if key.modifiers.shift {
                        self.list_focus.focus(window);
                    } else if self.semantic_unread(window).is_some() {
                        self.unread_focus.focus(window);
                    } else {
                        self.search_focus.focus(window);
                    }
                    cx.notify();
                    cx.stop_propagation();
                }
                // Space/Enter activate once via GPUI's native Keyboard ClickEvent.
                return;
            }
            if key.key == "tab"
                && key.modifiers.shift
                && self.search_focus.is_focused(window)
                && self.semantic_unread(window).is_none()
            {
                self.theme_focus.focus(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.semantic_unread(window).is_some()
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
        {
            if key.key == "tab" && key.modifiers.shift && self.search_focus.is_focused(window) {
                self.unread_focus.focus(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.unread_focus.is_focused(window) {
                if key.key == "tab" {
                    if key.modifiers.shift {
                        self.theme_focus.focus(window);
                    } else {
                        self.search_focus.focus(window);
                    }
                } else {
                    // GPUI emits one Keyboard ClickEvent on Space/Enter release.
                    return;
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
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
                NavigationAction::LinkConfirmation(intent) => {
                    timeline.apply_confirmation(&intent, window, cx);
                }
                NavigationAction::MessageLink(intent) => {
                    timeline.apply_link(&intent, window);
                }
                NavigationAction::MessageSelection(intent) => {
                    timeline.apply_selection(&intent, window, cx);
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
                self.media_image = None;
                self.operations.borrow_mut().clear_selection();
                self.confirm_redaction = None;
                self.active_scope = request.tag.scope().clone();
                self.composer.update(cx, |editor, cx| {
                    editor.switch_room(self.active_scope.clone(), cx)
                });
                self.timeline = Some(TimelineView::connected(
                    self.active_scope.clone(),
                    cx.focus_handle(),
                ));
                self.pending_history = Some(request);
                self.refresh_delivery(false);
            } else if scope.is_none() {
                self.set_room_details_open(false);
                self.media.clear();
                self.media_image = None;
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
            self.active_scope = scope;
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let searching = self.search_focus.is_focused(window);
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.list_focus.is_focused(window)
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && !key.modifiers.shift
            && let Some(intent) = self.a11y_rooms.activation_key(&self.sidebar, &key.key)
        {
            self.apply_semantic_room(intent, false, window, cx);
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
        if self.composer.read(cx).is_focused(window) && !(key.modifiers.control && key.key == "f") {
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
            if let Some(position) = self.sidebar.navigate(direction) {
                self.scroll.scroll_to_item(position, ScrollStrategy::Top);
            }
            self.sync_room(cx);
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

    fn flush_operations(&mut self) {
        use matrix_engine::{
            TrySendError,
            operations::{OperationCommand, OperationError, OperationResult},
        };
        if self.pending_operation_command.is_none() {
            self.pending_operation_command = self.operations.borrow_mut().take_command();
        }
        if let Some(command) = self.pending_operation_command.take()
            && let Some(sender) = &mut self.operation_commands
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
                    };
                    self.operations.borrow_mut().apply(result);
                }
            }
        }
    }

    fn render_operations(&mut self, _window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        use matrix_engine::operations::OperationError;
        let actions = self.operations.borrow();
        if self.media_target.as_ref() != actions.selection() {
            self.media.clear();
            self.media_image = None;
            self.media_target = None;
        }
        let selected = actions.selection().is_some();
        let busy = actions.is_pending();
        let permissions = actions.details().map(|d| d.permissions).unwrap_or_default();
        let error = actions.error();
        if self.confirm_redaction.as_ref() != actions.selection() {
            self.confirm_redaction = None;
        }
        let confirming = self.confirm_redaction.is_some();
        let notice = actions.notice(confirming);
        drop(actions);
        if !selected && !busy && error.is_none() {
            return div().into_any_element();
        }
        let t = if self.dark { DARK } else { LIGHT };
        let probe = std::env::var("MATRIX_DESKTOP_TIMELINE_PROBE").as_deref() == Ok("1");
        let controls = if confirming {
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
                ("edit", "Modifica", permissions.edit && !busy),
                ("redact", "Elimina", permissions.redact && !busy),
                ("react", "+1 / togli", permissions.react && !busy),
                ("retry", "Riprova", error == Some(OperationError::Transport)),
                ("close", "Chiudi", true),
                ("preview", "Anteprima PNG", selected),
            ]
        };
        div().flex_shrink_0().px_4().py_2().bg(rgb(t.surface)).border_t_1().border_color(rgb(t.selection))
            .child(div().text_size(px(11.)).text_color(rgb(t.secondary)).whitespace_normal().child(notice))
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
                            "close" => {
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                let target = this.operations.borrow().selection().cloned();
                                this.operations.borrow_mut().clear_selection();
                                this.confirm_redaction = None;
                                this.media.clear();
                                this.media_image = None;
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                if let Some(target) = target && let Some(timeline) = &this.timeline {
                                    // Retiring the focused mouse button must not orphan
                                    // keyboard dispatch (Ctrl+F/search after dismissal).
                                    timeline.focus_semantic_actions(&target, _window);
                                }
                            },
                            "preview" => {
                                if let Some(id) = this.operations.borrow().selection()
                                    && let matrix_domain::TimelineId::Server(event) = &id.id {
                                    this.media_image = None;
                                    this.media.request(id.scope.clone(), event.clone());
                                    this.media_target = Some(id.clone());
                                }
                            },
                            "redact" => {
                                let target = this.operations.borrow().selection().cloned();
                                if let Some(target) = target {
                                    this.operations.borrow_mut().select(target.clone());
                                    this.confirm_redaction = Some(target);
                                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                    this.operation_ui.focus.focus(_window);
                                }
                            },
                            "cancel-redact" => this.confirm_redaction = None,
                            "retry" => { this.operations.borrow_mut().retry(); },
                            _ => {
                                #[cfg(feature="live")]
                                {
                                    let transaction = matrix_engine::sending::fresh_transaction_id();
                                    match name {
                                        "reply" | "edit" => this.composer.update(cx, |editor,cx| editor.accept_operation(&mut this.operations.borrow_mut(), name == "edit", transaction, cx)),
                                        "react" => { this.operations.borrow_mut().toggle_reaction("+1", transaction); },
                                        "confirm-redact" => {
                                            if this.confirm_redaction.as_ref() == this.operations.borrow().selection() { this.operations.borrow_mut().submit(matrix_engine::operations::Operation::Redact, transaction); }
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
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let d = d.when(name == "close" || name == "cancel-redact", |d| d.track_focus(&self.operation_ui.focus)
                    .when(self.operation_ui.focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                    .children(self.operation_control_observer("close", cx)))
                    .when(name == "preview" && enabled, |d| d.track_focus(&self.operation_ui.preview_focus)
                        .when(self.operation_ui.preview_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("preview", cx)))
                    .when(name == "reply" && enabled, |d| d.track_focus(&self.operation_ui.reply_focus)
                        .when(self.operation_ui.reply_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("reply", cx)))
                    .when(name == "edit" && enabled, |d| d.track_focus(&self.operation_ui.edit_focus)
                        .when(self.operation_ui.edit_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("edit", cx)))
                    .when(name == "react" && enabled, |d| d.track_focus(&self.operation_ui.react_focus)
                        .when(self.operation_ui.react_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("react", cx)))
                    .when((name == "redact" || name == "confirm-redact") && enabled, |d| d.track_focus(&self.operation_ui.redact_focus)
                        .when(self.operation_ui.redact_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer(name, cx)))
                    .when(name == "retry" && enabled, |d| d.track_focus(&self.operation_ui.retry_focus)
                        .when(self.operation_ui.retry_focus.is_focused(_window), |d| d.border_1().border_color(rgb(t.accent)))
                        .children(self.operation_control_observer("retry", cx)));
                d
            }))).into_any_element()
    }

    fn flush_history(&mut self) {
        if let Some(request) = self.pending_history.take()
            && let Some(sender) = &mut self.history_commands
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
        #[cfg(all(feature = "live", unix))]
        if let Some(signin) = &self.signin {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            self.accessibility.update(
                matrix_desktop::accessibility::NavigationSnapshot::for_signin(
                    signin.read(cx).semantic_snapshot(window, cx),
                ),
                window.is_window_active(),
            );
            return div().size_full().child(signin.clone()).into_any_element();
        }
        let compact = window.viewport_size().width < px(800.);
        #[cfg(test)]
        if navigation_resize_test::FROZEN.load(std::sync::atomic::Ordering::SeqCst) {
            return div().size_full().into_any_element();
        }
        self.navigation_wide = !compact;
        self.refresh_navigation_visibility();
        self.reconcile_sidebar_focus(window, cx);
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
        self.accessibility.update(
            matrix_desktop::accessibility::NavigationSnapshot {
                #[cfg(all(feature = "live", unix))]
                signin: None,
                openers: self.semantic_openers(window),
                back: self.semantic_back(window),
                scrollbars: self.semantic_scrollbars(window),
                operation_panel: self.semantic_operation_panel(window),
                spaces: self.semantic_spaces(window),
                navigation_revision: self.sidebar.navigation_revision(),
                theme_focused: self.theme_focus.is_focused(window),
                theme_bounds: self.a11y_theme_bounds,
                unread_filter: self.semantic_unread(window),
                devices: self.semantic_devices(window),
                details: self.semantic_details(window),
                notifications: self.semantic_notifications(window),
                dark: self.dark,
                visible: !self.notifications_open
                    && !self.verification_open
                    && (window.viewport_size().width >= px(800.) || !self.compact_chat),
                query: self.sidebar.query().to_owned(),
                search_focused: self.search_focus.is_focused(window),
                search_bounds: self.a11y_search_bounds,
                rooms: self.a11y_rooms.project(&self.sidebar),
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
        if self.notifications_open {
            return self.render_notifications(cx);
        }
        if self.verification_open {
            return self.render_verification(cx);
        }
        let t = if self.dark { DARK } else { LIGHT };
        let selected = self.sidebar.selected();
        let header_name = selected.map_or("Nessuna stanza".to_owned(), |room| room.name.clone());
        let header_initials = selected.map_or(String::new(), |room| room.initials.clone());
        let header_color = selected.map_or(t.disabled, |room| room.color);
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
        window.set_window_title(&format!(
            "{} — Gnobbi — {} ({}/{})",
            header_name,
            connection_label,
            self.sidebar.visible_indices().len(),
            self.sidebar.rooms().len()
        ));
        div()
            .track_focus(&self.list_focus)
            .relative()
            .capture_key_down(cx.listener({
                let opener_keys=self.opener_key_callback();
                move |this,event,window,cx| {
                    if this.sidebar_activation_down(event,window,cx) || opener_keys(this,event,window,cx) {cx.stop_propagation();}
                    else {this.navigation_key(event,window,cx);}
                }
            }))
            .capture_key_up(cx.listener(Self::sidebar_activation_up))
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .size_full()
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .font_family("DejaVu Sans")
            .text_size(px(14.))
            .when(!compact, |d| d.child(self.render_rail(window, cx)))
            .when(!compact || !self.compact_chat, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .w(if compact {
                            window.viewport_size().width
                        } else {
                            px(t.sidebar)
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
                                .px_5()
                                .gap_3()
                                .child(
                                    div()
                                        .text_size(px(18.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("Gnobbi"),
                                )
                                .child(
                                    div()
                                        .id("theme-toggle")
                                        .relative()
                                        .children(self.theme_observer(cx))
                                .children({
                                    #[cfg(test)] { sidebar_navigation_test::observe("theme".into()) }
                                    #[cfg(not(test))] { None::<gpui::AnyElement> }
                                })
                                        .map(|d| self.track_theme_focus(d, window))
                                        .px_2()
                                        .py_2()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .text_color(rgb(t.accent))
                                        .child(if self.dark { "Chiaro" } else { "Scuro" })
                                        .on_click({
                                            let callback = self.sidebar_pointer_callback(false);
                                            #[cfg(test)]
                                            let callback = sidebar_navigation_test::capture_pointer(false, callback);
                                            cx.listener(move |this, _, window, cx| callback(this, window, cx))
                                        }),
                                ),
                        )
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
                                .border_color(rgb(if self.search_focus.is_focused(window) {
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
                                    "Cerca conversazioni · Esc cancella".to_owned()
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
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                let focused = this.a11y_rooms.focused(&this.sidebar).map_or(selected, |target| *target == scope);
                                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                                let focused = selected;
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
                                            .border_l_2()
                                            .border_color(rgb(
                                                if focused && this.list_focus.is_focused(window) {
                                                    t.accent
                                                } else {
                                                    background
                                                },
                                            ))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.navigation_wide = window.viewport_size().width >= px(800.);
                                                this.refresh_navigation_visibility();
                                                if !source_visit(&this.sidebar) { return; }
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                let Some(scope) = callback(&this.a11y_rooms, &this.sidebar) else { return; };
                                                if !this.sidebar.select(&scope) { return; }
                                                this.sync_room(cx);
                                                this.set_compact_chat(true);
                                                this.list_focus.focus(window);
                                                cx.notify();
                                            }))
                                            .flex()
                                            .items_center()
                                            .h(px(68.))
                                            .flex_shrink_0()
                                            .px_4()
                                            .gap_3()
                                            .bg(rgb(background))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .size(px(46.))
                                                    .flex_shrink_0()
                                                    .rounded_full()
                                                    .bg(rgb(room.color))
                                                    .text_color(rgb(t.avatar_text))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(room.initials.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .justify_between()
                                                            .gap_2()
                                                            .child(
                                                                div()
                                                                    .font_weight(
                                                                        gpui::FontWeight::SEMIBOLD,
                                                                    )
                                                                    .child(room.name.clone()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(px(11.))
                                                                    .text_color(rgb(t.secondary))
                                                                    .when(this.connected && room.unread > 0, |d| d.px_2().rounded_full().bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                                                                    .child(if this.connected && room.unread > 0 { room.unread.to_string() } else { room.time.to_owned() }),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .text_color(rgb(t.secondary))
                                                            .overflow_hidden()
                                                            .child(room.preview.clone()),
                                                    ),
                                            )
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
                        .child(
                            div()
                                .px_4()
                                .py_4()
                                .border_t_1()
                                .border_color(rgb(t.border))
                                .text_size(px(11.))
                                .text_color(rgb(t.secondary))
                                .flex().items_center().justify_between()
                                .when(self.connected, |d| d.flex_wrap().gap_2())
                                .child(div().when(self.connected, |d| d.w_full()).child(format!(
                                    "{}/{} conversazioni · {}",
                                    self.sidebar.visible_indices().len(), self.sidebar.rooms().len(),
                                    if self.connected { connection_label } else { "fixture offline" }
                                )))
                                .when(self.connected, |d| d.child(self.render_root_opener(matrix_desktop::panel_openers::Control::Devices, window, cx)))
                                .when(self.connected, |d| d.child(self.render_root_opener(matrix_desktop::panel_openers::Control::Notifications, window, cx)))
                                .when(self.connected, |d| d.child(self.render_unread(window, cx))),
                        ),
                )
            })
            .when(!compact || self.compact_chat, |d| {
                d.child(
                    div()
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
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(40.))
                                            .rounded_full()
                                            .bg(rgb(header_color))
                                            .text_color(rgb(t.avatar_text))
                                            .child(header_initials),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .flex_1().min_w_0().overflow_hidden()
                                        .child(
                                            div()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(header_name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(t.secondary))
                                                .child(if self.connected { "Matrix · storico, invio e ricezione live" } else { DEMO_NOTICE }),
                                        ),
                                )
                                .when(self.connected && self.sidebar.selected().is_some(), |d| d.child(self.render_root_opener(matrix_desktop::panel_openers::Control::Info, window, cx))),
                        )
                        .when(self.connected, |d| d.child(div().id("load-server-history").relative().h(px(32.)).flex_shrink_0().px_6().flex().items_center().bg(rgb(t.surface)).text_size(px(12.)).text_color(rgb(t.accent)).cursor_pointer()
                            .map(|d| {
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                {
                                    let entity = cx.entity();
                                    d.track_focus(&self.history_focus).border_1().border_color(rgb(if self.history_focus.is_focused(window) { t.accent } else { t.surface })).child(gpui::canvas(|_,_,_| (), move |bounds,_,window,cx| {
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
                            .child(match self.history.status() {
                                HistoryStatus::Empty => "Seleziona una stanza",
                                HistoryStatus::Loading => "Caricamento storico…",
                                HistoryStatus::More => "Carica precedenti · Ctrl+PageUp",
                                HistoryStatus::Start => "Inizio dello storico raggiunto",
                                HistoryStatus::Failed => "Storico non disponibile · clic per riprovare",
                            }).on_click(cx.listener(|this, _, _, cx| { this.request_older(); cx.notify(); }))))
                        .when_some(self.timeline.as_ref(), |d, timeline| d.child(timeline.render(t, window, self.connected.then(|| self.delivery.clone()), self.connected.then(|| self.operations.clone()))))
                        .when(self.connected && self.timeline.is_none(), |d| d.child(div().flex_1().bg(rgb(t.timeline)).p_6().child("Nessuna stanza selezionata. Nessun messaggio demo.")))
                        .when(self.connected, |d| d.child(self.render_operations(window, cx)))
                        .when(self.connected, |d| d.child(self.render_media()))
                        .when(!self.connected || self.sidebar.selected().is_some(), |d| d.child(self.composer.clone())),
                )
            })
            .when(self.room_details_open, |d| d.child(self.render_room_details(window, cx)))
            .into_any_element()
    }
}

fn main() {
    #[cfg(all(feature = "perf-probe", feature = "live", target_os = "macos"))]
    if std::env::var_os("P115B_NATIVE").is_some() {
        return login_field::mac_clipboard_probe::run();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "macos")]
    let args = {
        let exe = std::env::current_exe().ok().and_then(|path| path.to_str().map(str::to_owned));
        let home = std::env::var("HOME").ok();
        let bundled = exe
            .as_deref()
            .and_then(|exe| matrix_desktop::bundle_launch_args(exe, home.as_deref(), &args));
        if let Some(profile) = bundled.as_ref().and_then(|args| args.get(1)) {
            let _ = matrix_desktop::create_private_dir(std::path::Path::new(profile));
        }
        bundled.unwrap_or(args)
    };
    #[cfg(all(target_os = "linux", not(test)))]
    let args = matrix_desktop::linux_launch_args(&args);
    let legacy_offer = (args.first().is_some_and(|a| a == "--signin") && args.get(1).is_none_or(|a| a == "--ca-file"))
        .then(|| {
            let dirs = matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok())?;
            let legacy = matrix_desktop::legacy_profile::legacy_path(std::path::Path::new(&std::env::var("HOME").ok()?));
            let renamed = matrix_desktop::legacy_profile::renamed_path(&dirs.data);
            [Some(legacy), renamed]
                .into_iter()
                .flatten()
                .find(|old| matrix_desktop::legacy_profile::offered(old, &dirs.data))
                .map(|old| (old, dirs.data))
        })
        .flatten();
    // Old profile folders whose vault keys may still be saved under the previous app name.
    let vault_candidates = (args.first().is_some_and(|a| a == "--signin") && args.get(1).is_none_or(|a| a == "--ca-file"))
        .then(|| {
            let dirs = matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok())?;
            let home = std::env::var("HOME").ok()?;
            let old = [Some(matrix_desktop::legacy_profile::legacy_path(std::path::Path::new(&home))), matrix_desktop::legacy_profile::renamed_path(&dirs.data)];
            Some((old.into_iter().flatten().collect::<Vec<_>>(), dirs.data.join("profile")))
        })
        .flatten();
    let args = match matrix_desktop::app_dirs(matrix_desktop::Platform::CURRENT, |k| std::env::var(k).ok()) {
        Some(dirs) if args.first().is_some_and(|a| a == "--signin") && args.get(1).is_none_or(|a| a == "--ca-file") => {
            #[cfg(unix)]
            if let Err(error) = dirs.ensure() {
                eprintln!("Gnobbi: cannot create {}: {error}", dirs.data.display());
                std::process::exit(2);
            }
            #[cfg(unix)]
            let _ = matrix_desktop::create_private_dir(&dirs.data.join("profile"));
            matrix_desktop::default_signin_args(&args, &dirs)
        }
        _ => args,
    };
    #[cfg(test)]
    let args = if navigation_resize_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || panel_opener_view::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
        || sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
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
    #[cfg(all(feature = "live", unix))]
    let mut signin_ui = None;
    let mut signin_vault = false;
    let account_id = options
        .as_ref()
        .map_or(AccountId::new(0), |options| options.account_id);
    let mut receiver: Option<matrix_engine::Receiver<RoomListEvent>> = None;
    let mut media_commands = None;
    let mut media_events: Option<matrix_engine::Receiver<matrix_engine::media::MediaEvent>> = None;
    let mut history_commands = None;
    let mut send_commands = None;
    let mut operation_commands = None;
    let mut operation_events = None;
    let mut verification_commands = None;
    let mut verification_events: Option<
        matrix_engine::Receiver<matrix_engine::verification::VerificationEvent>,
    > = None;
    let mut send_events: Option<matrix_engine::Receiver<matrix_engine::sending::SendResult>> = None;
    let mut history_events: Option<matrix_engine::Receiver<matrix_engine::history::HistoryResult>> =
        None;
    #[cfg(all(feature = "live", unix))]
    let mut signin_options = signin_options;
    #[cfg(all(feature = "live", unix))]
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
                Some(source) => matrix_engine::signin::interactive_with_source(options.root, trust, source),
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
        receiver = Some(route);
        if let Some((commands, events)) = worker.take_media_route() {
            media_commands = Some(commands);
            media_events = Some(events);
        }
        if let Some((commands, events)) = worker.take_verification_route() {
            verification_commands = Some(commands);
            verification_events = Some(events);
        }
        if let Some((commands, events)) = worker.take_operation_route() {
            operation_commands = Some(commands);
            operation_events = Some(events);
        }
        if let Some((commands, events)) = worker.take_send_route() {
            send_commands = Some(commands);
            send_events = Some(events);
        }
        if let Some((commands, events)) = worker.take_history_route() {
            history_commands = Some(commands);
            history_events = Some(events);
        }
        worker
    });
    #[cfg(not(all(feature = "live", unix)))]
    if connected {
        eprintln!("Live restore requires a Unix build with --features live");
        std::process::exit(2);
    }
    let notification_worker = connected
        .then(matrix_desktop::notifications::NotificationWorker::spawn)
        .and_then(Result::ok);
    let mut notification_handle = notification_worker.as_ref().map(|worker| worker.handle());
    Application::new().run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        cx.set_menus(vec![
            gpui::Menu { name: "Gnobbi".into(), items: vec![] },
            editing_command::edit_menu(),
        ]);
        let bounds = Bounds::centered(None, size(px(LIGHT.width), px(LIGHT.height)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Gnobbi — demo offline".into()),
                    ..Default::default()
                }),
                focus: false,
                window_min_size: Some(size(px(360.), px(480.))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let search_focus = cx.focus_handle();
                    let theme_focus = cx.focus_handle();
                    let unread_focus = cx.focus_handle();
                    let rail_ui = rail_view::RailUi::new(cx);
                    let spaces_ui = spaces_view::SpacesUi::new(cx);
                    for focus in [&theme_focus, &unread_focus, &rail_ui.focus, &spaces_ui.focus] {
                        cx.on_blur(focus, window, |this: &mut Shell, _, _| {
                            if let Some(press) = &mut this.pending_sidebar_key { press.cancelled = true; }
                        }).detach();
                    }
                    search_focus.focus(window);
                    #[cfg(all(feature = "live", unix))]
                    let signin = signin_ui.take().map(|route| cx.new(|cx| {
                        let mut view = signin_view::SignInView::new(route, cx).with_vault(signin_vault).with_legacy(legacy_offer.clone());
                        if let Some((candidates, profile)) = &vault_candidates { view.check_vault_copy(candidates.clone(), profile.clone(), cx); }
                        view
                    }));
                    #[cfg(all(feature = "live", unix))]
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
                    let composer = cx.new(|cx| {
                        let mut editor = ComposerView::new(active_scope.clone(), cx);
                        editor.set_live(connected);
                        editor
                    });
                    cx.subscribe(
                        &composer,
                        |this: &mut Shell, _, submitted: &Submitted, cx| {
                            if this.connected {
                                #[cfg(feature = "live")]
                                if this.sidebar.selected().is_some() && this.send_commands.is_some()
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
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    cx.observe(&composer, |_, _, cx| cx.notify()).detach();
                    if let Some(mut route) = receiver.take() {
                        let mut media_route = media_events.take();
                        let mut verification_route = verification_events.take();
                        let mut history_route = history_events.take();
                        let mut send_route = send_events.take();
                        let mut operation_route: Option<
                            matrix_engine::Receiver<matrix_engine::operations::OperationResult>,
                        > = operation_events.take();
                        let executor = cx.background_executor().clone();
                        cx.spawn(async move |this, cx| {
                            loop {
                                executor.timer(std::time::Duration::from_millis(16)).await;
                                let mut events = Vec::new();
                                let mut pages = Vec::new();
                                let mut deliveries = Vec::new();
                                let mut operations = Vec::new();
                                let mut verifications = Vec::new();
                                let mut media = Vec::new();
                                if let Some(route) = &mut media_route {
                                    for _ in 0..2 {
                                        match route.try_recv() {
                                            Ok(event) => media.push(event),
                                            Err(_) => break,
                                        }
                                    }
                                }
                                if let Some(route) = &mut verification_route {
                                    for _ in 0..8 {
                                        match route.try_recv() {
                                            Ok(event) => verifications.push(event),
                                            Err(_) => break,
                                        }
                                    }
                                }
                                if let Some(route) = &mut operation_route {
                                    for _ in 0..8 {
                                        match route.try_recv() {
                                            Ok(result) => operations.push(result),
                                            Err(_) => break,
                                        }
                                    }
                                }
                                if let Some(route) = &mut send_route {
                                    for _ in 0..8 {
                                        match route.try_recv() {
                                            Ok(result) => deliveries.push(result),
                                            Err(_) => break,
                                        }
                                    }
                                }
                                if let Some(route) = &mut history_route {
                                    for _ in 0..8 {
                                        match route.try_recv() {
                                            Ok(page) => pages.push(page),
                                            Err(_) => break,
                                        }
                                    }
                                }
                                let mut closed = false;
                                for _ in 0..32 {
                                    match route.try_recv() {
                                        Ok(event) => events.push(event),
                                        Err(matrix_engine::TryReceiveError::Closed) => {
                                            closed = true;
                                            break;
                                        }
                                        Err(_) => break,
                                    }
                                }
                                if this
                                    .update(cx, |this: &mut Shell, cx| {
                                        #[cfg(all(feature = "live", unix))]
                                        if let Some(signin) = &this.signin
                                            && let Some(account) = signin.update(cx, |view, cx| view.poll(cx)) {
                                            this.signin = None;
                                            this.sidebar = Sidebar::connected(account);
                                            this.active_scope.account_id = account;
                                            this.notifications = matrix_domain::notifications::NotificationPolicy::new(account);
                                            this.verification = matrix_desktop::DeviceVerification::new(account);
                                            this.composer.update(cx, |editor, cx| editor.switch_room(this.active_scope.clone(), cx));
                                            cx.notify();
                                        }
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
                                            this.operations.borrow_mut().apply(result);
                                        }
                                        for event in media {
                                            if this.media.apply(event) {
                                                this.update_media_image();
                                            }
                                        }
                                        for event in verifications {
                                            this.verification.apply(event);
                                        }
                                        for result in deliveries {
                                            this.delivery.borrow_mut().apply(result);
                                        }
                                        for event in events {
                                            match event {
                                                RoomListEvent::Diff(diff) => {
                                                    match &diff {
                                                        matrix_engine::RoomListDiff::Upsert(
                                                            room,
                                                        ) => {
                                                            if let Some(alert) = this
                                                                .notifications
                                                                .update(&room.scope, room.unread)
                                                                && let Some(handle) =
                                                                    &this.notification_handle
                                                            {
                                                                handle.submit(alert);
                                                            }
                                                        }
                                                        matrix_engine::RoomListDiff::Remove(
                                                            scope,
                                                        ) => {
                                                            this.notifications.remove(scope);
                                                            if let Some(handle) =
                                                                &this.notification_handle
                                                            {
                                                                handle.cancel_pending();
                                                            }
                                                        }
                                                    }
                                                    this.sidebar.apply_diff(diff);
                                                }
                                                RoomListEvent::TimelineActivity(scope) => this.history.activity(&scope),
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
                                        if changed {
                                            this.sync_room(cx);
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
                                        this.flush_history();
                                        if let Some(sender) = &mut this.media_commands {
                                            this.media.flush(sender);
                                        }
                                        this.flush_operations();
                                        if let Some(sender) = &mut this.verification_commands {
                                            this.verification.flush(sender);
                                        }
                                        if let Some(sender) = &mut this.send_commands {
                                            this.delivery.borrow_mut().flush(sender);
                                        }
                                    })
                                    .is_err()
                                    || closed
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
                    Shell {
                        #[cfg(all(feature = "live", unix))]
                        signin: signin.clone(),
                        pending_sidebar_key: None,
                        rail_ui,
                        opener_ui: panel_opener_view::OpenerUi::new(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        back_ui: compact_view::BackUi::new(cx),
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        back_focus: cx.focus_handle(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        operation_ui: operation_panel_view::OperationUi::new(cx),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        accessibility: matrix_desktop::accessibility::LinuxAccessibility::new(
                            matrix_desktop::accessibility::NavigationSnapshot {
                                #[cfg(all(feature = "live", unix))]
                                signin: signin.as_ref().map(|view| view.read(cx).semantic_snapshot(window, cx)),
                                openers: vec![],
                                back: None,
        scrollbars: vec![],
        operation_panel: None,
                                spaces: None,
                navigation_revision: 0,
                theme_focused: false,
                                theme_bounds: None,
                                unread_filter: None,
                                devices: Default::default(),
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
                        unread_focus,
                        theme_focus,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_theme_bounds: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_unread_bounds: None,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
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
                        notification_handle: notification_handle.take(),
                        notifications_open: false,
                        room_details_open: false,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        details_focus: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_details_bounds: [None; 2],
                        spaces_ui,
                        spaces_open: false,
                        navigation_wide: true,
                        notification_failed: false,
                        media: matrix_desktop::LiveMedia::default(),
                        media_target: None,
                        media_image: None,
                        media_commands: media_commands.take(),
                        verification: matrix_desktop::DeviceVerification::new(account_id),
                        verification_open: false,
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        device_focus: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_device_bounds: [None; 6],
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        device_scroll: Default::default(),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        a11y_device_nodes: Default::default(),
                        verification_commands: verification_commands.take(),
                        delivery: std::rc::Rc::default(),
                        send_commands: send_commands.take(),
                        operations: std::rc::Rc::default(),
                        operation_commands: operation_commands.take(),
                        pending_operation_command: None,
                        confirm_redaction: None,
                        connected,
                        status: RoomSyncStatus::Connecting,
                        sidebar,
                        composer_visit: composer.read(cx).visit.clone(),
                        composer,
                        timeline: (!connected)
                            .then(|| TimelineView::new(active_scope.clone(), cx.focus_handle())),
                        active_scope,
                        saved_timelines: HashMap::new(),
                        history: HistorySession::new(),
                        history_commands: history_commands.take(),
                        pending_history: None,
                        compact_chat: false,
                        dark: false,
                        search_focus,
                        list_focus: cx.focus_handle(),
                        scroll: UniformListScrollHandle::new(),
                        room_scrollbar: Default::default(),
                    }
                })
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
    #[cfg(all(feature = "live", unix))]
    if let Some(worker) = worker
        && worker.shutdown().is_err()
    {
        eprintln!("Matrix backend shutdown failed");
        std::process::exit(1);
    }
}
