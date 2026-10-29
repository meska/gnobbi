//! «Nuova chat» (direct chat by user ID) and «Nuova stanza» (name, topic, public or private): a small
//! dialog opened from the drawer. The engine does the work; the result closes it or shows one error line.
use crate::*;
use matrix_engine::operations::{OperationCommand, OperationError};

pub(crate) struct NewRoom {
    dm: bool,
    public: bool,
    /// DM: user ID. Room: name, then topic.
    fields: Vec<gpui::Entity<crate::login_field::LoginField>>,
    public_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    busy: bool,
    error: Option<OperationError>,
}

/// One line per failure, nothing else.
fn error_text(dm: bool, error: OperationError) -> &'static str {
    match (error, dm) {
        (OperationError::InvalidInput, true) => "ID utente non valido",
        (OperationError::InvalidInput, false) => "Nome mancante",
        (OperationError::Unavailable, true) => "Utente non trovato",
        (OperationError::Forbidden, _) => "Non consentito dal server",
        _ => "Connessione non riuscita",
    }
}

impl NewRoom {
    /// Tab order: fields, «Pubblica» (rooms only), «Annulla», submit.
    fn ring(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut ring: Vec<_> = self
            .fields
            .iter()
            .map(|f| f.read(cx).focus.clone())
            .collect();
        if !self.dm {
            ring.push(self.public_focus.clone());
        }
        ring.push(self.cancel_focus.clone());
        ring.push(self.submit_focus.clone());
        ring
    }
}

impl Shell {
    pub(crate) fn open_new_room(&mut self, dm: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_menu_open(false, window, cx);
        let labels: &[&'static str] = if dm {
            &["ID utente"]
        } else {
            &["Nome", "Argomento"]
        };
        let fields: Vec<_> = labels
            .iter()
            .map(|&label| {
                let field = cx.new(|cx| {
                    let mut field = crate::login_field::LoginField::new(false, cx);
                    field.enabled = true;
                    field.placeholder = label;
                    field
                });
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            })
            .collect();
        fields[0].read(cx).focus.focus(window);
        self.new_room = Some(NewRoom {
            dm,
            public: false,
            fields,
            public_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            busy: false,
            error: None,
        });
        cx.notify();
    }

