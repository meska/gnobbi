//! Linux only: the «Aggiungi al menu» proposal after a successful start, and the Settings row.
use crate::*;
use matrix_desktop::install_offer::Choice;

impl Shell {
    /// The 280 px card covers the composer when the chat fills the window (<800 px): wait for the list.
    pub(crate) fn install_offer_visible(&self) -> bool {
        self.install_offer
            .visible(!self.navigation_wide && self.compact_chat)
    }

    /// Called every render: cheap until the one-time check runs.
    pub(crate) fn poll_install_offer(&mut self, window: &Window, cx: &mut Context<Self>) {
        let typing = self.composer.read(cx).is_focused(window);
        let dont_ask = self.prefs.desktop_entry_dont_ask;
        let ready = self.connected && self.status == RoomSyncStatus::Ready;
        self.install_offer.poll(ready, typing, || {
            matrix_desktop::desktop_entry::detect::should_prompt(
                &matrix_desktop::desktop_entry::detect::Env::from_process(),
                dont_ask,
            )
        });
    }

    pub(crate) fn choose_install(&mut self, choice: Choice, cx: &mut Context<Self>) {
        let icon = self.prefs.icon;
        let changed =
            self.install_offer
                .choose(choice, &mut self.prefs.desktop_entry_dont_ask, |replace| {
                    matrix_desktop::desktop_entry::install_for_current_user(icon, replace)
                });
        if changed {
            self.save_prefs(cx);
        }
        cx.notify();
    }

    /// Escape = «Non ora», only when no drawer/overlay is open: those close first. No Enter shortcut,
    /// it would fire on list/quick-search activations meant for something else.
    pub(crate) fn install_offer_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let other_layer = self.menu_open
            || self.settings_open
            || self.quick.is_some()
            || self.new_room.is_some()
            || self.room_edit.is_some()
            || self.groups_edit.is_some()
            || self.notifications_open
            || self.verification_open
            || self.room_details_open;
        if !self.install_offer_visible() || other_layer || key != "escape" {
            return false;
        }
        self.choose_install(Choice::Later, cx);
        true
    }

    #[cfg(feature = "linux-a11y")]
    pub(crate) fn publish_install_offer(&mut self) {
        let o = &self.install_offer;
        let b = self.a11y_offer_bounds;
        self.accessibility.update_install_offer(
            matrix_desktop::accessibility::install_offer::Snapshot {
                generation: o.generation,
                dialog: self.install_offer_visible().then(|| [b[0], b[1], b[2]]),
                settings_row: self.settings_open.then_some(b[3]),
                confirm: o.confirm,
                error: o.error.clone(),
            },
        );
    }

    /// Records the painted bounds of a control for the accessible tree (no-op without AT support).
    pub(crate) fn offer_observer(
        &self,
        _index: usize,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        #[cfg(feature = "linux-a11y")]
        {
            let entity = _cx.entity();
            Some(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let s = f64::from(window.scale_factor());
                        let f = |v: gpui::Pixels| f64::from(f32::from(v)) * s;
                        let rect = accesskit::Rect::new(
                            f(bounds.left()),
                            f(bounds.top()),
                            f(bounds.right()),
                            f(bounds.bottom()),
                        );
                        if entity.read(cx).a11y_offer_bounds[_index] != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_offer_bounds[_index] = Some(rect);
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
        #[cfg(not(feature = "linux-a11y"))]
        None
    }

    pub(crate) fn render_install_offer(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.install_offer_visible() {
            return None;
        }
        let confirm = self.install_offer.confirm;
        let (title, buttons) = if confirm {
            (
                "Sostituire gnobbi.AppImage?",
                vec![
                    (
                        "install-offer-replace",
                        "Sostituisci",
                        t.accent,
                        Choice::Replace,
                    ),
                    (
                        "install-offer-cancel",
                        "Annulla",
                        t.secondary,
                        Choice::Later,
                    ),
                ],
            )
        } else {
            (
                "Aggiungi al menu",
                vec![
                    (
                        "install-offer-add",
                        "Aggiungi al menu",
                        t.accent,
                        Choice::Add,
                    ),
                    ("install-offer-later", "Non ora", t.secondary, Choice::Later),
                    (
                        "install-offer-never",
                        "Non chiedere più",
                        t.secondary,
                        Choice::Never,
                    ),
                ],
            )
        };
        let mut button =
            |i: usize, id: &'static str, label: &'static str, color: u32, c: Choice| {
                div()
                    .id(id)
                    .relative()
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .tab_index(i as isize)
                    .text_color(rgb(color))
                    .child(label)
                    .children(self.offer_observer(i, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_install(c, cx)))
            };
        Some(
            div()
                .id("install-offer-layer")
                .absolute()
                .bottom_4()
                .left_4()
                .child(
                    div()
                        .id("install-offer")
                        // as wide as the room list (312 px less the margins): never over the composer
                        .w(gpui::px(280.))
                        .p_3()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(t.text))
                                .child(title),
                        )
                        .when_some(self.install_offer.error.clone(), |d, e| {
                            d.child(div().text_color(rgb(t.error)).child(e))
                        })
                        .child(
                            div().flex().flex_wrap().justify_end().gap_2().children(
                                buttons.into_iter().enumerate().map(
                                    |(i, (id, label, color, c))| button(i, id, label, color, c),
                                ),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Always-available Settings row; installs and clears «Non chiedere più».
    pub(crate) fn render_install_row(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .child(
                div()
                    .id("settings-add-to-menu")
                    .relative()
                    .h(px(44.))
                    .mx_2()
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_color(rgb(t.text))
                    .child("Aggiungi al menu")
                    .children(self.offer_observer(3, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.choose_install(Choice::Add, cx))),
            )
            .when_some(self.install_offer.error.clone(), |d, e| {
                d.child(div().mx_2().px_3().text_color(rgb(t.error)).child(e))
            })
            .into_any_element()
    }
}
