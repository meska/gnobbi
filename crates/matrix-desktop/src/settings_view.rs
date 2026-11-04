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
            self.settings_row = 0;
            self.settings_focus.focus(window)
        } else {
            self.drawer_focus.focus(window)
        }
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_settings(
        &self,
    ) -> Option<matrix_desktop::accessibility::settings::SettingsSnapshot> {
        self.settings_open.then(|| {
            matrix_desktop::accessibility::settings::SettingsSnapshot {
                focus: self.settings_row,
                on: [
                    self.prefs.debug,
                    self.prefs.read_receipts,
                    self.prefs.hide_redacted,
                    self.prefs.close_to_tray,
                ],
            }
        })
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_settings(
        &mut self,
        action: matrix_desktop::accessibility::settings::SettingsAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::settings::SettingsAction;
        if !self.settings_open {
            return;
        }
        match action {
            SettingsAction::Focus(row) => {
                self.settings_row = row;
                self.settings_focus.focus(window);
                cx.notify();
            }
            SettingsAction::Set(row, on) => {
                self.settings_row = row;
                if self.semantic_settings().is_some_and(|s| s.on[row] != on) {
                    self.toggle_setting(row, cx);
                }
            }
        }
    }

    /// Settings rows in screen order: 0 debug, 1 read receipts, 2 hide deleted messages, 3 close to tray.
    pub(crate) fn toggle_setting(&mut self, row: usize, cx: &mut Context<Self>) {
        match row {
            0 => self.toggle_debug(cx),
            1 => self.toggle_read_receipts(cx),
            2 => self.toggle_hide_redacted(cx),
            _ => self.toggle_close_to_tray(cx),
        }
    }

    pub(crate) fn toggle_debug(&mut self, cx: &mut Context<Self>) {
        self.prefs.debug = !self.prefs.debug;
        self.save_prefs(cx);
    }

    pub(crate) fn choose_theme(&mut self, theme: matrix_desktop::Theme, cx: &mut Context<Self>) {
        self.prefs.theme = Some(theme);
        self.apply_theme(cx);
        self.save_prefs(cx);
    }

    pub(crate) fn choose_auto_night(&mut self, auto: matrix_desktop::AutoNight, cx: &mut Context<Self>) {
        self.prefs.auto_night = auto;
        self.apply_theme(cx);
        self.save_prefs(cx);
    }

    pub(crate) fn choose_font(&mut self, font: matrix_desktop::FontChoice, cx: &mut Context<Self>) {
        self.prefs.font = font;
        matrix_desktop::set_font(font);
        self.save_prefs(cx);
    }

    pub(crate) fn choose_icon(&mut self, icon: matrix_desktop::IconChoice, window: &Window, cx: &mut Context<Self>) {
        self.prefs.icon = icon;
        matrix_desktop::app_icon::apply(icon, window);
        self.save_prefs(cx);
    }

    /// Three thumbnails in a row; the chosen one gets an accent border. No visible text.
    fn icon_row(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .px_4()
            .py_1()
            .flex()
            .gap_2()
            .children(matrix_desktop::IconChoice::ALL.map(|icon| {
                let picture = std::sync::Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    icon.thumbnail().to_vec(),
                ));
                div()
                    .id(("settings-icon", icon as usize))
                    .p_1()
                    .rounded_lg()
                    .border_2()
                    .border_color(rgb(if self.prefs.icon == icon { t.accent } else { t.surface }))
                    .cursor_pointer()
                    .child(gpui::img(picture).size(px(48.)))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose_icon(icon, window, cx)))
            }))
            .into_any_element()
    }

    /// One row of mutually exclusive text choices; the chosen one is semibold with an accent border.
    fn choice_row<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        current: T,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
        pick: fn(&mut Self, T, &mut Context<Self>),
    ) -> gpui::AnyElement {
        div()
            .px_4()
            .py_1()
            .flex()
            .gap_2()
            .children(options.iter().enumerate().map(|(i, &(value, label))| {
                let chosen = value == current;
                div()
                    .id((id, i))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if chosen { t.accent } else { t.surface }))
                    .text_color(rgb(t.text))
                    .when(chosen, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| pick(this, value, cx)))
            }))
            .into_any_element()
    }

    /// `None` follows the OS accent. The accent is process-wide so every `tokens()` call sees it.
    pub(crate) fn choose_accent(&mut self, accent: Option<matrix_desktop::Accent>, cx: &mut Context<Self>) {
        self.prefs.accent = accent;
        matrix_desktop::set_accent(self.prefs.effective_accent());
        self.set_theme(self.theme, cx);
        self.save_prefs(cx);
    }

    /// Mini window in the theme's own colours (rail, bubbles, accent); the chosen one gets an accent border.
    fn render_theme_preview(
        &self,
        theme: matrix_desktop::Theme,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = theme.tokens();
        let chosen = self.theme == theme;
        div()
            .id(("settings-theme", theme as usize))
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .py_1()
            .rounded_lg()
            .border_2()
            .border_color(rgb(if chosen { t.accent } else { t.surface }))
            .cursor_pointer()
            .child(
                div()
                    .w_full()
                    .h(px(64.))
                    .flex()
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(div().w(px(12.)).h_full().bg(rgb(p.surface)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .bg(rgb(p.timeline))
                            .child(div().w(px(18.)).h(px(10.)).rounded_sm().bg(rgb(p.surface)))
                            .child(div().flex().justify_end().child(div().w(px(22.)).h(px(10.)).rounded_sm().bg(rgb(p.outgoing))))
                            .child(div().w(px(12.)).h(px(10.)).rounded_sm().bg(rgb(p.accent))),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(t.text))
                    .when(chosen, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                    .child(theme.label()),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.choose_theme(theme, cx)))
            .into_any_element()
    }

    pub(crate) fn toggle_read_receipts(&mut self, cx: &mut Context<Self>) {
        self.prefs.read_receipts = !self.prefs.read_receipts;
        self.save_prefs(cx);
    }

    pub(crate) fn toggle_hide_redacted(&mut self, cx: &mut Context<Self>) {
        self.prefs.hide_redacted = !self.prefs.hide_redacted;
        self.save_prefs(cx);
    }

    pub(crate) fn toggle_close_to_tray(&mut self, cx: &mut Context<Self>) {
        self.prefs.close_to_tray = !self.prefs.close_to_tray;
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
                let error = self.switch_error.filter(|(e, _)| *e == id).map(|(_, m)| m);
                div()
                    .id(("menu-account", a.id.as_u64() as usize))
                    .min_h(px(52.))
                    .mx_2()
                    .px_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(ring(index))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(t.surface)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_account(id, window, cx)),
                    )
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
                            .when_some(error, |d, m| {
                                d.child(
                                    // wraps: a refusal is never cut mid-word
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(t.accent))
                                        .child(m),
                                )
                            })
                            .when(error.is_none() && a.name != a.user_id, |d| {
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
                .min_h(px(44.))
                .mx_2()
                .px_3()
                .py_1()
                .gap_3()
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
                            .text_color(rgb(t.accent))
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
                    .child(
                        row("menu-join-room")
                            .border_1()
                            .border_color(ring(n + 6))
                            .text_color(rgb(t.text))
                            .child("Unisciti")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_join_room(window, cx)
                            })),
                    )
                    .child(
                        row("menu-quick-search")
                            .border_1()
                            .border_color(ring(n + 7))
                            .text_color(rgb(t.text))
                            .child("Ricerca rapida")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_quick_search(window, cx)
                            })),
                    )
                })
                .into_any_element();
        }
        // The label takes the leftover width and wraps; the switch never shrinks (BUG-4).
        let label = |text: &'static str| div().flex_1().min_w_0().child(text);
        let switch = |on: bool| {
            div()
                .flex_shrink_0()
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
                    .border_color(rgb(
                        if self.settings_row == 0 && self.settings_focus.is_focused(window) {
                            t.accent
                        } else {
                            t.surface
                        },
                    ))
                    .child(label("Modalità debug"))
                    .child(switch(self.prefs.debug))
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        // Enter/Space are handled in menu_key; GPUI's key-up click on this focused row would toggle twice.
                        if !event.is_keyboard() {
                            this.toggle_debug(cx)
                        }
                    })),
            )
            .child(
                div()
                    .h(px(t.header))
                    .px_4()
                    .flex()
                    .items_center()
                    .text_color(rgb(t.text))
                    .child("Aspetto"),
            )
            .child(div().px_3().flex().gap_1().children(
                matrix_desktop::Theme::ALL.map(|theme| self.render_theme_preview(theme, t, cx)),
            ))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(matrix_desktop::Accent::ALL.map(|a| {
                        let tone = self.theme.tokens_with(a).accent;
                        let chosen = self.prefs.accent == Some(a);
                        div()
                            .id(("settings-accent", a as usize))
                            .size(px(28.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(rgb(if chosen { t.text } else { t.surface }))
                            .cursor_pointer()
                            .child(div().size(px(20.)).rounded_full().bg(rgb(tone)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.choose_accent(Some(a), cx)
                            }))
                    })),
            )
            .child(
                row("settings-accent-system")
                    .text_color(rgb(t.text))
                    .child("Colore principale di sistema")
                    .child(switch(self.prefs.accent.is_none()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let next = this.prefs.accent.map_or(Some(matrix_desktop::Accent::Blue), |_| None);
                        this.choose_accent(next, cx)
                    })),
            )
            .child(self.icon_row(t, cx))
            .child(
                div().px_4().pt_2().text_color(rgb(t.text)).child("Notte automatica"),
            )
            .child(self.choice_row(
                "settings-auto-night",
                &matrix_desktop::AutoNight::ALL.map(|a| (a, a.label())),
                self.prefs.auto_night,
                t,
                cx,
                Self::choose_auto_night,
            ))
            .child(div().px_4().pt_2().text_color(rgb(t.text)).child("Carattere"))
            .child(self.choice_row(
                "settings-font",
                &matrix_desktop::FontChoice::ALL.map(|f| (f, f.label())),
                self.prefs.font,
                t,
                cx,
                Self::choose_font,
            ))
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
                    .border_1()
                    .border_color(rgb(
                        if self.settings_row == 1 && self.settings_focus.is_focused(window) {
                            t.accent
                        } else {
                            t.surface
                        },
                    ))
                    .child(label("Conferme di lettura"))
                    .child(switch(self.prefs.read_receipts))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_read_receipts(cx))),
            )
            .child(
                row("settings-hide-redacted")
                    .text_color(rgb(t.text))
                    .border_1()
                    .border_color(rgb(
                        if self.settings_row == 2 && self.settings_focus.is_focused(window) {
                            t.accent
                        } else {
                            t.surface
                        },
                    ))
                    .child(label("Nascondi messaggi eliminati"))
                    .child(switch(self.prefs.hide_redacted))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_hide_redacted(cx))),
            )
            .child(
                row("settings-close-to-tray")
                    .text_color(rgb(t.text))
                    .border_1()
                    .border_color(rgb(
                        if self.settings_row == 3 && self.settings_focus.is_focused(window) {
                            t.accent
                        } else {
                            t.surface
                        },
                    ))
                    .child(label("Riduci nel tray alla chiusura"))
                    .child(switch(self.prefs.close_to_tray))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_close_to_tray(cx))),
            )
            .into_any_element()
    }
}

