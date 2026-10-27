//! Persistent desktop grouping navigation; no network operations or synthetic Spaces.
use crate::*;

struct RailTooltip {
    label: String,
    dark: bool,
}
impl Render for RailTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark { DARK } else { LIGHT };
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .font_family("DejaVu Sans")
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

pub(crate) struct RailUi {
    pub(crate) focus: FocusHandle,
    pub(crate) target: Option<RoomScope>,
    scroll: gpui::ScrollHandle,
    anchors: std::cell::RefCell<HashMap<Option<RoomScope>, gpui::ScrollAnchor>>,
}
impl RailUi {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            focus: cx.focus_handle(),
            target: None,
            scroll: gpui::ScrollHandle::new(),
            anchors: Default::default(),
        }
    }
}
impl Shell {
    pub(crate) fn reveal_rail_target(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(anchor) = self.rail_ui.anchors.borrow().get(&self.rail_ui.target) {
            anchor.scroll_to(window, cx);
        }
    }

    fn rail_observer(
        &self,
        _scope: &Option<RoomScope>,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let view = self.semantic_spaces(_window)?;
            let id = view.choices.iter().find(|c| c.scope == *_scope)?.id;
            Some(self.space_observer(id, _cx))
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            #[cfg(test)]
            {
                crate::sidebar_navigation_test::observe(format!("rail-{_scope:?}"))
            }
            #[cfg(not(test))]
            {
                None
            }
        }
    }

    pub(crate) fn rail_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        if window.viewport_size().width < px(800.)
            || self.notifications_open
            || self.verification_open
            || self.room_details_open
        {
            return false;
        }
        if key.modifiers.control && key.modifiers.shift && key.key == "s" {
            self.rail_ui.target = self.sidebar.active_space().cloned();
            self.rail_ui.focus.focus(window);
            cx.notify();
            return true;
        }
        if !self.rail_ui.focus.is_focused(window)
            || key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
        {
            return false;
        }
        let entries = self.sidebar.navigation_entries();
        let index = entries
            .iter()
            .position(|entry| entry.scope == self.rail_ui.target)
            .unwrap_or(0);
        let last = entries.len() - 1;
        if key.key == "tab"
            && key.modifiers.shift
            && index == 0
            && self.focus_last_chat_control(window, cx)
        {
            return true;
        }
        let next = match key.key.as_str() {
            "down" => Some((index + 1).min(last)),
            "up" => Some(index.saturating_sub(1)),
            "home" => Some(0),
            "end" => Some(last),
            "tab" if key.modifiers.shift && index > 0 => Some(index - 1),
            "tab" if !key.modifiers.shift && index < last => Some(index + 1),
            "tab" if !key.modifiers.shift => {
                self.theme_focus.focus(window);
                None
            }
            "tab" | "escape" => {
                self.search_focus.focus(window);
                None
            }
            _ => return false,
        };
        if let Some(next) = next {
            self.rail_ui.target = entries[next].scope.clone();
            self.reveal_rail_target(window, cx);
        }
        cx.notify();
        true
    }

    /// Runs on the release of a completed Space/Enter gesture (see `sidebar_activation_up`).
    pub(crate) fn rail_activate(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[2].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let entries = self.sidebar.navigation_entries();
        let index = entries
            .iter()
            .position(|entry| entry.scope == self.rail_ui.target)
            .unwrap_or(0);
        if self.sidebar.set_space(entries[index].scope.clone()) {
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        cx.notify();
    }

    pub(crate) fn render_rail(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = if self.dark { DARK } else { LIGHT };
        div()
            .id("navigation-rail")
            .track_focus(&self.rail_ui.focus)
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.rail_ui.scroll)
            .bg(rgb(t.input))
            .border_r_1()
            .border_color(rgb(t.border))
            .children(
                self.sidebar
                    .navigation_entries()
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        let scope = entry.scope;
                        let source_visit = self.sidebar.navigation_callback();
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        let callback = self.semantic_spaces(window).and_then(|view| {
                            let choice = view.choices.iter().find(|c| c.scope == scope)?;
                            Some(crate::spaces_view::space_callback(
                                matrix_desktop::accessibility::spaces::SpacesAction::Choose {
                                    revision: view.revision,
                                    account: view.account,
                                    id: choice.id,
                                    scope: scope.clone(),
                                    previous: view.active.clone(),
                                },
                            ))
                        });
                        let anchor = self
                            .rail_ui
                            .anchors
                            .borrow_mut()
                            .entry(scope.clone())
                            .or_insert_with(|| {
                                gpui::ScrollAnchor::for_handle(self.rail_ui.scroll.clone())
                            })
                            .clone();
                        let focused =
                            self.rail_ui.focus.is_focused(window) && self.rail_ui.target == scope;
                        let initials: String = if scope.is_none() {
                            "▦".into()
                        } else {
                            entry.name.chars().take(2).collect()
                        };
                        let label = if scope.is_none() {
                            "Tutte le conversazioni".to_owned()
                        } else {
                            format!("Spazio Matrix: {}", entry.name)
                        };
                        let scope_key = scope.clone();
                        let pointer =
                            move |this: &mut Shell,
                                  window: &mut Window,
                                  cx: &mut Context<Shell>| {
                                this.navigation_wide = window.viewport_size().width >= px(800.);
                                this.refresh_navigation_visibility();
                                if !source_visit(&this.sidebar) {
                                    return;
                                }
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                {
                                    if let Some(action) = callback
                                        .as_ref()
                                        .and_then(|f| f(this.semantic_spaces(window).as_ref()))
                                    {
                                        this.apply_semantic_spaces(action, window, cx);
                                    }
                                }
                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                if this.sidebar.set_space(scope.clone()) {
                                    this.rail_ui.target = scope.clone();
                                    this.rail_ui.focus.focus(window);
                                    this.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
                                    cx.notify();
                                }
                            };
                        #[cfg(test)]
                        let pointer = sidebar_navigation_test::capture_rail(&scope_key, pointer);
                        let dark = self.dark;
                        div()
                            .id(("rail-entry", index))
                            .relative()
                            .anchor_scroll(Some(anchor))
                            .tooltip(move |_, cx| {
                                cx.new(|_| RailTooltip {
                                    label: label.clone(),
                                    dark,
                                })
                                .into()
                            })
                            .w_full()
                            .h(px(84.))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .border_2()
                            .border_color(rgb(if focused { t.accent } else { t.input }))
                            .bg(rgb(if entry.selected { t.selected } else { t.input }))
                            .text_color(rgb(if entry.selected {
                                t.accent
                            } else {
                                t.secondary
                            }))
                            .cursor_pointer()
                            .children(self.rail_observer(&scope_key, window, cx))
                            .child(
                                div()
                                    .w(px(36.))
                                    .h(px(32.))
                                    .rounded_lg()
                                    .bg(rgb(t.surface))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(18.))
                                    .child(initials),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .px_1()
                                    .truncate()
                                    .text_center()
                                    .text_size(px(11.))
                                    .child(entry.name),
                            )
                            .when(entry.selected, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top(px(16.))
                                        .w(px(3.))
                                        .h(px(28.))
                                        .bg(rgb(t.accent)),
                                )
                            })
                            .when(entry.unread > 0, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(3.))
                                        .top(px(6.))
                                        .rounded_full()
                                        .px_1()
                                        .bg(rgb(t.accent))
                                        .text_color(rgb(t.on_accent))
                                        .text_size(px(10.))
                                        .child(if entry.unread > 99 {
                                            "99+".into()
                                        } else {
                                            entry.unread.to_string()
                                        }),
                                )
                            })
                            .on_click(
                                cx.listener(move |this, _, window, cx| pointer(this, window, cx)),
                            )
                    }),
            )
            .into_any_element()
    }
}
