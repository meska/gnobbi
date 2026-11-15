//! «Gruppi» dialog of the «Modifica» panel: create, rename and delete groups and put rooms in them.
//! Every change is one explicit click or Enter and goes to the server as a standard room tag through
//! the engine; nothing here writes by itself. The lists come from the tags the sync already delivered.
use crate::switch_row::switch_row;
use crate::*;
use matrix_engine::operations::{OperationCommand, OperationError, TagEdit};

pub(crate) struct GroupsEdit {
    field: gpui::Entity<crate::login_field::LoginField>,
    /// Tag being edited (`m.favourite`, `m.lowpriority` or `u.*`).
    selected: String,
    /// Groups created here that no room carries yet (a group exists on the server once a room has its tag).
    drafts: Vec<String>,
    busy: bool,
    error: bool,
}

impl Shell {
    fn groups_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Entity<crate::login_field::LoginField> {
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        field
    }

    pub(crate) fn open_groups_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.groups_field(window, cx);
        self.groups_edit = Some(GroupsEdit { field, selected: "m.favourite".into(), drafts: Vec::new(), busy: false, error: false });
        cx.notify();
    }

    fn close_groups_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.groups_edit = None;
        self.rail_ui.edit_focus.focus(window);
        cx.notify();
    }

    /// Standard groups first, then the user groups some room carries, then the drafts.
    fn group_tags(&self) -> Vec<String> {
        let Some(edit) = &self.groups_edit else { return Vec::new() };
        let mut user: Vec<String> = self.sidebar.rooms().iter().flat_map(|r| r.info.tags.iter()).chain(edit.drafts.iter())
            .filter(|t| t.starts_with("u.")).cloned().collect();
        user.sort();
        user.dedup();
        ["m.favourite", "m.lowpriority"].map(String::from).into_iter().chain(user).collect()
    }

    /// Sends one tag change; refused while another command waits. Returns whether it was queued.
    fn send_tag_edit(&mut self, edit: TagEdit, cx: &mut Context<Self>) -> bool {
        let Some(state) = &mut self.groups_edit else { return false };
        if state.busy || self.pending_operation_command.is_some() || self.sidebar.tag_pending() {
            return false;
        }
        state.busy = true;
        state.error = false;
        self.pending_operation_command = Some(OperationCommand::EditTags(edit));
        cx.notify();
        true
    }

    /// The one favourite / low-priority action for the room menu and the drawer: optimistic flip, the tag through the SDK.
    /// Refused while another tag command is in flight.
    pub(crate) fn toggle_tag(&mut self, scope: &RoomScope, tag: &'static str, cx: &mut Context<Self>) {
        if self.pending_operation_command.is_some() || self.groups_edit.as_ref().is_some_and(|e| e.busy) {
            return;
        }
        if let Some(edit) = self.sidebar.toggle_tag(scope, tag) {
            self.pending_operation_command = Some(OperationCommand::EditTags(edit));
            cx.notify();
        }
    }

    /// Typed name → `u.<name>` (None for an empty name).
    fn typed_group(&self, cx: &Context<Self>) -> Option<String> {
        let text = self.groups_edit.as_ref()?.field.read(cx).input.display_text();
        let name = text.trim();
        (!name.is_empty()).then(|| format!("u.{name}"))
    }

    fn groups_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tag) = self.typed_group(cx) else { return };
        let field = self.groups_field(window, cx);
        if let Some(edit) = &mut self.groups_edit {
            if !edit.drafts.contains(&tag) {
                edit.drafts.push(tag.clone());
            }
            edit.selected = tag;
            edit.field = field;
        }
        cx.notify();
    }

    fn groups_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(to), Some(edit)) = (self.typed_group(cx), &self.groups_edit) else { return };
        let from = edit.selected.clone();
        if !from.starts_with("u.") || from == to {
            return;
        }
        let queued = if edit.drafts.contains(&from) && !self.sidebar.rooms().iter().any(|r| r.info.tags.contains(&from)) {
            true // nothing on the server yet: only the local draft changes
        } else {
            self.send_tag_edit(TagEdit::Rename { from: from.clone(), to: to.clone() }, cx)
        };
        if queued {
            let field = self.groups_field(window, cx);
            if let Some(edit) = &mut self.groups_edit {
                edit.drafts.retain(|d| *d != from);
                edit.drafts.push(to.clone());
                edit.selected = to;
                edit.field = field;
            }
            cx.notify();
        }
    }

    fn groups_delete(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &self.groups_edit else { return };
        let tag = edit.selected.clone();
        if !tag.starts_with("u.") {
            return;
        }
        let on_server = self.sidebar.rooms().iter().any(|r| r.info.tags.contains(&tag));
        if !on_server || self.send_tag_edit(TagEdit::Delete { tag: tag.clone() }, cx) {
            if let Some(edit) = &mut self.groups_edit {
                edit.drafts.retain(|d| *d != tag);
                edit.selected = "m.favourite".into();
            }
            cx.notify();
        }
    }

    fn groups_toggle_room(&mut self, room_id: String, on: bool, cx: &mut Context<Self>) {
        let Some(edit) = &self.groups_edit else { return };
        let tag = edit.selected.clone();
        self.send_tag_edit(TagEdit::Assign { room_id, tag, on }, cx);
    }

    /// The engine's answer. The lists change with the next sync; only the busy flag and the error live here.
    pub(crate) fn groups_edited(&mut self, result: Result<(), OperationError>, cx: &mut Context<Self>) {
        if self.sidebar.tag_pending() {
            self.sidebar.tag_done(result);
            cx.notify();
            return;
        }
        if let Some(edit) = &mut self.groups_edit {
            edit.busy = false;
            edit.error = result.is_err();
        }
        cx.notify();
    }

    pub(crate) fn render_groups_edit(&self, t: matrix_desktop::DesignTokens, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let edit = self.groups_edit.as_ref()?;
        edit.field.update(cx, |f, _| f.theme = t);
        let user_group = edit.selected.starts_with("u.");
        let button = |id: &'static str, label: &'static str, color: u32| {
            div().id(id).px_3().py_2().rounded_md().cursor_pointer().text_color(rgb(color)).child(label)
        };
        let tags = self.group_tags();
        let mut rooms: Vec<_> = self.sidebar.rooms().iter().filter(|r| !r.info.is_space).collect();
        rooms.sort_by_key(|r| r.name.to_lowercase());
        let busy_color = if edit.busy { t.disabled } else { t.accent };
        Some(
            div()
                .id("groups-layer")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_groups_edit(window, cx)))
                .child(
                    div()
                        .id("groups-dialog")
                        .w(px(360.))
                        .max_h((window.viewport_size().height - px(48.)).min(px(520.)))
                        .p_4()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "enter" => this.groups_create(window, cx),
                                "escape" => this.close_groups_edit(window, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        .child(
                            div().flex().items_center().justify_between()
                                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.text)).child("Gruppi"))
                                .child(
                                    div().id("groups-close").w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer()
                                        .hover(|s| s.bg(rgb(t.border)))
                                        .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                                        .on_click(cx.listener(|this, _, window, cx| this.close_groups_edit(window, cx))),
                                ),
                        )
                        .child(div().flex().flex_wrap().gap_1().children(tags.into_iter().enumerate().map(|(i, tag)| {
                            let on = tag == edit.selected;
                            let label = matrix_desktop::group_label(&tag);
                            div()
                                .id(("groups-tag", i))
                                .px_3()
                                .py_1()
                                .rounded_full()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .bg(rgb(if on { t.accent } else { t.input }))
                                .text_color(rgb(if on { t.on_accent } else { t.text }))
                                .child(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(edit) = &mut this.groups_edit {
                                        edit.selected = tag.clone();
                                        edit.field.read(cx).focus.focus(window);
                                    }
                                    cx.notify();
                                }))
                        })))
                        .child(edit.field.clone())
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(button("groups-create", "Crea", t.accent).on_click(cx.listener(|this, _, window, cx| this.groups_create(window, cx))))
                                .when(user_group, |d| {
                                    d.child(button("groups-rename", "Rinomina", busy_color).on_click(cx.listener(|this, _, window, cx| this.groups_rename(window, cx))))
                                        .child(button("groups-delete", "Elimina", if edit.busy { t.disabled } else { t.error }).on_click(cx.listener(|this, _, _, cx| this.groups_delete(cx))))
                                }),
                        )
                        .child(
                            div().id("groups-rooms").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().children(rooms.into_iter().enumerate().map(|(i, room)| {
                                let has = room.info.tags.contains(&edit.selected);
                                let room_id = room.scope.room_id.as_str().to_owned();
                                switch_row(("groups-room", i), room.name.clone(), has, true, false, t)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.groups_toggle_room(room_id.clone(), !has, cx);
                                        if let Some(edit) = &this.groups_edit {
                                            edit.field.read(cx).focus.focus(window);
                                        }
                                    }))
                            })),
                        )
                        .when(edit.error, |d| d.child(div().text_size(px(13.)).text_color(rgb(t.error)).child("Non riuscito. Riprova."))),
                )
                .into_any_element(),
        )
    }
}
