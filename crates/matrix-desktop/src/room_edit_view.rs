//! «Modifica» from the room details sheet: name, topic and avatar through the SDK setters.
//! Same dialog shape as «Nuova stanza»; the server decides on permissions, one error line on failure.
use crate::*;
use matrix_engine::operations::{OperationCommand, OperationError};

pub(crate) struct RoomEdit {
    scope: RoomScope,
    /// Name, then topic.
    fields: Vec<gpui::Entity<crate::login_field::LoginField>>,
    original: [String; 2],
    avatar: Option<std::path::PathBuf>,
    avatar_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    busy: bool,
    error: Option<OperationError>,
}

/// `name_blank`: the name sent was empty; any other InvalidInput is the avatar file.
fn error_text(name_blank: bool, error: OperationError) -> &'static str {
    match error {
        OperationError::InvalidInput if name_blank => "Nome mancante",
        OperationError::InvalidInput => "Immagine non valida",
        OperationError::Forbidden => "Non consentito dal server",
        _ => "Connessione non riuscita",
    }
}

impl RoomEdit {
    fn ring(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut ring: Vec<_> = self.fields.iter().map(|f| f.read(cx).focus.clone()).collect();
        ring.extend([self.avatar_focus.clone(), self.cancel_focus.clone(), self.submit_focus.clone()]);
        ring
    }

    fn text(&self, i: usize, cx: &gpui::App) -> String {
        self.fields[i].read(cx).input.display_text().trim().to_owned()
    }

    fn name_blank(&self, cx: &gpui::App) -> bool {
        self.text(0, cx).is_empty() && !self.original[0].is_empty()
    }

    /// Only what differs from the room as listed.
    fn command(&self, cx: &gpui::App) -> Option<OperationCommand> {
        let changed = |i: usize| (self.text(i, cx) != self.original[i]).then(|| self.text(i, cx));
        let (name, topic) = (changed(0), changed(1));
        (name.is_some() || topic.is_some() || self.avatar.is_some()).then(|| OperationCommand::EditRoom {
            scope: self.scope.clone(),
            name,
            topic,
            avatar: self.avatar.clone(),
        })
    }
}

impl Shell {
    pub(crate) fn open_room_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(room) = self.sidebar.selected() else { return };
        let original = [room.name.clone(), room.info.topic.clone().unwrap_or_default()];
        let scope = room.scope.clone();
        let fields: Vec<_> = [("Nome", &original[0]), ("Argomento", &original[1])]
            .into_iter()
            .map(|(label, value)| {
                let field = cx.new(|cx| {
                    let mut field = crate::login_field::LoginField::new(false, cx);
                    field.enabled = true;
                    field.placeholder = label;
                    field.input.replace(None, value);
                    field
                });
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            })
            .collect();
        fields[0].read(cx).focus.focus(window);
        self.room_edit = Some(RoomEdit {
            scope,
            fields,
            original,
            avatar: None,
            avatar_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            busy: false,
            error: None,
        });
        cx.notify();
    }

    fn close_room_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.room_edit = None;
        self.list_focus.focus(window);
        cx.notify();
    }

    fn pick_room_avatar(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(edit) = &mut this.room_edit {
                        edit.avatar = paths.into_iter().next();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn submit_room_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.room_edit else { return };
        if edit.busy || self.pending_operation_command.is_some() {
            return;
        }
        let Some(command) = edit.command(cx) else { return };
        edit.busy = true;
        edit.error = None;
        for field in &edit.fields {
            field.update(cx, |f, _| f.enabled = false);
        }
        self.pending_operation_command = Some(command);
        cx.notify();
    }

    /// The engine's answer: success closes the dialog, the next sync projects the new values.
    pub(crate) fn room_edited(&mut self, result: Result<(), OperationError>, cx: &mut Context<Self>) {
        let Some(edit) = self.room_edit.as_mut().filter(|edit| edit.busy) else { return };
        edit.busy = false;
        match result {
            Ok(()) => self.room_edit = None,
            Err(error) => {
                edit.error = Some(error);
                for field in &edit.fields {
                    field.update(cx, |f, _| f.enabled = true);
                }
            }
        }
        cx.notify();
    }

    fn room_edit_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.room_edit else { return };
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            let ring = edit.ring(cx);
            let at = ring.iter().position(|h| h.is_focused(window)).unwrap_or(0);
            let next = if event.keystroke.modifiers.shift {
                (at + ring.len() - 1) % ring.len()
            } else {
                (at + 1) % ring.len()
            };
            ring[next].focus(window);
        } else if key == "escape" || (matches!(key, "enter" | "space") && edit.cancel_focus.is_focused(window)) {
            self.close_room_edit(window, cx);
        } else if matches!(key, "enter" | "space") && edit.avatar_focus.is_focused(window) {
            self.pick_room_avatar(cx);
        } else if key == "enter" || (key == "space" && edit.submit_focus.is_focused(window)) {
            self.submit_room_edit(cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    pub(crate) fn render_room_edit(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let edit = self.room_edit.as_ref()?;
        for field in &edit.fields {
            field.update(cx, |f, _| f.theme = t);
        }
        let button = |id: &'static str, label: String, color: u32, focus: &FocusHandle| {
            div()
                .id(id)
                .track_focus(focus)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(rgb(if focus.is_focused(window) { t.accent } else { t.surface }))
                .text_color(rgb(color))
                .child(label)
        };
        let avatar_label = edit
            .avatar
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("Immagine".to_owned(), |n| n.to_string_lossy().into_owned());
        Some(
            div()
                .id("room-edit-layer")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_room_edit(window, cx)))
                .child(
                    div()
                        .id("room-edit-dialog")
                        .w(px(320.))
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
                            this.room_edit_key(event, window, cx)
                        }))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(t.text))
                                .child("Modifica stanza"),
                        )
                        .children(edit.fields.iter().cloned())
                        .child(
                            button("room-edit-avatar", avatar_label, t.accent, &edit.avatar_focus)
                                .on_click(cx.listener(|this, _, _, cx| this.pick_room_avatar(cx))),
                        )
                        .children(edit.error.map(|error| {
                            div().text_size(px(13.)).text_color(rgb(t.error)).child(error_text(edit.name_blank(cx), error))
                        }))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("room-edit-cancel", "Annulla".into(), t.accent, &edit.cancel_focus)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_room_edit(window, cx)
                                        })),
                                )
                                .child(
                                    button(
                                        "room-edit-submit",
                                        "Salva".into(),
                                        if edit.busy { t.disabled } else { t.accent },
                                        &edit.submit_focus,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.submit_room_edit(cx))),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_per_failure() {
        assert_eq!(error_text(true, OperationError::InvalidInput), "Nome mancante");
        assert_eq!(error_text(false, OperationError::InvalidInput), "Immagine non valida");
        assert_eq!(error_text(false, OperationError::Forbidden), "Non consentito dal server");
        assert_eq!(error_text(false, OperationError::Transport), "Connessione non riuscita");
    }
}
