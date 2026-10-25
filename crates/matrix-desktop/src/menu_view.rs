//! Menu button and side drawer: opens over the list, closes on outside click, Esc or the button;
//! focus moves into the drawer and returns to the button.
use crate::*;

pub(crate) const DRAWER_WIDTH: f32 = 280.;

impl Shell {
    pub(crate) fn set_menu_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        if open { self.drawer_focus.focus(window) } else { self.menu_focus.focus(window) }
        cx.notify();
    }

    /// Runs before every other key handler; true when the key was consumed.
    pub(crate) fn menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        if self.connected && m.control && m.shift && key.key == "m" {
            self.set_menu_open(!self.menu_open, window, cx);
            return true;
        }
        if self.menu_open {
            match key.key.as_str() {
                "escape" => self.set_menu_open(false, window, cx),
                // The empty drawer holds one focus stop: Tab stays in it.
                "tab" => self.drawer_focus.focus(window),
                _ => (),
            }
            return matches!(key.key.as_str(), "escape" | "tab");
        }
        if !m.modified() && matches!(key.key.as_str(), "enter" | "space") && self.menu_focus.is_focused(window) {
            self.set_menu_open(true, window, cx);
            return true;
        }
        false
    }

    pub(crate) fn render_menu_button(&self, t: matrix_desktop::DesignTokens, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let line = || div().w(px(16.)).h(px(2.)).rounded_sm().bg(rgb(t.text));
        div()
            .id("menu-button")
            .track_focus(&self.menu_focus)
            .w(px(36.)).h(px(36.)).flex_shrink_0()
            .flex().flex_col().items_center().justify_center().gap(px(3.))
            .rounded_lg().cursor_pointer()
            .border_1().border_color(rgb(if self.menu_focus.is_focused(window) { t.accent } else { t.surface }))
            .child(line()).child(line()).child(line())
            .on_click(cx.listener(|this, _, window, cx| this.set_menu_open(!this.menu_open, window, cx)))
            .into_any_element()
    }

    /// Scrim and drawer start below the header so the button stays clickable.
    pub(crate) fn render_drawer(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> gpui::AnyElement {
        use gpui::{Animation, AnimationExt};
        div()
            .id("menu-scrim")
            .absolute().left_0().right_0().bottom_0().top(px(t.header))
            .bg(gpui::rgba(0x00000066))
            .on_click(cx.listener(|this, _, window, cx| this.set_menu_open(false, window, cx)))
            .child(
                div()
                    .id("menu-drawer")
                    .track_focus(&self.drawer_focus)
                    .absolute().top_0().bottom_0().w(px(DRAWER_WIDTH))
                    .bg(rgb(t.surface))
                    .border_r_1().border_color(rgb(t.border))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .with_animation(
                        "menu-drawer-slide",
                        Animation::new(std::time::Duration::from_millis(140)),
                        |d, delta| d.left(px(-DRAWER_WIDTH * (1. - delta))),
                    ),
            )
            .into_any_element()
    }
}
