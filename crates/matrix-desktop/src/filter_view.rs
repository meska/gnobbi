//! Keyboard activation (Space/Enter on key-up) and Tab order of the sidebar controls.
use crate::*;

/// Control that received the key-down; the release only activates the same, still current source.
#[derive(PartialEq)]
pub(crate) enum PressKind {
    Rail(Option<RoomScope>),
    Spaces(spaces_view::SpacesTarget),
}

pub(crate) struct SidebarPress {
    kind: PressKind,
    key: String,
    account: AccountId,
    revision: u64,
    pub(crate) cancelled: bool,
}

impl Shell {
    pub(crate) fn sidebar_activation_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        #[cfg(test)]
        if sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            sidebar_navigation_test::KEYDOWNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            eprintln!(
                "Shell capture down key={} held={} revision={}",
                key.key,
                event.is_held,
                self.sidebar.navigation_revision()
            );
        }
        if key.modifiers.modified() || !matches!(key.key.as_str(), "space" | "enter") {
            if let Some(press) = &mut self.pending_sidebar_key {
                press.cancelled = true;
            }
            return false;
        }
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        if self.notifications_open
            || self.verification_open
            || (!self.navigation_wide && self.compact_chat)
        {
            if let Some(press) = &mut self.pending_sidebar_key {
                press.cancelled = true;
            }
            return false;
        }
        let kind = if self.navigation_wide && self.rail_ui.focus.is_focused(window) {
            PressKind::Rail(self.rail_ui.target.clone())
        } else if !self.navigation_wide
            && self.connected
            && self.sidebar.spaces().next().is_some()
            && self.spaces_ui.focus.is_focused(window)
        {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            let target = self.spaces_ui.target;
            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
            let target = self.spaces_ui.target.clone();
            PressKind::Spaces(target)
        } else {
            return false;
        };
        if !event.is_held && self.pending_sidebar_key.is_none() {
            self.pending_sidebar_key = Some(SidebarPress {
                kind,
                key: key.key.clone(),
                account: self.active_scope.account_id,
                revision: self.sidebar.navigation_revision(),
                cancelled: false,
            });
        }
        true
    }

    pub(crate) fn sidebar_activation_up(
        &mut self,
        event: &gpui::KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        #[cfg(test)]
        if sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            sidebar_navigation_test::UPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            eprintln!("Shell capture up key={}", key.key);
        }
        if !matches!(key.key.as_str(), "space" | "enter") {
            return;
        }
        let pending = self.pending_sidebar_key.take();
        let swallow = std::mem::take(&mut self.swallow_menu_key_up);
        // GPUI's automatic key-up Click does not retain the key-down source.
        if pending.is_some() || swallow {
            cx.stop_propagation();
        }
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        if let Some(press) = pending
            && !press.cancelled
            && !key.modifiers.modified()
            && press.key == key.key
            && press.account == self.active_scope.account_id
            && press.revision == self.sidebar.navigation_revision()
            && !self.notifications_open
            && !self.verification_open
            && (self.navigation_wide || !self.compact_chat)
            && match &press.kind {
                PressKind::Rail(target) => {
                    self.navigation_wide
                        && self.rail_ui.focus.is_focused(window)
                        && self.rail_ui.target == *target
                }
                PressKind::Spaces(target) => {
                    !self.navigation_wide
                        && self.spaces_ui.focus.is_focused(window)
                        && self.spaces_ui.target == *target
                }
            }
        {
            match press.kind {
                PressKind::Rail(_) => self.rail_activate(cx),
                PressKind::Spaces(_) => self.spaces_activate(window, cx),
            }
        }
    }

    pub(crate) fn reconcile_sidebar_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "live")]
        if self.signin.is_some() {
            return;
        }
        if self.notifications_open || self.verification_open {
            return;
        }
        // Back only exists on the narrow chat page: widening hands its focus to the composer.
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if window.viewport_size().width >= px(800.) && self.back_ui.focus.is_focused(window) {
            self.composer
                .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
        }
        // The chat entry keeps focus across a shrink: show the chat page it belongs to.
        if window.viewport_size().width < px(800.)
            && !self.compact_chat
            && (self.composer.read(cx).is_focused(window) || {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    self.history_focus.is_focused(window)
                }
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                {
                    false
                }
            })
        {
            self.set_compact_chat(true);
        }
        if window.viewport_size().width < px(800.) && self.compact_chat {
            if self.rail_ui.focus.is_focused(window)
                || self.spaces_ui.focus.is_focused(window)
                || self.search_focus.is_focused(window)
                || self.list_focus.is_focused(window)
                || self.opener_ui.hidden_focused(window)
            {
                self.composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            }
            return;
        }
        if self.opener_ui.hidden_focused(window) {
            self.list_focus.focus(window);
        }
        if !self
            .rail_entries()
            .iter()
            .any(|entry| entry.scope == self.rail_ui.target)
        {
            self.rail_ui.target = None;
        }
        if window.viewport_size().width < px(800.) && self.spaces_ui.focus.is_focused(window) {
            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
            if !self.spaces_open
                || !self
                    .sidebar
                    .navigation_entries()
                    .iter()
                    .any(|entry| Some(entry.scope.clone()) == self.spaces_ui.target)
            {
                self.spaces_ui.target = None;
            }
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if self
                .semantic_spaces(window)
                .is_some_and(|view| !view.nodes().iter().any(|(id, _)| Some(*id) == view.focused))
            {
                self.spaces_shortcut_focus(window, cx);
            }
        }
        if window.viewport_size().width < px(800.)
            && self.spaces_ui.focus.is_focused(window)
            && (!self.connected || self.sidebar.spaces().next().is_none())
        {
            self.first_list_stop().focus(window);
        }
        if window.viewport_size().width < px(800.) && self.rail_ui.focus.is_focused(window) {
            self.search_focus.focus(window);
        }
        if window.viewport_size().width >= px(800.) && self.spaces_ui.focus.is_focused(window) {
            self.first_list_stop().focus(window);
        }
    }

    /// First stop of the list column: the disconnection icon while shown, else the search field.
    pub(crate) fn first_list_stop(&self) -> &FocusHandle {
        if self.disconnected_shown().is_some() {
            &self.alert_focus
        } else {
            &self.search_focus
        }
    }

    pub(crate) fn sidebar_tab(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let compact = window.viewport_size().width < px(800.);
        let spaces = compact && self.connected && self.sidebar.spaces().next().is_some();
        if (compact && self.compact_chat)
            || key.key != "tab"
            || key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
        {
            return false;
        }
        let target = if self.alert_focus.is_focused(window) {
            // The icon exists only while shown: Tab goes on to the list, Shift-Tab leaves the column backwards.
            if key.modifiers.shift {
                if compact {
                    &self.menu_focus
                } else {
                    self.rail_ui.on_unread = false;
                    self.rail_ui.target = self.rail_entries().last().unwrap().scope.clone();
                    &self.rail_ui.edit_focus
                }
            } else {
                &self.search_focus
            }
        } else if self.disconnected_shown().is_some()
            && self.search_focus.is_focused(window)
            && key.modifiers.shift
        {
            &self.alert_focus
        } else if self.search_focus.is_focused(window) && key.modifiers.shift {
            // Shift-Tab from the first stop of the list column leaves it backwards.
            if compact {
                &self.menu_focus
            } else {
                self.rail_ui.on_unread = false;
                self.rail_ui.target = self.rail_entries().last().unwrap().scope.clone();
                &self.rail_ui.edit_focus
            }
        } else if self.list_focus.is_focused(window) && key.modifiers.shift {
            &self.search_focus
        } else if spaces && self.list_focus.is_focused(window) {
            self.compact_spaces_tab(false, window, cx);
            return true;
        } else if spaces && self.spaces_ui.focus.is_focused(window) {
            self.compact_spaces_tab(key.modifiers.shift, window, cx);
            return true;
        } else if compact && self.list_focus.is_focused(window) {
            &self.menu_focus
        } else {
            // owns the forward list/chat seam; preserve its current dispatch.
            return false;
        };
        target.focus(window);
        cx.notify();
        true
    }

}