    fn close_new_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_room = None;
        self.menu_focus.focus(window);
        cx.notify();
    }

    fn submit_new_room(&mut self, cx: &mut Context<Self>) {
        let Some(room) = &mut self.new_room else {
            return;
        };
        let text = |i: usize| {
            room.fields[i]
                .read(cx)
                .input
                .display_text()
                .trim()
                .to_owned()
        };
        let first = text(0);
        if room.busy || first.is_empty() || self.pending_operation_command.is_some() {
            return;
        }
        let command = if room.dm {
            OperationCommand::StartDm { user_id: first }
        } else {
            let topic = text(1);
            OperationCommand::CreateRoom {
                name: first,
                topic: (!topic.is_empty()).then_some(topic),
                public: room.public,
            }
        };
        room.busy = true;
        room.error = None;
        for field in &room.fields {
            field.update(cx, |f, _| f.enabled = false);
        }
        self.pending_operation_command = Some(command);
        cx.notify();
    }

    /// The engine's answer: success closes the dialog and opens the room once the sync lists it.
    pub(crate) fn room_started(
        &mut self,
        result: Result<RoomScope, OperationError>,
        cx: &mut Context<Self>,
    ) {
        // A late answer for a dialog the user already closed (or reopened) is ignored.
        let Some(room) = self.new_room.as_mut().filter(|room| room.busy) else {
            return;
        };
        room.busy = false;
        match result {
            Ok(scope) => {
                self.sidebar.keep_visible(scope.clone());
                self.new_room = None;
                self.refocus_menu = true;
                self.pending_open = Some(scope);
                self.open_started_room(cx);
            }
            Err(error) => {
                room.error = Some(error);
                for field in &room.fields {
                    field.update(cx, |f, _| f.enabled = true);
                }
            }
        }
        cx.notify();
    }

    /// Selects the room created by the dialog as soon as it is in the list.
    pub(crate) fn open_started_room(&mut self, cx: &mut Context<Self>) {
        if let Some(scope) = &self.pending_open
            && self.sidebar.select(scope)
        {
            self.pending_open = None;
            self.sync_room(cx);
            self.set_compact_chat(true);
            cx.notify();
        }
    }

    fn new_room_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(room) = &mut self.new_room else {
            return;
        };
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            let ring = room.ring(cx);
            let at = ring.iter().position(|h| h.is_focused(window)).unwrap_or(0);
            let next = if event.keystroke.modifiers.shift {
                (at + ring.len() - 1) % ring.len()
            } else {
                (at + 1) % ring.len()
            };
            ring[next].focus(window);
        } else if key == "escape"
            || (matches!(key, "enter" | "space") && room.cancel_focus.is_focused(window))
        {
            self.close_new_room(window, cx);
        } else if matches!(key, "enter" | "space") && room.public_focus.is_focused(window) {
            room.public = !room.public;
        } else if key == "enter" || (key == "space" && room.submit_focus.is_focused(window)) {
            self.submit_new_room(cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    pub(crate) fn render_new_room(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let room = self.new_room.as_ref()?;
        for field in &room.fields {
            field.update(cx, |f, _| f.theme = t);
        }
        let button = |id: &'static str, label: &'static str, color: u32, focus: &FocusHandle| {
            div()
                .id(id)
                .track_focus(focus)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(rgb(if focus.is_focused(window) {
                    t.accent
                } else {
                    t.surface
                }))
                .text_color(rgb(color))
                .child(label)
        };
        let public = room.public;
        Some(
            div()
                .id("new-room-layer")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_new_room(window, cx)))
                .child(
                    div()
                        .id("new-room-dialog")
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
                            this.new_room_key(event, window, cx)
                        }))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(t.text))
                                .child(if room.dm {
                                    "Nuova chat"
                                } else {
                                    "Nuova stanza"
                                }),
                        )
                        .children(room.fields.iter().cloned())
                        .when(!room.dm, |d| {
                            d.child(
                                div()
                                    .id("new-room-public")
                                    .track_focus(&room.public_focus)
                                    .h(px(36.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(rgb(if room.public_focus.is_focused(window) {
                                        t.accent
                                    } else {
                                        t.surface
                                    }))
                                    .text_color(rgb(t.text))
                                    .child("Pubblica")
                                    .child(
                                        div()
                                            .w(px(36.))
                                            .h(px(20.))
                                            .rounded_full()
                                            .flex()
                                            .items_center()
                                            .px(px(2.))
                                            .bg(rgb(if public { t.accent } else { t.disabled }))
                                            .when(public, |d| d.justify_end())
                                            .child(
                                                div()
                                                    .size(px(16.))
                                                    .rounded_full()
                                                    .bg(rgb(t.on_accent)),
                                            ),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(room) = &mut this.new_room {
                                            room.public = !room.public;
                                            room.public_focus.focus(window);
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .children(room.error.map(|error| {
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(t.error))
                                .child(error_text(room.dm, error))
                        }))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button(
                                        "new-room-cancel",
                                        "Annulla",
                                        t.accent,
                                        &room.cancel_focus,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.close_new_room(window, cx),
                                    )),
                                )
                                .child(
                                    button(
                                        "new-room-submit",
                                        if room.dm { "Avvia" } else { "Crea" },
                                        if room.busy { t.disabled } else { t.accent },
                                        &room.submit_focus,
                                    )
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.submit_new_room(cx)),
                                    ),
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
        use OperationError::*;
        assert_eq!(error_text(true, InvalidInput), "ID utente non valido");
        assert_eq!(error_text(false, InvalidInput), "Nome mancante");
        assert_eq!(error_text(true, Unavailable), "Utente non trovato");
        assert_eq!(error_text(false, Unavailable), "Connessione non riuscita");
        assert_eq!(error_text(true, Forbidden), "Non consentito dal server");
        assert_eq!(error_text(true, Transport), "Connessione non riuscita");
    }
}
