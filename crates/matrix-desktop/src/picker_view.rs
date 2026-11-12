//! Emoji picker: hover popup / pinned drawer state machine and the view drawn in either.
//! Design: the EMOJI_PICKER notes.
use gpui::{
    Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, prelude::*, px, rgb,
};
use matrix_desktop::{DesignTokens, EMOJI_SET, LIGHT};
use std::{cell::Cell, rc::Rc, time::Duration};

/// Grace period after the pointer leaves icon and popup (Telegram Desktop: 300 ms).
pub(crate) const CLOSE_AFTER: Duration = Duration::from_millis(300);
const COLS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum PickerMode {
    #[default]
    Closed,
    /// Small popup under the pointer.
    Hover,
    /// Fixed drawer on the right.
    Pinned,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Zone {
    Icon,
    Popup,
}

/// What the owner does with the close timer after a transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Timer {
    Cancel,
    Start,
}

/// Pure state machine; the owner runs the timer and the focus.
#[derive(Default)]
pub(crate) struct Picker {
    pub mode: PickerMode,
    icon: bool,
    popup: bool,
}

impl Picker {
    pub fn hover(&mut self, zone: Zone, over: bool) -> Timer {
        match zone {
            Zone::Icon => self.icon = over,
            Zone::Popup => self.popup = over,
        }
        if over && zone == Zone::Icon && self.mode == PickerMode::Closed {
            self.mode = PickerMode::Hover;
        }
        self.settle()
    }

    /// The pointer left the window: gpui sends no on_hover(false) then, so both zones are out.
    pub fn leave_window(&mut self) -> Timer {
        self.icon = false;
        self.popup = false;
        self.settle()
    }

    /// Icon click, mouse or keyboard: open the drawer, or close it.
    pub fn click(&mut self) {
        self.mode = if self.mode == PickerMode::Pinned { PickerMode::Closed } else { PickerMode::Pinned };
        self.settle();
    }

    /// Esc / Close / room switch. True when something was open.
    pub fn close(&mut self) -> bool {
        let was = self.mode != PickerMode::Closed;
        self.mode = PickerMode::Closed;
        self.settle();
        was
    }

    /// The close timer ran out. Ignored if the pointer came back or the mode changed.
    pub fn fired(&mut self) -> bool {
        let close = self.mode == PickerMode::Hover && !self.icon && !self.popup;
        if close {
            self.mode = PickerMode::Closed;
        }
        close
    }

    fn settle(&mut self) -> Timer {
        if self.mode != PickerMode::Hover {
            self.popup = false; // the popup is gone: no leave event will come
            return Timer::Cancel;
        }
        if self.icon || self.popup { Timer::Cancel } else { Timer::Start }
    }
}

/// Arrow/Home/End movement over the grid, stopping at the edges. None: not a movement key.
pub(crate) fn grid_move(at: usize, key: &str, len: usize) -> Option<usize> {
    let last = len - 1;
    Some(match key {
        "left" => at.saturating_sub(1),
        "right" => (at + 1).min(last),
        "up" => at.saturating_sub(COLS),
        "down" if at + COLS <= last => at + COLS,
        "down" => at,
        "home" => 0,
        "end" => last,
        _ => return None,
    })
}

/// Tabs of the picker; Sticker and GIF are a variant here plus one in `Pick` and a body below.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerTab {
    Emoji,
}

impl PickerTab {
    const ALL: &'static [PickerTab] = &[PickerTab::Emoji];

    fn label(self) -> &'static str {
        match self {
            PickerTab::Emoji => "Emoji",
        }
    }
}

pub(crate) enum Pick {
    /// Index into `EMOJI_SET`.
    Emoji(usize),
}

pub(crate) enum PickerEvent {
    Picked(Pick),
    Close,
    FocusField,
    FocusIcon,
}

pub(crate) struct PickerView {
    tab: PickerTab,
    selected: Rc<Cell<usize>>,
    grid_focus: FocusHandle,
    close_focus: FocusHandle,
    pub drawer: bool,
    pub theme: DesignTokens,
}

impl EventEmitter<PickerEvent> for PickerView {}

impl PickerView {
    /// `grid_focus`, `close_focus` and `selected` are shared with the composer, which exposes them to the a11y tree.
    pub fn new(grid_focus: FocusHandle, close_focus: FocusHandle, selected: Rc<Cell<usize>>) -> Self {
        Self {
            tab: PickerTab::Emoji,
            selected,
            grid_focus,
            close_focus,
            drawer: false,
            theme: LIGHT,
        }
    }

    fn grid_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if let Some(to) = grid_move(self.selected.get(), key, EMOJI_SET.len()) {
            self.selected.set(to);
            cx.notify();
        } else {
            match key {
                "enter" | "space" => cx.emit(PickerEvent::Picked(Pick::Emoji(self.selected.get()))),
                "escape" => cx.emit(PickerEvent::Close),
                "tab" if event.keystroke.modifiers.shift => cx.emit(PickerEvent::FocusIcon),
                "tab" if self.drawer => self.close_focus.focus(window),
                "tab" => cx.emit(PickerEvent::FocusField),
                _ => return,
            }
        }
        cx.stop_propagation();
    }

    fn close_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(PickerEvent::Close),
            "tab" if event.keystroke.modifiers.shift => self.grid_focus.focus(window),
            "tab" => cx.emit(PickerEvent::FocusField),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn emoji_grid(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let focused = self.grid_focus.is_focused(window);
        div()
            .id("composer-emoji-grid")
            .track_focus(&self.grid_focus)
            .on_key_down(cx.listener(Self::grid_key))
            .w(px((COLS * 40) as f32))
            .flex()
            .flex_wrap()
            .children(EMOJI_SET.iter().enumerate().map(|(i, (_, emoji))| {
                div()
                    .id(("composer-emoji", i))
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_size(px(24.))
                    .when(focused && i == self.selected.get(), |d| d.bg(rgb(t.selection)))
                    .hover(|s| s.bg(rgb(t.selected)))
                    .child(*emoji)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PickerEvent::Picked(Pick::Emoji(i)))))
            }))
    }
}

