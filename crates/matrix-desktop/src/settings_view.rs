//! Settings inside the side drawer: switches (debug, theme, read receipts), saved to the preferences file.
use crate::*;

impl Shell {
    pub(crate) fn set_settings_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_open = open;
        if open {
            self.settings_focus.focus(window)
        } else {
            self.drawer_focus.focus(window)
        }
        cx.notify();
    }

    pub(crate) fn toggle_debug(&mut self, cx: &mut Context<Self>) {
        self.prefs.debug = !self.prefs.debug;
        self.save_prefs(cx);
    }

    pub(crate) fn choose_theme(&mut self, theme: matrix_desktop::Theme, cx: &mut Context<Self>) {
        self.set_theme(theme, cx);
        self.prefs.theme = Some(theme);
        self.save_prefs(cx);
    }

    pub(crate) fn toggle_read_receipts(&mut self, cx: &mut Context<Self>) {
        self.prefs.read_receipts = !self.prefs.read_receipts;
        self.save_prefs(cx);
    }

    fn save_prefs(&mut self, cx: &mut Context<Self>) {
        if let Some(dirs) = &self.prefs_dirs
            && let Err(error) = self.prefs.save(dirs)
        {
            eprintln!("Gnobbi: cannot save preferences: {error}");
        }
        cx.notify();
    }

    /// The drawer's account rows: none while offline; the shown account is marked current.
    pub(crate) fn drawer_accounts(&self) -> Vec<matrix_desktop::AccountRow> {
        if !self.connected {
            return vec![];
        }
        let shown = self.active_id.and_then(|id| self.accounts.get(id));
        let current = shown
            .and_then(|a| a.user_id.as_deref())
            .or_else(|| self.verification.profile().map(|(id, _)| id.as_str()));
        let mut rows =
            matrix_desktop::account_rows(&self.accounts, current, &self.account_counts());
        rows.retain(|a| !self.removals.iter().any(|r| r.id == a.id));
        rows
    }

    /// One row per catalog account (avatar, name, user ID); a dot marks the shown account and a
    /// click shows another one.
    fn render_account_list(
        &self,
        t: matrix_desktop::DesignTokens,
        ring: &dyn Fn(usize) -> gpui::Rgba,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        self.drawer_accounts()
            .into_iter()
            .enumerate()
            .map(|(index, a)| {
                let id = a.id;
                let switchable = self.can_switch_to(id);
                div()
                    .id(("menu-account", a.id.as_u64() as usize))
                    .h(px(52.))
                    .mx_2()
                    .px_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(ring(index))
                    .when(switchable, |d| {
                        d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_account(id, window, cx)
                            }))
                    })
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(matrix_desktop::AVATAR_FALLBACK))
                            .text_color(rgb(t.avatar_text))
                            .child(a.initials.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_color(rgb(t.text))
                                    .child(a.name.clone()),
                            )
                            .when(a.name != a.user_id, |d| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(t.secondary))
                                        .child(a.user_id.clone()),
                                )
                            }),
                    )
                    .when(a.unread > 0, |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .rounded_full()
                                .px_2()
                                .text_size(px(11.))
                                .bg(rgb(if a.mentions > 0 { t.accent } else { t.disabled }))
                                .text_color(rgb(if a.mentions > 0 { t.on_accent } else { t.text }))
                                .child(if a.unread > 99 {
                                    "99+".to_string()
                                } else {
                                    a.unread.to_string()
                                }),
                        )
                    })
                    .child(
                        div()
                            .id(("menu-account-remove", a.id.as_u64() as usize))
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(18.))
                            .text_color(rgb(t.secondary))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.confirm_removal = Some(id);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .size(px(8.))
                            .flex_shrink_0()
                            .rounded_full()
                            .when(a.current, |d| d.bg(rgb(t.accent))),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(crate) fn render_drawer_body(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let row = |id: &'static str| {
            div()
                .id(id)
                .h(px(44.))
                .mx_2()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .rounded_lg()
                .cursor_pointer()
        };
        if !self.settings_open {
            let n = self.drawer_accounts().len();
            let ring = |i: usize| {
                rgb(
                    if self.drawer_row == i && self.drawer_focus.is_focused(window) {
                        t.accent
                    } else {
                        t.surface
                    },
                )
            };
            return div()
                .mt_2()
                .children(self.render_account_list(t, &ring, cx))
                .child(
                    row("menu-settings")
                        .border_1()
                        .border_color(ring(n))
                        .text_color(rgb(t.text))
                        .child("Impostazioni")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_settings_open(true, window, cx)
                        })),
                )
                .when(self.connected, |d| {
                    d.child(
                        row("menu-devices")
                            .border_1()
                            .border_color(ring(n + 1))
                            .text_color(rgb(t.text))
                            .child("Dispositivi")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_from_drawer(window, cx, Shell::open_devices_panel)
                            })),
                    )
                    .child(
                        row("menu-notifications")
                            .border_1()
                            .border_color(ring(n + 2))
                            .text_color(rgb(t.text))
                            .child("Notifiche")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_from_drawer(window, cx, Shell::open_notifications_panel)
                            })),
                    )
                    .child(
                        row("menu-add-account")
                            .border_1()
                            .border_color(ring(n + 3))
                            .text_color(rgb(t.text))
                            .child("Aggiungi account")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_account(window, cx)),
                            ),
                    )
                    .child(
                        row("menu-new-chat")
                            .border_1()
                            .border_color(ring(n + 4))
                            .text_color(rgb(t.text))
                            .child("Nuova chat")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_room(true, window, cx)
                            })),
                    )
                    .child(
                        row("menu-new-room")
                            .border_1()
                            .border_color(ring(n + 5))
                            .text_color(rgb(t.text))
                            .child("Nuova stanza")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_room(false, window, cx)
                            })),
                    )
                })
                .into_any_element();
        }
        let switch = |on: bool| {
            div()
                .w(px(36.))
                .h(px(20.))
                .rounded_full()
                .flex()
                .items_center()
                .px(px(2.))
                .bg(rgb(if on { t.accent } else { t.disabled }))
                .when(on, |d| d.justify_end())
                .child(
                    div()
                        .w(px(16.))
                        .h(px(16.))
                        .rounded_full()
                        .bg(rgb(t.on_accent)),
                )
        };
        div()
            .child(
                div()
                    .h(px(t.header))
                    .px_4()
                    .flex()
                    .items_center()
                    .text_color(rgb(t.text))
                    .child("Impostazioni"),
            )
            .child(
                row("settings-debug")
                    .track_focus(&self.settings_focus)
                    .text_color(rgb(t.text))
                    .border_1()
                    .border_color(rgb(if self.settings_focus.is_focused(window) {
                        t.accent
                    } else {
                        t.surface
                    }))
                    .child("Modalità debug")
                    .child(switch(self.prefs.debug))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_debug(cx))),
            )
            .children(matrix_desktop::Theme::ALL.map(|theme| {
                row([
                    "settings-theme-classic",
                    "settings-theme-day",
                    "settings-theme-dark",
                    "settings-theme-night",
                ][theme as usize])
                .text_color(rgb(t.text))
                .child(theme.label())
                .child(switch(self.theme == theme))
                .on_click(cx.listener(move |this, _, _, cx| this.choose_theme(theme, cx)))
            }))
            .child(
                div()
                    .h(px(t.header))
                    .px_4()
                    .flex()
                    .items_center()
                    .text_color(rgb(t.text))
                    .child("Privacy e sicurezza"),
            )
            .child(
                row("settings-read-receipts")
                    .text_color(rgb(t.text))
                    .child("Conferme di lettura")
                    .child(switch(self.prefs.read_receipts))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_read_receipts(cx))),
            )
            .into_any_element()
    }
}
