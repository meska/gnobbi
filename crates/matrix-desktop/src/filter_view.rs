//! Keyboard and assistive entry to the existing session-only unread filter.
use crate::*;

/// Control that received the key-down; the release only activates the same, still current source.
#[derive(PartialEq)]
pub(crate) enum PressKind {
    Theme,
    Unread,
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
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn dispatch_sidebar_action(
        &mut self,
        action: &matrix_desktop::accessibility::NavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use matrix_desktop::accessibility::NavigationAction as A;
        let (A::ToggleTheme(revision) | A::FocusTheme(revision)) = action else {
            return false;
        };
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        if *revision != self.sidebar.navigation_revision()
            || !self.sidebar.navigation_callback()(&self.sidebar)
        {
            return true;
        }
        match action {
            A::ToggleTheme(_) => self.toggle_theme(cx),
            A::FocusTheme(_) => {
                self.theme_focus.focus(window);
                cx.notify();
            }
            _ => unreachable!(),
        }
        true
    }

    pub(crate) fn sidebar_pointer_callback(
        &self,
        unread: bool,
    ) -> impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + use<> {
        let source = self.sidebar.navigation_callback();
        move |this, window, cx| {
            this.navigation_wide = window.viewport_size().width >= px(800.);
            this.refresh_navigation_visibility();
            if !source(&this.sidebar) {
                return;
            }
            if unread {
                this.toggle_unread(window, cx);
            } else {
                this.toggle_theme(cx);
            }
        }
    }

