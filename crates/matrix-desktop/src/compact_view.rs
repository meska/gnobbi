use crate::*;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
use matrix_desktop::accessibility::compact::{BackIntent, BackKey, BackNodes, BackSnapshot};

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) struct BackUi {
    pub(crate) nodes: std::cell::RefCell<BackNodes>,
    pub(crate) focus: FocusHandle,
    bounds: Option<(accesskit::NodeId, accesskit::Rect)>,
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
impl BackUi {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            nodes: Default::default(),
            focus: cx.focus_handle(),
            bounds: None,
        }
    }
}

// Capture the painted visit, never resolve a queued click against a new view.
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn back_callback(intent: BackIntent) -> impl Fn(Option<&BackSnapshot>) -> Option<BackIntent> {
    move |current| {
        current
            .filter(|view| view.allows(&intent))
            .map(|_| intent.clone())
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) fn room_callback(
    intent: matrix_desktop::accessibility::RoomIntent,
) -> impl Fn(&matrix_desktop::accessibility::RoomNodes, &matrix_desktop::Sidebar) -> Option<RoomScope>
{
    move |nodes, sidebar| nodes.resolve_action(&intent, sidebar).cloned()
}

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_room(
        &mut self,
        intent: matrix_desktop::accessibility::RoomIntent,
        focus_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigation_wide = window.viewport_size().width >= px(800.);
        self.refresh_navigation_visibility();
        if self
            .a11y_rooms
            .resolve_action(&intent, &self.sidebar)
            .is_none()
        {
            return;
        }
        if focus_only {
            self.a11y_rooms.focus_action(&intent, &self.sidebar);
        } else {
            if !self.sidebar.select(&intent.scope) {
                return;
            }
            self.sync_room(cx);
            self.set_compact_chat(true);
        }
        self.list_focus.focus(window);
        if let Some(position) = self
            .sidebar
            .visible_indices()
            .iter()
            .position(|&i| self.sidebar.rooms()[i].scope == intent.scope)
        {
            self.scroll.scroll_to_item(position, ScrollStrategy::Top);
        }
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn room_observer(
        &self,
        intent: matrix_desktop::accessibility::RoomIntent,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let scale = f64::from(window.scale_factor());
                let rect = accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                );
                let intent = intent.clone();
                let entity = entity.clone();
                window.on_next_frame(move |_, cx| {
                    entity.update(cx, |this, cx| {
                        if this.a11y_rooms.set_bounds(&intent, &this.sidebar, rect) {
                            cx.notify();
                        }
                    })
                });
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    pub(crate) fn set_compact_chat(&mut self, chat: bool) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if self.compact_chat != chat {
            self.back_ui.nodes.borrow_mut().project(None);
        }
        self.compact_chat = chat;
        self.refresh_navigation_visibility();
    }

    pub(crate) fn refresh_navigation_visibility(&mut self) {
        self.sidebar
            .set_navigation_context(self.active_scope.account_id, self.connected);
        self.refresh_openers();
        self.composer_visit.set_visible(
            !self.notifications_open
                && !self.verification_open
                && (self.navigation_wide || self.compact_chat)
                && (!self.connected || self.sidebar.selected().is_some()),
        );
        if !self.navigation_wide
            && !self.compact_chat
            && let Some(timeline) = &self.timeline
        {
            timeline.retire_event_id_copies();
            timeline.retire_offline_delivery();
        }
        self.sidebar.set_navigation_visibility(
            self.navigation_wide,
            !self.notifications_open
                && !self.verification_open
                && (self.navigation_wide || !self.compact_chat),
            self.spaces_open,
        );
    }

    pub(crate) fn set_spaces_open(&mut self, open: bool) {
        self.spaces_open = open;
        self.refresh_navigation_visibility();
    }

    pub(crate) fn set_room_details_open(&mut self, open: bool) {
        self.room_details_open = open;
        self.profile = None;
        self.refresh_navigation_visibility();
    }

    pub(crate) fn set_notifications_open(&mut self, open: bool) {
        self.notifications_open = open;
        self.refresh_navigation_visibility();
    }

    pub(crate) fn set_verification_open(&mut self, open: bool) {
        self.verification_open = open;
        self.refresh_navigation_visibility();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_back(&self, window: &Window) -> Option<BackSnapshot> {
        let scope = (window.viewport_size().width < px(800.)
            && self.compact_chat
            && !self.notifications_open
            && !self.verification_open
            && self.timeline.is_some())
        .then(|| {
            self.sidebar
                .selected()
                .filter(|r| r.scope == self.active_scope)
                .map(|r| r.scope.clone())
        })
        .flatten();
        let mut view = self.back_ui.nodes.borrow_mut().project(scope)?;
        view.focused = self.back_ui.focus.is_focused(window);
        view.bounds = self
            .back_ui
            .bounds
            .filter(|(id, _)| *id == view.id)
            .map(|(_, bounds)| bounds);
        Some(view)
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_back(
        &mut self,
        intent: BackIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .semantic_back(window)
            .is_some_and(|view| view.allows(&intent))
        {
            return;
        }
        if intent.focus_only {
            self.back_ui.focus.focus(window);
        } else {
            self.set_compact_chat(false);
            self.list_focus.focus(window);
        }
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn back_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
            return false;
        }
        let Some(view) = self.semantic_back(window) else {
            return false;
        };
        let Some(action) = view.key(
            &key.key,
            key.modifiers.shift,
            self.composer
                .read(cx)
                .semantic_snapshot(self.active_scope.clone(), window)
                .focused,
            self.list_focus.is_focused(window),
        ) else {
            return false;
        };
        match action {
            BackKey::Focus | BackKey::Activate => self.apply_semantic_back(
                BackIntent {
                    id: view.id,
                    scope: view.scope,
                    focus_only: matches!(action, BackKey::Focus),
                },
                window,
                cx,
            ),
            BackKey::Composer => self
                .composer
                .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx)),
            BackKey::List => {
                self.set_compact_chat(false);
                self.list_focus.focus(window);
            }
        }
        cx.notify();
        true
    }

    pub(crate) fn render_back(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let control = div()
            .id("back-to-rooms")
            .relative()
            .flex_shrink_0()
            .py_2()
            .cursor_pointer()
            .text_color(rgb(t.accent))
            .child("Indietro");
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let Some(view) = self.semantic_back(window) else {
                return div().into_any_element();
            };
            let id = view.id;
            let callback = back_callback(BackIntent {
                id,
                scope: view.scope,
                focus_only: false,
            });
            let entity = cx.entity();
            control
                .track_focus(&self.back_ui.focus)
                .border_1()
                .border_color(rgb(if input_modality::only_keys(view.focused, cx) { t.accent } else { t.surface }))
                .child(
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
                            if entity.read(cx).back_ui.bounds != Some((id, rect)) {
                                let entity = entity.clone();
                                window.on_next_frame(move |window, cx| {
                                    entity.update(cx, |this, cx| {
                                        if this.semantic_back(window).is_some_and(|v| v.id == id) {
                                            this.back_ui.bounds = Some((id, rect));
                                            cx.notify();
                                        }
                                    })
                                });
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .on_click(cx.listener(move |this, event, window, cx| {
                    // Key-down owns activation; key-up may hit replacement focus.
                    if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                        return;
                    }
                    if let Some(intent) = callback(this.semantic_back(window).as_ref()) {
                        this.apply_semantic_back(intent, window, cx);
                    }
                }))
                .into_any_element()
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            let focused = input_modality::only_keys(self.back_focus.is_focused(window), cx);
            control
                .track_focus(&self.back_focus)
                .border_1()
                .border_color(rgb(if focused { t.accent } else { t.surface }))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_compact_chat(false);
                    this.list_focus.focus(window);
                    cx.notify();
                }))
                .into_any_element()
        }
    }
}

