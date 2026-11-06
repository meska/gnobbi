//! Main-page Tab ring seam: room list, footer openers, compact Back, Info and the chat entry.
use crate::*;
use crate::timeline_view::IconStep;
use matrix_desktop::panel_openers::{Control, Intent};

impl Shell {
    pub(crate) fn back_focus_handle(&self) -> &FocusHandle {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        return &self.back_ui.focus;
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        &self.back_focus
    }

    pub(crate) fn focus_opener(
        &mut self,
        control: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(source) = self.opener_ui.source(control) else {
            return false;
        };
        self.apply_opener(
            Intent {
                control,
                source,
                focus_only: true,
            },
            window,
            cx,
        );
        self.opener_ui.focused(control, window)
    }

    fn back_visible(&self, window: &Window) -> bool {
        window.viewport_size().width < px(800.)
            && self.compact_chat
            && !self.notifications_open
            && !self.verification_open
            && !self.room_details_open
            && self.timeline.is_some()
            && self
                .sidebar
                .selected()
                .is_some_and(|room| room.scope == self.active_scope)
    }

    fn chat_visible(&self, window: &Window) -> bool {
        self.timeline.is_some()
            && self
                .sidebar
                .selected()
                .is_some_and(|room| room.scope == self.active_scope)
            && (window.viewport_size().width >= px(800.) || self.compact_chat)
    }

    /// Wrap backwards: Send when eligible, else the text field.
    pub(crate) fn focus_last_chat_control(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.chat_visible(window) {
            return false;
        }
        self.composer.update(cx, |editor, cx| {
            editor.focus_last_stop(window, cx)
        });
        cx.notify();
        true
    }

    /// Wrap forwards: compact Back, else the first rail entry.
    fn focus_first_nav_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.viewport_size().width < px(800.) {
            self.back_focus_handle().focus(window);
        } else {
            self.menu_focus.focus(window);
        }
        cx.notify();
    }

    fn focus_chat_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.connected {
            self.history_focus.focus(window);
            cx.notify();
            return;
        }
        // The «Scarica» icons painted now come before the composer.
        if self.chat_visible(window) && self.timeline.as_ref().is_some_and(|t| t.enter_icons(false, window)) {
            cx.notify();
            return;
        }
        self.composer
            .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
        cx.notify();
    }

    /// Shift-Tab from the chat entry or Info: the previous visible stop.
    fn focus_before_chat(&mut self, from_info: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !from_info && self.focus_opener(Control::Info, window, cx) {
            return;
        }
        if window.viewport_size().width < px(800.) {
            if self.back_visible(window) {
                self.back_focus_handle().focus(window);
            } else {
                self.list_focus.focus(window);
            }
        } else if !self.focus_opener(Control::Notifications, window, cx) {
            self.list_focus.focus(window);
        }
        cx.notify();
    }

    /// Forward from the room list: Info on wide windows, else spaces or the menu button.
    fn after_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.viewport_size().width >= px(800.) {
            if !self.focus_opener(Control::Search, window, cx) {
                self.focus_chat_entry(window, cx);
            }
        } else if self.connected && self.sidebar.spaces().next().is_some() {
            self.compact_spaces_tab(false, window, cx);
        } else {
            self.menu_focus.focus(window);
        }
    }

    /// Tab/Shift-Tab on a footer or header opener.
    pub(crate) fn opener_tab(
        &mut self,
        control: Control,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (control, shift) {
            (Control::Search, false) => {
                if !self.focus_opener(Control::Info, window, cx) {
                    self.focus_chat_entry(window, cx);
                }
            }
            (Control::Search, true) => self.focus_before_chat(true, window, cx),
            (Control::Info, false) => self.focus_chat_entry(window, cx),
            // The guard moves focus to the magnifier; only without it Shift-Tab leaves the header.
            (Control::Info, true) if !self.focus_opener(Control::Search, window, cx) => {
                self.focus_before_chat(true, window, cx)
            }
            // Devices and Notifications live in the drawer, not in the ring.
            _ => {}
        }
        cx.notify();
    }

    /// List, compact Back and chat-entry stops of the ring; false leaves the key to the older handlers.
    pub(crate) fn ring_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        if key.key != "tab"
            || key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
            || self.notifications_open
            || self.verification_open
            || self.room_details_open
        {
            return false;
        }
        let shift = key.modifiers.shift;
        if self.chat_visible(window)
            && let Some(timeline) = &self.timeline
        {
            let step = timeline.step_icons(shift, window, cx);
            match step {
                IconStep::Moved => {}
                IconStep::Left if shift => self.focus_before_chat(false, window, cx),
                IconStep::Left => self
                    .composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx)),
                IconStep::Ignored => {}
            }
            if !matches!(step, IconStep::Ignored) {
                cx.notify();
                return true;
            }
        }
        if self.list_focus.is_focused(window) && !shift {
            if self.connected {
                self.after_list(window, cx);
                cx.notify();
                return true;
            }
            if self.back_visible(window) {
                self.back_focus_handle().focus(window);
                cx.notify();
                return true;
            }
            if !self.connected && window.viewport_size().width >= px(800.) {
                self.focus_chat_entry(window, cx);
                return true;
            }
            return false;
        }
        if self.back_focus_handle().is_focused(window) && self.back_visible(window) {
            if !shift {
                if !self.focus_opener(Control::Search, window, cx) {
                    self.focus_chat_entry(window, cx);
                }
                return true;
            }
            if self.focus_last_chat_control(window, cx) {
                return true;
            }
        }
        if !shift && self.chat_visible(window) {
            let editor = self.composer.read(cx);
            if editor.at_last_stop(window) {
                self.focus_first_nav_control(window, cx);
                return true;
            }
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if shift && self.connected && self.chat_visible(window) && self.composer.read(cx).tab_back_ready(window) {
            self.history_focus.focus(window);
            cx.notify();
            return true;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let entry = if self.connected {
            self.history_focus.is_focused(window)
        } else {
            self.composer.read(cx).tab_back_ready(window)
        };
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let entry = self.composer.read(cx).tab_back_ready(window);
        if entry && shift && (window.viewport_size().width >= px(800.) || self.compact_chat) {
            let icons = self.chat_visible(window) && self.timeline.as_ref().is_some_and(|t| t.enter_icons(true, window));
            if !icons {
                self.focus_before_chat(false, window, cx);
            }
            return true;
        }
        false
    }
}
