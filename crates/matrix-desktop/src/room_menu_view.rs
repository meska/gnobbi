//! Context menu of a room row: «Segna come letto» / «Segna come non letto» / «Aggiungi/Togli dai preferiti» / «Esci» (asks «Uscire?» once).
use crate::*;

pub(crate) const ITEMS: [&str; 2] = ["Segna come letto", "Segna come non letto"];

impl Shell {
    /// Queues the room-level read marker for the engine; reading also drops the badge at once.
    pub(crate) fn mark_room(&mut self, scope: RoomScope, read: bool, main_only: bool, cx: &mut Context<Self>) {
        self.room_menu = None;
        if self.pending_operation_command.is_none() {
            self.pending_operation_command =
                Some(matrix_engine::operations::OperationCommand::MarkRoom {
                    scope: scope.clone(),
                    read,
                    public: self.prefs.read_receipts,
                    main_only,
                });
            if read {
                self.sidebar.mark_read_optimistic(&scope);
            }
        }
        cx.notify();
    }

    /// Opening or receiving in the room on screen: send the marker when the newest message is in view.
    pub(crate) fn mark_open_room_read(&mut self, cx: &mut Context<Self>) {
        if !self.read_dirty || !self.connected || self.pending_operation_command.is_some() {
            return;
        }
        let (Some(room), Some(timeline)) = (self.sidebar.selected(), &self.timeline) else {
            return;
        };
        if cx.active_window().is_none() || !timeline.at_bottom() {
            return;
        }
        let scope = room.scope.clone();
        self.read_dirty = false;
        self.mark_room(scope, true, true, cx);
    }

    /// First click arms «Uscire?», the second leaves; the sync then drops the room from the list.
    pub(crate) fn leave_room(&mut self, scope: RoomScope, cx: &mut Context<Self>) {
        if self.confirm_leave.as_ref() != Some(&scope) {
            self.confirm_leave = Some(scope);
        } else if self.pending_operation_command.is_none() {
            self.confirm_leave = None;
            self.room_menu = None;
            self.pending_operation_command =
                Some(matrix_engine::operations::OperationCommand::LeaveRoom { scope });
        }
        cx.notify();
    }

    /// Room rows for the screen-reader tree; the row with the open menu gets its «Esci» button.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_rooms(&mut self) -> Vec<matrix_desktop::accessibility::SemanticRoom> {
        let mut rooms = self.a11y_rooms.project(&self.sidebar);
        if let Some((scope, _)) = &self.room_menu {
            let confirming = self.confirm_leave.as_ref() == Some(scope);
            for room in rooms.iter_mut().filter(|room| room.scope == *scope) {
                room.leave = Some(confirming);
            }
        }
        rooms
    }

    pub(crate) fn render_room_menu(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some((scope, at)) = self.room_menu.clone() else {
            return div().into_any_element();
        };
        let item = |i: usize| {
            let scope = scope.clone();
            div()
                .id(("room-menu-item", i))
                .px_4()
                .py_2()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(t.text))
                .hover(|s| s.bg(rgb(t.border)))
                .child(ITEMS[i])
                .on_click(
                    cx.listener(move |this, _, _, cx| this.mark_room(scope.clone(), i == 0, false, cx)),
                )
        };
        gpui::deferred(
            gpui::anchored()
                .position(at)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("room-menu")
                        .occlude()
                        .flex()
                        .flex_col()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(t.border))
                        .bg(rgb(t.surface))
                        .min_w(px(180.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.room_menu = None;
                            this.confirm_leave = None;
                            cx.notify();
                        }))
                        .child(item(0))
                        .child(item(1))
                        .child({
                            let scope = scope.clone();
                            let label = self.sidebar.favourite_label(&scope);
                            div()
                                .id("room-menu-favourite")
                                .px_4()
                                .py_2()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(rgb(t.text))
                                .hover(|s| s.bg(rgb(t.border)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.room_menu = None;
                                    this.toggle_favourite(&scope, cx);
                                }))
                        })
                        .child({
                            let scope = scope.clone();
                            let confirming = self.confirm_leave.as_ref() == Some(&scope);
                            div()
                                .id("room-menu-leave")
                                .px_4()
                                .py_2()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(rgb(t.error))
                                .hover(|s| s.bg(rgb(t.border)))
                                .child(if confirming { "Uscire?" } else { "Esci" })
                                .on_click(cx.listener(move |this, _, _, cx| this.leave_room(scope.clone(), cx)))
                        }),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}
