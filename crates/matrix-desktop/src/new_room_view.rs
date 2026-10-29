//! «Nuova chat» (direct chat by user ID) and «Nuova stanza» (name, public or private): a small
//! dialog opened from the drawer. The engine does the work; the result only closes it or shows an error.
use crate::*;
use matrix_engine::operations::{OperationCommand, OperationError};

pub(crate) struct NewRoom {
    dm: bool,
    public: bool,
    field: gpui::Entity<crate::login_field::LoginField>,
    busy: bool,
    error: bool,
}

impl Shell {
    pub(crate) fn open_new_room(&mut self, dm: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_menu_open(false, window, cx);
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        self.new_room = Some(NewRoom {
            dm,
            public: false,
            field,
            busy: false,
            error: false,
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
        let text = room.field.read(cx).input.display_text().trim().to_owned();
        if room.busy || text.is_empty() || self.pending_operation_command.is_some() {
            return;
        }
        room.busy = true;
        room.error = false;
        self.pending_operation_command = Some(if room.dm {
            OperationCommand::StartDm { user_id: text }
        } else {
            OperationCommand::CreateRoom {
                name: text,
                topic: None,
                public: room.public,
            }
        });
        cx.notify();
    }

    /// The engine's answer: success closes the dialog (the room arrives with the next sync).
    pub(crate) fn room_started(
        &mut self,
        result: Result<RoomScope, OperationError>,
        cx: &mut Context<Self>,
    ) {
        let Some(room) = &mut self.new_room else {
            return;
        };
        room.busy = false;
        match result {
            Ok(scope) => {
                self.sidebar.keep_visible(scope);
                self.new_room = None;
                self.refocus_menu = true;
            }
            Err(_) => room.error = true,
        }
        cx.notify();
    }

    pub(crate) fn render_new_room(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let room = self.new_room.as_ref()?;
        room.field.update(cx, |f, _| f.theme = t);
        let button = |id: &'static str, label: &'static str, color: u32| {
            div()
                .id(id)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
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
                            match event.keystroke.key.as_str() {
                                "enter" => this.submit_new_room(cx),
                                "escape" => this.close_new_room(window, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
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
                        .child(room.field.clone())
                        .when(!room.dm, |d| {
                            d.child(
                                div()
                                    .id("new-room-public")
                                    .h(px(36.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .cursor_pointer()
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
                                            // Keep the keyboard in the dialog (Enter creates, Esc closes).
                                            room.field.read(cx).focus.focus(window);
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .when(room.error, |d| {
                            d.child(div().text_size(px(13.)).text_color(rgb(t.error)).child(
                                if room.dm {
                                    "Non riuscito. Controlla l'ID utente e riprova."
                                } else {
                                    "Non riuscito. Riprova."
                                },
                            ))
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(button("new-room-cancel", "Annulla", t.accent).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.close_new_room(window, cx)
                                    }),
                                ))
                                .child(
                                    button(
                                        "new-room-submit",
                                        if room.dm { "Avvia" } else { "Crea" },
                                        if room.busy { t.disabled } else { t.accent },
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
