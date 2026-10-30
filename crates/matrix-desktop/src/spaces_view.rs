//! GPUI entry to the existing joined-Space filter; no Matrix I/O.
use crate::*;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
use matrix_desktop::accessibility::spaces::{SpaceNodes, SpacesAction, SpacesSnapshot, TOGGLE};
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
use std::cell::RefCell;

#[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
pub(crate) struct SpacesUi {
    pub(crate) focus: FocusHandle,
    // None is the toggle; Some(None) is All; scoped choices survive reordering.
    pub(crate) target: SpacesTarget,
}
#[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
pub(crate) type SpacesTarget = Option<Option<RoomScope>>;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) type SpacesTarget = accesskit::NodeId;
impl Shell {
    /// Runs on the release of a completed Space/Enter gesture (see `sidebar_activation_up`).
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    pub(crate) fn spaces_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.viewport_size().width >= px(800.)
            || self.compact_chat
            || !self.connected
            || self.sidebar.spaces().next().is_none()
            || !self.spaces_ui.focus.is_focused(window)
        {
            return;
        }
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[1].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.spaces_ui.target.is_none() {
            self.set_spaces_open(!self.spaces_open);
        } else {
            let entries = self.sidebar.navigation_entries();
            let Some(entry) = entries
                .iter()
                .find(|entry| Some(entry.scope.clone()) == self.spaces_ui.target)
                .filter(|_| self.spaces_open)
            else {
                return;
            };
            if !self.sidebar.set_space(entry.scope.clone()) {
                return;
            }
            self.set_spaces_open(false);
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        self.spaces_shortcut_focus(window, cx);
    }
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    pub(crate) fn spaces_shortcut_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.spaces_ui.target = None;
        self.spaces_ui.focus.focus(window);
        cx.notify();
    }
    pub(crate) fn compact_spaces_tab(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let Some(view) = self.semantic_spaces(window) else {
                return;
            };
            let id = if self.spaces_ui.focus.is_focused(window) {
                view.next_focus(reverse)
            } else {
                let nodes = view.nodes();
                Some(nodes[if reverse { nodes.len() - 1 } else { 0 }].0)
            };
            if let Some(id) = id {
                self.apply_semantic_spaces(
                    SpacesAction::Focus {
                        revision: view.revision,
                        account: view.account,
                        id,
                    },
                    window,
                    cx,
                );
                return;
            }
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            let last = if self.spaces_open {
                self.sidebar.navigation_entries().len()
            } else {
                0
            };
            let entries = self.sidebar.navigation_entries();
            let index = entries
                .iter()
                .position(|entry| Some(entry.scope.clone()) == self.spaces_ui.target)
                .map_or(0, |index| index + 1)
                .min(last);
            let next = if !self.spaces_ui.focus.is_focused(window) {
                Some(if reverse { last } else { 0 })
            } else if reverse {
                index.checked_sub(1)
            } else {
                (index < last).then_some(index + 1)
            };
            if let Some(next) = next {
                self.spaces_ui.target = next
                    .checked_sub(1)
                    .map(|index| entries[index].scope.clone());
                self.spaces_ui.focus.focus(window);
                cx.notify();
                return;
            }
        }
        if reverse {
            if !(self.connected
                && self.focus_opener(
                    matrix_desktop::panel_openers::Control::Notifications,
                    window,
                    cx,
                ))
            {
                self.list_focus.focus(window);
            }
        } else {
            self.menu_focus.focus(window);
        }
        cx.notify();
    }
}
#[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
impl SpacesUi {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            focus: cx.focus_handle(),
            target: None,
        }
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) struct SpacesUi {
    nodes: RefCell<SpaceNodes>,
    pub(crate) focus: FocusHandle,
    pub(crate) target: SpacesTarget,
    bounds: HashMap<accesskit::NodeId, accesskit::Rect>,
    scroll: gpui::ScrollHandle,
    anchors: RefCell<HashMap<accesskit::NodeId, gpui::ScrollAnchor>>,
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
impl SpacesUi {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            nodes: Default::default(),
            focus: cx.focus_handle(),
            target: TOGGLE,
            bounds: Default::default(),
            scroll: gpui::ScrollHandle::new(),
            anchors: Default::default(),
        }
    }
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) fn space_callback(
    intent: SpacesAction,
) -> impl Fn(Option<&SpacesSnapshot>) -> Option<SpacesAction> {
    move |current| {
        current
            .filter(|view| view.allows(&intent))
            .map(|_| intent.clone())
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
impl Shell {
    pub(crate) fn spaces_shortcut_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.spaces_ui.target = TOGGLE;
        self.spaces_ui.bounds.clear();
        self.spaces_ui.focus.focus(window);
        cx.notify();
    }
    pub(crate) fn spaces_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
            return false;
        }
        let Some(view) = self.semantic_spaces(window) else {
            return false;
        };
        if self.list_focus.is_focused(window) && key.key == "tab" && key.modifiers.shift {
            let id = view.nodes().last().unwrap().0;
            self.apply_semantic_spaces(
                SpacesAction::Focus {
                    revision: view.revision,
                    account: view.account,
                    id,
                },
                window,
                cx,
            );
            return true;
        }
        if !self.spaces_ui.focus.is_focused(window) {
            return false;
        }
        match key.key.as_str() {
            "tab" => {
                if let Some(id) = view.next_focus(key.modifiers.shift) {
                    self.apply_semantic_spaces(
                        SpacesAction::Focus {
                            revision: view.revision,
                            account: view.account,
                            id,
                        },
                        window,
                        cx,
                    );
                } else {
                    if key.modifiers.shift {
                        self.search_focus.focus(window);
                    } else {
                        self.list_focus.focus(window);
                    }
                    cx.notify();
                }
            }
            _ => return false,
        }
        true
    }
    /// Runs on the release of a completed Space/Enter gesture (see `sidebar_activation_up`).
    pub(crate) fn spaces_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.semantic_spaces(window) else {
            return;
        };
        #[cfg(test)]
        crate::sidebar_navigation_test::ACTIVATIONS[1]
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(action) = view.action(&accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_node: self.spaces_ui.target,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        }) {
            self.apply_semantic_spaces(action, window, cx);
        }
    }
    pub(crate) fn semantic_spaces(&self, window: &Window) -> Option<SpacesSnapshot> {
        if self.notifications_open
            || self.verification_open
            || self.room_details_open
            || (window.viewport_size().width < px(800.) && self.compact_chat)
        {
            return None;
        }
        let persistent = window.viewport_size().width >= px(800.);
        let choices = if persistent {
            self.spaces_ui
                .nodes
                .borrow_mut()
                .rail_choices(&self.sidebar)
        } else {
            self.spaces_ui.nodes.borrow_mut().choices(&self.sidebar)
        };
        if choices.is_empty() {
            return None;
        }
        Some(SpacesSnapshot {
            revision: self.sidebar.navigation_revision(),
            persistent,
            account: self.active_scope.account_id,
            open: persistent || self.spaces_open,
            active: self.sidebar.active_space().cloned(),
            focused: if persistent {
                self.rail_ui.focus.is_focused(window).then(|| {
                    choices
                        .iter()
                        .find(|choice| choice.scope == self.rail_ui.target)
                        .unwrap_or(&choices[0])
                        .id
                })
            } else {
                self.spaces_ui
                    .focus
                    .is_focused(window)
                    .then_some(self.spaces_ui.target)
            },
            choices,
            bounds: self.spaces_ui.bounds.clone(),
        })
    }
    pub(crate) fn apply_semantic_spaces(
        &mut self,
        action: SpacesAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        let Some(view) = self
            .semantic_spaces(window)
            .filter(|view| view.allows(&action))
        else {
            return;
        };
        match action {
            SpacesAction::Focus { id, .. } => {
                if view.persistent {
                    self.rail_ui.target = view
                        .choices
                        .iter()
                        .find(|c| c.id == id)
                        .unwrap()
                        .scope
                        .clone();
                    self.reveal_rail_target(window, cx);
                }
                self.spaces_ui.target = id;
                if let Some(anchor) = self.spaces_ui.anchors.borrow().get(&id) {
                    anchor.scroll_to(window, cx);
                }
            }
            SpacesAction::SetOpen { open, .. } => {
                self.set_spaces_open(open);
                self.spaces_ui.target = TOGGLE;
                self.spaces_ui.bounds.clear();
            }
            SpacesAction::Choose { scope, .. } => {
                if !self.sidebar.set_space(scope.clone()) {
                    return;
                }
                self.set_spaces_open(false);
                self.spaces_ui.target = TOGGLE;
                if view.persistent {
                    self.rail_ui.target = scope;
                }
                self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            }
        }
        if view.persistent {
            self.rail_ui.focus.focus(window);
        } else {
            self.spaces_ui.focus.focus(window);
        }
        cx.notify();
    }
    pub(crate) fn space_observer(
        &self,
        id: accesskit::NodeId,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                #[cfg(test)]
                if crate::sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
                {
                    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
                    eprintln!(
                        "painted semantic-space {id:?}: {bounds:?}; viewport={:?}",
                        window.viewport_size()
                    );
                }
                let scale = f64::from(window.scale_factor());
                let rect = accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                );
                if entity.read(cx).spaces_ui.bounds.get(&id) != Some(&rect) {
                    let entity = entity.clone();
                    window.on_next_frame(move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.spaces_ui.bounds.insert(id, rect);
                            cx.notify();
                        })
                    });
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
    pub(crate) fn render_accessible_spaces(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(view) = self.semantic_spaces(window) else {
            return div().into_any_element();
        };
        let t = self.theme.tokens();
        let focused = view.focused;
        self.spaces_ui
            .anchors
            .borrow_mut()
            .retain(|id, _| view.choices.iter().any(|c| c.id == *id));
        let nodes = view.nodes();
        let label = nodes[0].1.label().unwrap().to_owned();
        let toggle_action = space_callback(SpacesAction::SetOpen {
            revision: view.revision,
            account: view.account,
            open: !view.open,
        });
        let toggle_pointer =
            move |this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>| {
                if let Some(action) = toggle_action(this.semantic_spaces(window).as_ref()) {
                    this.apply_semantic_spaces(action, window, cx);
                }
            };
        #[cfg(test)]
        let toggle_pointer = crate::sidebar_navigation_test::capture_space(TOGGLE, toggle_pointer);
        let toggle = div()
            .id("spaces-toggle")
            .relative()
            .h(px(32.))
            .px_4()
            .flex()
            .items_center()
            .border_1()
            .border_color(rgb(if focused == Some(TOGGLE) {
                t.accent
            } else {
                t.surface
            }))
            .text_color(rgb(t.accent))
            .cursor_pointer()
            .overflow_hidden()
            .child(label)
            .child(self.space_observer(TOGGLE, cx))
            .on_click(cx.listener(move |this, _, window, cx| toggle_pointer(this, window, cx)));
        div()
            .id("spaces-navigation")
            .track_focus(&self.spaces_ui.focus)
            .flex()
            .flex_col()
            .flex_shrink_0()
            .text_size(px(12.))
            .child(toggle)
            .when(view.open, |d| {
                d.child(
                    div()
                        .id("space-choices")
                        .max_h(px(192.))
                        .overflow_y_scroll()
                        .track_scroll(&self.spaces_ui.scroll)
                        .flex()
                        .flex_col()
                        .children(view.choices.iter().map(|choice| {
                            let id = choice.id;
                            let action = space_callback(SpacesAction::Choose {
                                revision: view.revision,
                                account: view.account,
                                id,
                                scope: choice.scope.clone(),
                                previous: view.active.clone(),
                            });
                            let choice_pointer =
                                move |this: &mut Shell,
                                      window: &mut Window,
                                      cx: &mut Context<Shell>| {
                                    if let Some(action) =
                                        action(this.semantic_spaces(window).as_ref())
                                    {
                                        this.apply_semantic_spaces(action, window, cx);
                                    }
                                };
                            #[cfg(test)]
                            let choice_pointer =
                                crate::sidebar_navigation_test::capture_space(id, choice_pointer);
                            let anchor = self
                                .spaces_ui
                                .anchors
                                .borrow_mut()
                                .entry(id)
                                .or_insert_with(|| {
                                    gpui::ScrollAnchor::for_handle(self.spaces_ui.scroll.clone())
                                })
                                .clone();
                            div()
                                .id(("accessible-space-choice", id.0))
                                .anchor_scroll(Some(anchor))
                                .relative()
                                .h(px(32.))
                                .flex_shrink_0()
                                .px_4()
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .overflow_hidden()
                                .bg(rgb(if choice.scope == view.active {
                                    t.selected
                                } else {
                                    t.input
                                }))
                                .border_1()
                                .border_color(rgb(if focused == Some(id) {
                                    t.accent
                                } else {
                                    t.border
                                }))
                                .child(choice.name.clone())
                                .child(self.space_observer(id, cx))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    choice_pointer(this, window, cx)
                                }))
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(all(test, feature = "linux-a11y", target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn painted_space_callback_keeps_its_source_visit() {
        let mut sidebar = matrix_desktop::Sidebar::demo(AccountId::new(17));
        sidebar.set_navigation_visibility(false, true, false);
        let view = SpacesSnapshot {
            revision: sidebar.navigation_revision(),
            persistent: false,
            account: AccountId::new(17),
            open: false,
            active: None,
            focused: None,
            bounds: Default::default(),
            choices: vec![],
        };
        let intent = view
            .action(&accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_node: TOGGLE,
                target_tree: accesskit::TreeId::ROOT,
                data: None,
            })
            .unwrap();
        let captured = super::space_callback(intent);
        assert!(captured(Some(&view)).is_some());
        sidebar.set_navigation_visibility(false, true, true);
        sidebar.set_navigation_visibility(false, true, false);
        let current = SpacesSnapshot {
            revision: sidebar.navigation_revision(),
            ..view
        };
        assert!(captured(Some(&current)).is_none());
        assert!(captured(None).is_none());
    }
}
