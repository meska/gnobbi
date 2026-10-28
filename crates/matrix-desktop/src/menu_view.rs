//! Menu button and side drawer: opens over the list, closes on outside click, Esc or the button;
//! focus moves into the drawer and returns to the button.
use crate::*;

pub(crate) const DRAWER_WIDTH: f32 = 280.;
/// Settings, Devices, Notifications.
pub(crate) const DRAWER_ROWS: usize = 3;

impl Shell {
    pub(crate) fn set_menu_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        self.settings_open = false;
        self.drawer_row = 0;
        if open {
            self.drawer_focus.focus(window)
        } else {
            self.menu_focus.focus(window)
        }
        cx.notify();
    }

    /// Closes the drawer, then lets `open` show a panel (which takes focus).
    pub(crate) fn open_from_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        open: fn(&mut Self, &mut Window),
    ) {
        self.set_menu_open(false, window, cx);
        open(self, window);
        cx.notify();
    }

    /// Runs before every other key handler; true when the key was consumed.
    pub(crate) fn menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        if self.connected && m.control && m.shift && key.key == "m" {
            self.set_menu_open(!self.menu_open, window, cx);
            return true;
        }
        if self.menu_open {
            let activate = !m.modified() && matches!(key.key.as_str(), "enter" | "space");
            match key.key.as_str() {
                "escape" if self.settings_open => self.set_settings_open(false, window, cx),
                "escape" => self.set_menu_open(false, window, cx),
                "enter" | "space" if activate && self.settings_open => self.toggle_debug(cx),
                "enter" | "space" if activate => match self.drawer_row {
                    1 => self.open_from_drawer(window, cx, Shell::open_devices_panel),
                    2 => self.open_from_drawer(window, cx, Shell::open_notifications_panel),
                    _ => self.set_settings_open(true, window, cx),
                },
                // The drawer holds one focus stop; Up/Down/Tab move between its rows.
                "tab" if self.settings_open => self.settings_focus.focus(window),
                "up" | "down" | "tab" => {
                    let back = key.key == "up" || m.shift;
                    self.drawer_row =
                        (self.drawer_row + if back { DRAWER_ROWS - 1 } else { 1 }) % DRAWER_ROWS;
                    self.drawer_focus.focus(window);
                    cx.notify();
                }
                _ => (),
            }
            return matches!(key.key.as_str(), "escape" | "tab" | "up" | "down") || activate;
        }
        if !m.modified()
            && matches!(key.key.as_str(), "enter" | "space")
            && self.menu_focus.is_focused(window)
        {
            self.set_menu_open(true, window, cx);
            return true;
        }
        false
    }

    pub(crate) fn render_menu_button(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let line = || div().w(px(16.)).h(px(2.)).rounded_sm().bg(rgb(t.text));
        div()
            .id("menu-button")
            .relative()
            .children({
                #[cfg(test)] { crate::sidebar_navigation_test::observe("menu-button".into()) }
                #[cfg(not(test))] { None::<gpui::AnyElement> }
            })
            .track_focus(&self.menu_focus)
            .w(px(36.))
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .rounded_lg()
            .cursor_pointer()
            .border_1()
            .border_color(rgb(if self.menu_focus.is_focused(window) {
                t.accent
            } else {
                t.surface
            }))
            .child(line())
            .child(line())
            .child(line())
            .on_click(
                cx.listener(|this, _, window, cx| this.set_menu_open(!this.menu_open, window, cx)),
            )
            .into_any_element()
    }

    /// Scrim and drawer cover the whole window (rail and list included); the scrim takes every click.
    pub(crate) fn render_drawer(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use gpui::{Animation, AnimationExt};
        div()
            .id("menu-scrim")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .top_0()
            .bg(gpui::rgba(0x00000066))
            .on_click(cx.listener(|this, _, window, cx| this.set_menu_open(false, window, cx)))
            .child(
                div()
                    .id("menu-drawer")
                    .track_focus(&self.drawer_focus)
                    .children({
                        #[cfg(test)] { crate::sidebar_navigation_test::observe("menu-drawer".into()) }
                        #[cfg(not(test))] { None::<gpui::AnyElement> }
                    })
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .w(px(DRAWER_WIDTH))
                    .bg(rgb(t.surface))
                    .border_r_1()
                    .border_color(rgb(t.border))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .when(!self.settings_open, |d| d.children(self.render_drawer_header(t)))
                    .child(div().flex_1().child(self.render_drawer_body(t, window, cx)))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .text_size(px(12.))
                            .text_color(rgb(t.secondary))
                            .child(concat!("Gnobbi ", env!("CARGO_PKG_VERSION"))),
                    )
                    .with_animation(
                        "menu-drawer-slide",
                        Animation::new(std::time::Duration::from_millis(140)),
                        |d, delta| d.left(px(-DRAWER_WIDTH * (1. - delta))),
                    ),
            )
            .into_any_element()
    }

    /// Telegram-style profile block: initials avatar, name and Matrix ID, both from the SDK.
    fn render_drawer_header(&self, t: matrix_desktop::DesignTokens) -> Option<gpui::AnyElement> {
        let (id, name) = self.verification.profile()?;
        let name = name.clone().unwrap_or_else(|| id.clone());
        Some(
            div()
                .id("menu-profile")
                .p_4()
                .flex()
                .flex_col()
                .gap_1()
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .size(px(56.))
                        .mb_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(0x5288b5))
                        .text_size(px(20.))
                        .text_color(rgb(t.avatar_text))
                        .child(name.trim_start_matches('@').chars().take(2).collect::<String>()),
                )
                .child(
                    div()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(name.clone()),
                )
                .when(name != *id, |d| {
                    d.child(
                        div().truncate().text_size(px(13.)).text_color(rgb(t.secondary)).child(id.clone()),
                    )
                })
                .into_any_element(),
        )
    }
}
