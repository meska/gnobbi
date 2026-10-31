//! Keyboard and assistive entry to the existing session-only unread filter.
use crate::*;

/// Control that received the key-down; the release only activates the same, still current source.
#[derive(PartialEq)]
pub(crate) enum PressKind {
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
    pub(crate) fn sidebar_pointer_callback(
        &self,
    ) -> impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + use<> {
        let source = self.sidebar.navigation_callback();
        move |this, window, cx| {
            this.navigation_wide = window.viewport_size().width >= px(800.);
            this.refresh_navigation_visibility();
            if !source(&this.sidebar) {
                return;
            }
            this.toggle_unread(window, cx);
        }
    }

    fn toggle_unread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[0].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
                "Shell capture down key={} held={} unread={} revision={}",
                key.key,
                event.is_held,
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
        } else if self.navigation_wide && self.rail_ui.focus.is_focused(window) {
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
        // GPUI's automatic key-up Click does not retain the key-down source.
        if self.unread_focus.is_focused(window) || pending.is_some() {
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

    /// First stop of the list column: the unread filter once connected, else the search field.
    pub(crate) fn first_list_stop(&self) -> &FocusHandle {
        if self.connected { &self.unread_focus } else { &self.search_focus }
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
        let target = if self.connected && self.unread_focus.is_focused(window) && !key.modifiers.shift {
            &self.search_focus
        } else if self.search_focus.is_focused(window) && key.modifiers.shift && self.connected {
            &self.unread_focus
        } else if (self.unread_focus.is_focused(window) && self.connected
            || self.search_focus.is_focused(window) && !self.connected)
            && key.modifiers.shift
        {
            // Shift-Tab from the first stop of the list column leaves it backwards.
            if compact {
                &self.menu_focus
            } else {
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
            sidebar_navigation_test::ACTIVATIONS[0]
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.sidebar.set_unread_only(checked);
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        self.unread_focus.focus(window);
        cx.notify();
    }

    /// Personal groups as chips under the search field; pointer only (no tab stop yet).
    pub(crate) fn render_groups(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        div()
            .id("room-groups")
            .flex()
            .flex_wrap()
            .gap_2()
            .mx_4()
            .mb_3()
            .children(self.sidebar.group_entries().into_iter().enumerate().map(|(i, entry)| {
                let tag = entry.tag.clone();
                div()
                    .id(("room-group", i))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .bg(rgb(if entry.selected { t.accent } else { t.input }))
                    .text_color(rgb(if entry.selected { t.on_accent } else { t.secondary }))
                    .child(entry.name)
                    .when(entry.unread > 0, |d| {
                        d.child(
                            div()
                                .px_1()
                                .rounded_full()
                                .bg(rgb(if entry.selected { t.on_accent } else { t.accent }))
                                .text_color(rgb(if entry.selected { t.accent } else { t.on_accent }))
                                .child(entry.unread.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar.set_group(tag.clone());
                        this.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    pub(crate) fn render_unread(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = self.theme.tokens();
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
                let callback = self.sidebar_pointer_callback();
                #[cfg(test)]
                let callback = sidebar_navigation_test::capture_pointer(callback);
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
}
