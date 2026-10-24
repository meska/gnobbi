//! Main-page Tab ring seam: room list, footer openers, compact Back, Info and the chat entry.
use crate::*;
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

    fn focus_chat_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.connected {
            self.history_focus.focus(window);
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

    /// Tab/Shift-Tab on a footer or header opener.
    pub(crate) fn opener_tab(
        &mut self,
        control: Control,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (control, shift) {
            (Control::Devices, false) => {
                self.focus_opener(Control::Notifications, window, cx);
            }
            (Control::Devices, true) => self.list_focus.focus(window),
            (Control::Notifications, true) => {
                self.focus_opener(Control::Devices, window, cx);
            }
            (Control::Notifications, false) => {
                if window.viewport_size().width >= px(800.) {
                    if !self.focus_opener(Control::Info, window, cx) {
                        self.focus_chat_entry(window, cx);
                    }
                } else if self.connected && self.sidebar.spaces().next().is_some() {
                    self.compact_spaces_tab(false, window, cx);
                } else {
                    self.theme_focus.focus(window);
                }
            }
            (Control::Info, false) => self.focus_chat_entry(window, cx),
            (Control::Info, true) => self.focus_before_chat(true, window, cx),
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
        if self.list_focus.is_focused(window) && !shift {
            if self.connected && self.focus_opener(Control::Devices, window, cx) {
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
                if !self.focus_opener(Control::Info, window, cx) {
                    self.focus_chat_entry(window, cx);
                }
                return true;
            }
            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
            {
                self.set_compact_chat(false);
                self.list_focus.focus(window);
                cx.notify();
                return true;
            }
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let entry = self.connected && self.history_focus.is_focused(window);
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let entry = self.composer.read(cx).tab_back_ready(window);
        if entry && shift && (window.viewport_size().width >= px(800.) || self.compact_chat) {
            self.focus_before_chat(false, window, cx);
            return true;
        }
        false
    }
}
