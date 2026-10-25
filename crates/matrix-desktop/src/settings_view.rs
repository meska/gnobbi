//! Settings inside the side drawer: one switch, «Modalità debug», saved to the preferences file.
use crate::*;

impl Shell {
    pub(crate) fn set_settings_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = open;
        if open { self.settings_focus.focus(window) } else { self.drawer_focus.focus(window) }
        cx.notify();
    }

    pub(crate) fn toggle_debug(&mut self, cx: &mut Context<Self>) {
        self.prefs.debug = !self.prefs.debug;
        if let Some(dirs) = &self.prefs_dirs && let Err(error) = self.prefs.save(dirs) {
            eprintln!("Gnobbi: cannot save preferences: {error}");
        }
        cx.notify();
    }

    pub(crate) fn render_drawer_body(&self, t: matrix_desktop::DesignTokens, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = |id: &'static str| div().id(id).h(px(44.)).mx_2().px_3().flex().items_center().justify_between().rounded_lg().cursor_pointer();
        if !self.settings_open {
            return row("menu-settings").mt_2().text_color(rgb(t.text)).child("Impostazioni")
                .on_click(cx.listener(|this, _, window, cx| this.set_settings_open(true, window, cx)))
                .into_any_element();
        }
        let on = self.prefs.debug;
        let track = div().w(px(36.)).h(px(20.)).rounded_full().flex().items_center().px(px(2.))
            .bg(rgb(if on { t.accent } else { t.disabled }))
            .when(on, |d| d.justify_end())
            .child(div().w(px(16.)).h(px(16.)).rounded_full().bg(rgb(t.on_accent)));
        div()
            .child(div().h(px(t.header)).px_4().flex().items_center().text_color(rgb(t.text)).child("Impostazioni"))
            .child(
                row("settings-debug").track_focus(&self.settings_focus).text_color(rgb(t.text))
                    .border_1().border_color(rgb(if self.settings_focus.is_focused(window) { t.accent } else { t.surface }))
                    .child("Modalità debug").child(track)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_debug(cx))),
            )
            .into_any_element()
    }
}
