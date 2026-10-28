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
                    .map(|room| (room.scope.clone(), self.notifications.muted(&room.scope))),
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
            NotificationAction::SetEnabled(value) => self.notifications.set_enabled(value),
            NotificationAction::SetPreview(value) => self.notifications.set_count_preview(value),
            NotificationAction::SetMuted { scope, muted } => {
                self.notifications.set_muted(scope, muted)
            }
        }
        if policy_change && let Some(handle) = &self.notification_handle {
            handle.cancel_pending();
        }
        cx.notify();
    }

    pub(crate) fn change_notification_preference(&mut self, preference: u8) {
        if let Some(handle) = &self.notification_handle {
            handle.cancel_pending();
        }
        match preference {
            0 => self
                .notifications
                .set_enabled(!self.notifications.enabled()),
            1 => {
                if let Some(room) = self.sidebar.selected() {
                    let muted = self.notifications.muted(&room.scope);
                    self.notifications.set_muted(room.scope.clone(), !muted);
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
        let muted = selected.is_some_and(|room| self.notifications.muted(&room.scope));
        let room_available = selected.is_some();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_control = Some(self.notification_focus as usize);
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused_control: Option<usize> = None;
        div().size_full().flex().flex_col().bg(rgb(t.surface)).text_color(rgb(t.text))
            .font_family("DejaVu Sans").text_size(px(14.))
            .track_focus(&self.list_focus).capture_key_down(cx.listener(Self::navigation_key))
            .child(div().h(px(64.)).flex_shrink_0().px_4().flex().items_center().justify_between()
                .border_b_1().border_color(rgb(t.border))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Notifiche locali"))
                .child(div().id("notifications-close").relative().children(self.notification_observer(3, cx)).when(focused_control == Some(3), |d| d.border_1().border_color(rgb(t.accent))).cursor_pointer().text_color(rgb(t.accent)).child("Chiudi · Esc")
                    .on_click(cx.listener(|this, _, _, cx| { this.set_notifications_open(false); cx.notify(); }))))
            .child(div().id("notifications-content").flex_1().min_h_0().overflow_y_scroll().p_4().flex().flex_col().gap_3()
                .children([
                    (0u8, format!("Notifiche: {} · N", if enabled { "ON" } else { "OFF" }), true),
                    (1, format!("Stanza: {} · M", if !room_available { "nessuna selezione" } else if muted { "silenziata" } else { "attiva" }), room_available),
                    (2, format!("Conteggio: {} · P", if preview { "ON" } else { "OFF" }), true),
                ].into_iter().map(|(action, label, available)| {
                    div().id(("notification-setting", action as usize)).relative().children(self.notification_observer(action as usize, cx)).when(focused_control == Some(action as usize), |d| d.border_1().border_color(rgb(t.accent))).px_3().py_3().rounded_lg().bg(rgb(t.input))
                        .text_color(rgb(if available { t.accent } else { t.secondary }))
                        .when(available, |d| d.cursor_pointer()).child(label)
                        .on_click(cx.listener(move |this, _, _, cx| { if available { this.change_notification_preference(action); cx.notify(); } }))
                }))
                .child("Solo questa sessione: al riavvio notifiche e conteggio tornano OFF.")
                .child("Anteprima solo del conteggio. Mai testo, mittente, stanza, media o segreti nelle notifiche OS.")
                .child("Avvisi sugli aumenti dei non letti dopo la prima sync della stanza. Nessun push a client chiuso.")
                .child("Gli avvisi già consegnati al sistema non vengono ritirati cambiando preferenza.")
                .child(div().text_size(px(12.)).text_color(rgb(t.secondary)).child(if self.notification_failed {
                    "Servizio OS non disponibile o richiesta scaduta. Gli avvisi sono best effort."
                } else { "Linux: richiede notify-send e un servizio notifiche della sessione. Windows/macOS non implementati." }))
            ).into_any_element()
    }
}