#[cfg(all(test, feature = "linux-a11y", target_os = "linux"))]
mod tests {
    use matrix_desktop::accessibility::compact::{BackIntent, BackNodes};
    use matrix_domain::{AccountId, RoomId, RoomScope};
    #[test]
    fn painted_room_callback_rejects_a_returned_list_visit() {
        use matrix_desktop::{
            Sidebar,
            accessibility::{RoomIntent, RoomNodes},
        };
        let mut sidebar = Sidebar::demo(AccountId::new(17));
        sidebar.set_navigation_visibility(false, true, false);
        let mut nodes = RoomNodes::default();
        let room = nodes.project(&sidebar)[1].clone();
        let captured = super::room_callback(RoomIntent {
            id: room.id,
            scope: room.scope.clone(),
            revision: room.revision,
        });
        assert_eq!(captured(&nodes, &sidebar), Some(room.scope.clone()));
        sidebar.set_navigation_visibility(false, false, false);
        sidebar.set_navigation_visibility(false, true, false);
        nodes.project(&sidebar);
        assert!(captured(&nodes, &sidebar).is_none());
        assert_ne!(sidebar.selected().unwrap().scope, room.scope);
    }

    #[test]
    fn captured_native_back_cannot_act_on_a_replacement_visit() {
        let mut nodes = BackNodes::default();
        let scope = RoomScope {
            account_id: AccountId::new(17),
            room_id: RoomId::new("!room:test"),
        };
        for focus_only in [false, true] {
            let view = nodes.project(Some(scope.clone())).unwrap();
            let intent = BackIntent {
                id: view.id,
                scope: scope.clone(),
                focus_only,
            };
            let old = super::back_callback(intent.clone());
            for target in [
                None,
                Some(RoomScope {
                    room_id: RoomId::new("!other:test"),
                    ..scope.clone()
                }),
                Some(RoomScope {
                    account_id: AccountId::new(18),
                    ..scope.clone()
                }),
                None,
                Some(scope.clone()),
            ] {
                let replacement = nodes.project(target);
                assert!(old(replacement.as_ref()).is_none());
            }
            let replacement = nodes.project(Some(scope.clone())).unwrap();
            let current = super::back_callback(BackIntent {
                id: replacement.id,
                ..intent
            });
            assert!(current(Some(&replacement)).is_some());
            nodes.project(None);
            assert!(
                current(None).is_none(),
                "consumed visit cannot activate twice"
            );
        }
    }
}
