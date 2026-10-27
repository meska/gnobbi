//! Context menu of a room row: «Segna come letto» / «Segna come non letto».
use crate::*;

pub(crate) const ITEMS: [&str; 2] = ["Segna come letto", "Segna come non letto"];

impl Shell {
    /// Queues the room-level read marker for the engine; reading also drops the badge at once.
    pub(crate) fn mark_room(&mut self, scope: RoomScope, read: bool, cx: &mut Context<Self>) {
        self.room_menu = None;
        if self.pending_operation_command.is_none() {
            self.pending_operation_command = Some(matrix_engine::operations::OperationCommand::MarkRoom {
                scope: scope.clone(),
                read,
                public: self.prefs.read_receipts,
            });
            if read {
                self.sidebar.mark_read_optimistic(&scope);
            }
        }
        cx.notify();
    }

    pub(crate) fn render_room_menu(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some((scope, at)) = self.room_menu.clone() else { return div().into_any_element() };
        let item = |i: usize| {
            let scope = scope.clone();
            div().id(("room-menu-item", i)).px_4().py_2().cursor_pointer().text_size(px(13.)).text_color(rgb(t.text))
                .hover(|s| s.bg(rgb(t.border))).child(ITEMS[i])
                .on_click(cx.listener(move |this, _, _, cx| this.mark_room(scope.clone(), i == 0, cx)))
        };
        gpui::deferred(gpui::anchored().position(at).snap_to_window_with_margin(px(8.)).child(
            div().id("room-menu").occlude().flex().flex_col().py_1().rounded_md().border_1().border_color(rgb(t.border)).bg(rgb(t.surface)).min_w(px(180.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| { this.room_menu = None; cx.notify(); }))
                .child(item(0)).child(item(1)),
        )).with_priority(2).into_any_element()
    }
}
