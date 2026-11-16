//! Session-local privacy preferences; no message content enters this screen.
use crate::*;

impl Shell {
    fn notification_observer(
        &self,
        _index: usize,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
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
                        if entity.read(cx).a11y_notification_bounds[_index] != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_notification_bounds[_index] = Some(rect);
                                    cx.notify();
                                })
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .into_any_element(),
            )
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_notifications(
        &self,
        window: &Window,
    ) -> matrix_desktop::accessibility::notifications::NotificationView {
        use matrix_desktop::accessibility::notifications::{
            NotificationSnapshot, NotificationView,
        };
        if !self.connected || self.verification_open || self.room_details_open {
            return NotificationView::Unavailable;
        }
        if self.notifications_open {
            NotificationView::Open(Box::new(NotificationSnapshot {
                bounds: self.a11y_notification_bounds,
                focus: self.notification_focus,
                enabled: self.notifications.enabled(),
                preview: self.notifications.count_preview(),
                selected: self
                    .sidebar
                    .selected()
                    .map(|room| (room.scope.clone(), room.info.muted)),
                room_name: self.sidebar.selected().map(|room| room.name.clone()).unwrap_or_default(),
                service_failed: self.notification_failed,
            }))
        } else if window.viewport_size().width >= px(800.) || !self.compact_chat {
            NotificationView::Closed
        } else {
            NotificationView::Unavailable
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_notification(
        &mut self,
        action: matrix_desktop::accessibility::notifications::NotificationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::notifications::NotificationAction;
        if !self.semantic_notifications(window).allows(&action) {
            return;
        }
        let policy_change = matches!(
            action,
            NotificationAction::SetEnabled(_)
                | NotificationAction::SetPreview(_)
                | NotificationAction::SetMuted { .. }
        );
        // Set the requested state explicitly. Duplicate/stale queued requests cannot toggle back.
        match action {
            NotificationAction::Focus(control) => {
                self.notification_focus = control;
                self.list_focus.focus(window);
            }
            NotificationAction::Open => {
                self.notification_focus = Default::default();
                self.set_notifications_open(true);
                self.list_focus.focus(window);
            }
            NotificationAction::Close => {
                self.set_notifications_open(false);
                self.list_focus.focus(window);
            }
            NotificationAction::SetEnabled(value) => self.set_notifications_enabled(value),
            NotificationAction::SetPreview(value) => self.notifications.set_count_preview(value),
            NotificationAction::SetMuted { scope, muted } => self.set_room_muted(scope, muted),
        }
        if policy_change && let Some(handle) = &self.notification_handle {
            handle.cancel_pending();
        }
        cx.notify();
    }

    /// «Silenzia»: the room push rule is written on the account; the next sync shows the result.
    fn set_room_muted(&mut self, scope: RoomScope, muted: bool) {
        if self.pending_operation_command.is_none() {
            self.pending_operation_command =
                Some(matrix_engine::operations::OperationCommand::SetMuted { scope, muted });
        }
    }

    /// The one «Notifiche» choice (tray and panel), global for all accounts: shown and parked
    /// policies follow it, and it is saved in settings.json.
    pub(crate) fn set_notifications_enabled(&mut self, on: bool) {
        self.notifications.set_enabled(on);
        for state in self.parked.values_mut() {
            state.notifications.set_enabled(on);
        }
        let Some(dirs) = &self.prefs_dirs else {
            self.prefs.notifications = on;
            return;
        };
        match matrix_desktop::Preferences::update(dirs, |p| p.notifications = on) {
            Ok(prefs) => self.prefs = prefs,
            Err(error) => {
                eprintln!("Gnobbi: cannot save preferences: {error}");
                self.settings_error = Some("Impossibile salvare le impostazioni. Riprova.");
            }
        }
    }

    pub(crate) fn change_notification_preference(&mut self, preference: u8) {
        if let Some(handle) = &self.notification_handle {
            handle.cancel_pending();
        }
        match preference {
            0 => self.set_notifications_enabled(!self.notifications.enabled()),
            1 => {
                if let Some(room) = self.sidebar.selected() {
                    let (scope, muted) = (room.scope.clone(), !room.info.muted);
                    self.set_room_muted(scope, muted);
                }
            }
            2 => self
                .notifications
                .set_count_preview(!self.notifications.count_preview()),
            _ => (),
        }
    }

    pub(crate) fn render_notifications(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let enabled = self.notifications.enabled();
        let preview = self.notifications.count_preview();
        let selected = self.sidebar.selected();
        let muted = selected.is_some_and(|room| room.info.muted);
        let room_available = selected.is_some();
        let room_name = selected.map(|room| room.name.clone());
        let focused_control = Some(self.notification_focus as usize);
        div().size_full().flex().flex_col().bg(rgb(t.surface)).text_color(rgb(t.text))
            .font_family(matrix_desktop::ui_font()).text_size(px(14.))
            .track_focus(&self.list_focus).capture_key_down(cx.listener(Self::navigation_key))
            .child(div().h(px(64.)).flex_shrink_0().px_4().flex().items_center().justify_between()
                .border_b_1().border_color(rgb(t.border))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Notifiche locali"))
                .child(div().id("notifications-close").relative().children(self.notification_observer(3, cx)).when(input_modality::only_keys(focused_control == Some(3), cx), |d| d.border_1().border_color(rgb(t.accent)))
                    .w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer()
                    .hover(|s| s.bg(rgb(t.border)))
                    .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                    .on_click(cx.listener(|this, _, _, cx| { this.set_notifications_open(false); cx.notify(); }))))
            .child(div().id("notifications-content").flex_1().min_h_0().overflow_y_scroll().p_4().flex().flex_col().gap_3()
                .children([
                    (0u8, "Notifiche".to_owned(), enabled, true),
                    (1, room_name.map_or_else(|| "Stanza".to_owned(), |n| format!("Stanza: {n}")), !muted, room_available),
                    (2, "Conteggio".to_owned(), preview, true),
                ].into_iter().map(|(action, label, on, available)| {
                    crate::switch_row::switch_row(("notification-setting", action as usize), label, on, available, input_modality::only_keys(focused_control == Some(action as usize), cx), t)
                        .relative().children(self.notification_observer(action as usize, cx))
                        .on_click(cx.listener(move |this, _, _, cx| { if available { this.change_notification_preference(action); cx.notify(); } }))
                }))
                .when(self.notification_failed, |d| d.child(div().text_size(px(12.)).text_color(rgb(t.secondary)).child("Servizio OS non disponibile o richiesta scaduta. Gli avvisi sono best effort.")))
            ).into_any_element()
    }
}