    fn toggle_unread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[1].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.sidebar.set_unread_only(!self.sidebar.unread_only());
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.unread_focus.focus(window);
        cx.notify();
    }

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
                "Shell capture down key={} held={} theme={} unread={} revision={}",
                key.key,
                event.is_held,
                self.theme_focus.is_focused(window),
                self.unread_focus.is_focused(window),
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
            || self.room_details_open
            || (!self.navigation_wide && self.compact_chat)
        {
            if let Some(press) = &mut self.pending_sidebar_key {
                press.cancelled = true;
            }
            return false;
        }
        let kind = if self.connected && self.unread_focus.is_focused(window) {
            PressKind::Unread
        } else if self.theme_focus.is_focused(window) {
            PressKind::Theme
        } else if self.navigation_wide && self.rail_ui.focus.is_focused(window) {
            PressKind::Rail(self.rail_ui.target.clone())
        } else if !self.navigation_wide
            && self.connected
            && self.sidebar.spaces().next().is_some()
            && self.spaces_ui.focus.is_focused(window)
        {
            PressKind::Spaces(self.spaces_ui.target.clone())
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
        // GPUI's automatic key-up Click does not retain the key-down source.
        if self.theme_focus.is_focused(window)
            || self.unread_focus.is_focused(window)
            || pending.is_some()
        {
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
            && !self.room_details_open
            && (self.navigation_wide || !self.compact_chat)
            && match &press.kind {
                PressKind::Unread => self.connected && self.unread_focus.is_focused(window),
                PressKind::Theme => self.theme_focus.is_focused(window),
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
                PressKind::Unread => self.toggle_unread(window, cx),
                PressKind::Theme => self.toggle_theme(cx),
                PressKind::Rail(_) => self.rail_activate(cx),
                PressKind::Spaces(_) => self.spaces_activate(window, cx),
            }
        }
    }

    pub(crate) fn reconcile_sidebar_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "live", unix))]
        if self.signin.is_some() {
            return;
        }
        if self.notifications_open || self.verification_open || self.room_details_open {
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
                { self.history_focus.is_focused(window) }
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                { false }
            })
        {
            self.set_compact_chat(true);
        }
        if window.viewport_size().width < px(800.) && self.compact_chat {
            if self.rail_ui.focus.is_focused(window)
                || self.spaces_ui.focus.is_focused(window)
                || self.theme_focus.is_focused(window)
                || self.unread_focus.is_focused(window)
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
            .sidebar
            .navigation_entries()
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
            self.theme_focus.focus(window);
        }
        if window.viewport_size().width < px(800.) && self.rail_ui.focus.is_focused(window) {
            self.search_focus.focus(window);
        }
        if window.viewport_size().width >= px(800.) && self.spaces_ui.focus.is_focused(window) {
            self.theme_focus.focus(window);
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
        let target = if self.theme_focus.is_focused(window) {
            if key.modifiers.shift && spaces {
                self.compact_spaces_tab(true, window, cx);
                return true;
            } else if key.modifiers.shift && compact {
                if self.connected
                    && self.focus_opener(matrix_desktop::panel_openers::Control::Notifications, window, cx)
                {
                    cx.notify();
                    return true;
                }
                &self.list_focus
            } else if key.modifiers.shift {
                self.rail_ui.target = self
                    .sidebar
                    .navigation_entries()
                    .last()
                    .unwrap()
                    .scope
                    .clone();
                &self.rail_ui.focus
            } else if self.connected {
                &self.unread_focus
            } else {
                &self.search_focus
            }
        } else if self.connected && self.unread_focus.is_focused(window) {
            if key.modifiers.shift {
                &self.theme_focus
            } else {
                &self.search_focus
            }
        } else if self.search_focus.is_focused(window) && key.modifiers.shift {
            if self.connected {
                &self.unread_focus
            } else {
                &self.theme_focus
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
            &self.theme_focus
        } else {
            // owns the forward list/chat seam; preserve its current dispatch.
            return false;
        };
        target.focus(window);
        cx.notify();
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_unread(
        &self,
        window: &Window,
    ) -> Option<matrix_desktop::accessibility::filters::UnreadFilter> {
        (self.connected
            && !self.notifications_open
            && !self.verification_open
            && !self.room_details_open
            && (window.viewport_size().width >= px(800.) || !self.compact_chat))
            .then_some(matrix_desktop::accessibility::filters::UnreadFilter {
                account: self.active_scope.account_id,
                revision: self.sidebar.navigation_revision(),
                checked: self.sidebar.unread_only(),
                focused: self.unread_focus.is_focused(window),
                bounds: self.a11y_unread_bounds,
            })
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_filter(
        &mut self,
        action: matrix_desktop::accessibility::filters::FilterAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::filters::FilterAction;
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        if !self
            .semantic_unread(window)
            .is_some_and(|current| current.allows(&action))
        {
            return;
        }
        if let FilterAction::SetUnread { checked, .. } = action {
            #[cfg(test)]
            sidebar_navigation_test::ACTIVATIONS[1]
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.sidebar.set_unread_only(checked);
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        self.unread_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn render_unread(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = if self.dark { DARK } else { LIGHT };
        let focused = self.unread_focus.is_focused(window);
        let control = div()
            .id("unread-filter")
            .relative()
            .flex_shrink_0()
            .cursor_pointer()
            .text_color(rgb(t.accent))
            .border_1()
            .border_color(rgb(if focused { t.accent } else { t.surface }))
            .child(if self.sidebar.unread_only() {
                "Tutte"
            } else {
                "Non lette"
            })
            .children(self.unread_observer(cx))
            .children({
                #[cfg(test)]
                {
                    sidebar_navigation_test::observe("unread".into())
                }
                #[cfg(not(test))]
                {
                    None::<gpui::AnyElement>
                }
            })
            .on_click({
                let callback = self.sidebar_pointer_callback(true);
                #[cfg(test)]
                let callback = sidebar_navigation_test::capture_pointer(true, callback);
                cx.listener(move |this, _, window, cx| callback(this, window, cx))
            });
        let control = control.track_focus(&self.unread_focus);
        control.into_any_element()
    }

    fn unread_observer(&self, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = _cx.entity();
            Some(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = f64::from(window.scale_factor());
                        let rect = accesskit::Rect::new(
                            f64::from(f32::from(bounds.left())) * scale,
                            f64::from(f32::from(bounds.top())) * scale,
                            f64::from(f32::from(bounds.right())) * scale,
                            f64::from(f32::from(bounds.bottom())) * scale,
                        );
                        if entity.read(cx).a11y_unread_bounds != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_unread_bounds = Some(rect);
                                    cx.notify();
                                })
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .into_any_element(),
            )
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }
    pub(crate) fn theme_observer(&self, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = _cx.entity();
            Some(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = f64::from(window.scale_factor());
                        let rect = accesskit::Rect::new(
                            f64::from(f32::from(bounds.left())) * scale,
                            f64::from(f32::from(bounds.top())) * scale,
                            f64::from(f32::from(bounds.right())) * scale,
                            f64::from(f32::from(bounds.bottom())) * scale,
                        );
                        if entity.read(cx).a11y_theme_bounds != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_theme_bounds = Some(rect);
                                    cx.notify();
                                })
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .into_any_element(),
            )
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }
    pub(crate) fn track_theme_focus(
        &self,
        control: gpui::Stateful<gpui::Div>,
        window: &Window,
    ) -> gpui::Stateful<gpui::Div> {
        let t = if self.dark { DARK } else { LIGHT };
        control
            .track_focus(&self.theme_focus)
            .border_1()
            .border_color(rgb(if self.theme_focus.is_focused(window) {
                t.accent
            } else {
                t.surface
            }))
    }
}