impl Render for PickerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .when(!self.drawer, |d| d.border_1().border_color(rgb(t.border)).rounded_lg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .children(PickerTab::ALL.iter().map(|tab| {
                        div()
                            .px_2()
                            .py_1()
                            .rounded_lg()
                            .when(*tab == self.tab, |d| d.bg(rgb(t.selected)))
                            .child(tab.label())
                    }))
                    .when(self.drawer, |d| {
                        d.child(
                            div()
                                .id("composer-emoji-close")
                                .track_focus(&self.close_focus)
                                .on_key_down(cx.listener(Self::close_key))
                                .when(self.close_focus.is_focused(window), |d| d.border_1().border_color(rgb(t.text)))
                                .size(px(32.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_lg()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(t.selected)))
                                .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Close))),
                        )
                    }),
            )
            .child(match self.tab {
                PickerTab::Emoji => self.emoji_grid(window, cx),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PickerMode::*;

    #[test]
    fn hover_opens_and_leaving_both_starts_the_timer() {
        let mut p = Picker::default();
        assert_eq!(p.hover(Zone::Icon, true), Timer::Cancel);
        assert_eq!(p.mode, Hover);
        // Icon -> popup: the icon's leave comes before the popup's enter; no close in between.
        assert_eq!(p.hover(Zone::Icon, false), Timer::Start);
        assert_eq!(p.hover(Zone::Popup, true), Timer::Cancel);
        assert!(!p.fired());
        assert_eq!(p.hover(Zone::Popup, false), Timer::Start);
        assert!(p.fired());
        assert_eq!(p.mode, Closed);
    }

    #[test]
    fn leaving_the_window_from_icon_or_popup_starts_the_timer() {
        let mut p = Picker::default();
        p.hover(Zone::Icon, true);
        assert_eq!(p.leave_window(), Timer::Start);
        assert!(p.fired());
        p.hover(Zone::Icon, true);
        p.hover(Zone::Popup, true);
        assert_eq!(p.leave_window(), Timer::Start);
        assert!(p.fired());
        assert_eq!(Picker::default().leave_window(), Timer::Cancel); // closed: nothing to time
    }

    #[test]
    fn returning_within_the_grace_period_keeps_it_open() {
        let mut p = Picker::default();
        p.hover(Zone::Icon, true);
        assert_eq!(p.hover(Zone::Icon, false), Timer::Start);
        assert_eq!(p.hover(Zone::Icon, true), Timer::Cancel);
        assert!(!p.fired(), "a stale timer must not close it");
        assert_eq!(p.mode, Hover);
    }

    #[test]
    fn leaving_a_closed_picker_does_nothing() {
        let mut p = Picker::default();
        assert_eq!(p.hover(Zone::Icon, false), Timer::Cancel);
        assert_eq!(p.mode, Closed);
        assert!(!p.fired());
    }

    #[test]
    fn click_pins_from_closed_and_hover_and_toggles_back() {
        let mut p = Picker::default();
        p.click();
        assert_eq!(p.mode, Pinned);
        p.click();
        assert_eq!(p.mode, Closed);
        p.hover(Zone::Icon, true);
        p.click();
        assert_eq!(p.mode, Pinned);
    }

    #[test]
    fn pinned_ignores_hover_and_a_late_timer() {
        let mut p = Picker::default();
        p.hover(Zone::Icon, true);
        p.hover(Zone::Icon, false); // timer running
        p.click();
        assert_eq!(p.hover(Zone::Icon, true), Timer::Cancel);
        assert_eq!(p.hover(Zone::Icon, false), Timer::Cancel);
        assert!(!p.fired());
        assert_eq!(p.mode, Pinned);
    }

    #[test]
    fn esc_closes_either_mode_and_reports_whether_it_did() {
        let mut p = Picker::default();
        assert!(!p.close());
        p.hover(Zone::Icon, true);
        assert!(p.close());
        assert_eq!(p.mode, Closed);
        p.click();
        assert!(p.close());
        assert_eq!(p.mode, Closed);
    }

    #[test]
    fn a_vanished_popup_does_not_pin_the_pointer_inside_it() {
        let mut p = Picker::default();
        p.hover(Zone::Icon, true);
        p.hover(Zone::Popup, true); // pointer in the popup, which then closes by Esc
        p.close();
        p.hover(Zone::Icon, true);
        assert_eq!(p.hover(Zone::Icon, false), Timer::Start, "stale popup flag would keep it open");
        assert!(p.fired());
    }

    #[test]
    fn grid_stops_at_the_edges() {
        let n = 32;
        assert_eq!(grid_move(0, "left", n), Some(0));
        assert_eq!(grid_move(31, "right", n), Some(31));
        assert_eq!(grid_move(3, "up", n), Some(0));
        assert_eq!(grid_move(30, "down", n), Some(30));
        assert_eq!(grid_move(2, "down", n), Some(10));
        assert_eq!(grid_move(10, "right", n), Some(11));
        assert_eq!(grid_move(10, "home", n), Some(0));
        assert_eq!(grid_move(10, "end", n), Some(31));
        assert_eq!(grid_move(10, "a", n), None);
    }
}
