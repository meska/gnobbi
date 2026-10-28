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

    pub(crate) fn toggle_dark_setting(&mut self, cx: &mut Context<Self>) {
        self.toggle_theme(cx);
        self.prefs.theme = if self.dark { matrix_desktop::Theme::Dark } else { matrix_desktop::Theme::Light };
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
                .child(
                    row("menu-settings")
                        .border_1()
                        .border_color(ring(0))
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
                            .border_color(ring(1))
                            .text_color(rgb(t.text))
                            .child("Dispositivi")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_from_drawer(window, cx, Shell::open_devices_panel)
                            })),
                    )
                    .child(
                        row("menu-notifications")
                            .border_1()
                            .border_color(ring(2))
                            .text_color(rgb(t.text))
                            .child("Notifiche")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_from_drawer(window, cx, Shell::open_notifications_panel)
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
            .child(
                row("settings-theme")
                    .text_color(rgb(t.text))
                    .child("Tema scuro")
                    .child(switch(self.dark))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_dark_setting(cx))),
            )
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
