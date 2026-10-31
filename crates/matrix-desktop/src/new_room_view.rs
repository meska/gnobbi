//! «Nuova chat» (direct chat by user ID), «Nuova stanza» (name, topic, public or private) and
//! «Unisciti» (alias or room ID joins, any other text searches the public directory): a small
//! dialog opened from the drawer. The engine does the work; the result closes it or shows one error line.
use crate::*;
use matrix_desktop::new_chat::{Picker, Status};
use matrix_engine::operations::{DirectoryRoom, OperationCommand, OperationError, SearchState};

pub(crate) struct NewRoom {
    dm: bool,
    /// «Unisciti»: one field, directory hits below it.
    join: bool,
    /// Directory hits of the last search (`None` before any search) and their Tab stops.
    found: Option<Vec<DirectoryRoom>>,
    hits: Vec<FocusHandle>,
    public: bool,
    /// DM: user ID. Room: name, then topic.
    fields: Vec<gpui::Entity<crate::login_field::LoginField>>,
    public_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    busy: bool,
    error: Option<OperationError>,
    /// DM only: the search results and the keyboard position in them.
    picker: Picker,
    switch_focus: FocusHandle,
    /// What the field holds now, and the last term the engine was asked to search.
    term_wanted: String,
    term_sent: Option<(String, bool)>,
}

/// One line per failure, nothing else.
fn error_text(dm: bool, join: bool, error: OperationError) -> &'static str {
    match (error, dm, join) {
        (OperationError::InvalidInput, _, true) => "Alias non valido",
        (OperationError::Unavailable, _, true) => "Stanza non trovata",
        (OperationError::InvalidInput, true, _) => "ID utente non valido",
        (OperationError::InvalidInput, false, _) => "Nome mancante",
        (OperationError::Unavailable, true, _) => "Utente non trovato",
        (OperationError::Forbidden, ..) => "Non consentito dal server",
        _ => "Connessione non riuscita",
    }
}

impl NewRoom {
    fn title(&self) -> &'static str {
        match (self.join, self.dm) {
            (true, _) => "Unisciti",
            (_, true) => "Nuova chat",
            _ => "Nuova stanza",
        }
    }

    fn submit_label(&self) -> &'static str {
        match (self.join, self.dm) {
            (true, _) => "Vai",
            (_, true) => "Avvia",
            _ => "Crea",
        }
    }

    /// Tab order: fields, «Pubblica» (rooms only), «Annulla», submit.
    fn ring(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut ring: Vec<_> = self
            .fields
            .iter()
            .map(|f| f.read(cx).focus.clone())
            .collect();
        ring.extend(self.hits.iter().cloned());
        if !self.dm && !self.join {
            ring.push(self.public_focus.clone());
        }
        if self.dm && self.picker.all_available() {
            ring.push(self.switch_focus.clone());
        }
        ring.push(self.cancel_focus.clone());
        ring.push(self.submit_focus.clone());
        ring
    }
}

impl Shell {
    pub(crate) fn open_new_room(&mut self, dm: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(dm, false, window, cx)
    }

    pub(crate) fn open_join_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(false, true, window, cx)
    }

    fn open_dialog(&mut self, dm: bool, join: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_menu_open(false, window, cx);
        let labels: &[&'static str] = if join {
            &["Alias o nome"]
        } else if dm {
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
                cx.observe(&field, |this, field, cx| {
                    this.new_chat_typed(field.read(cx).input.display_text().trim().to_owned());
                    cx.notify();
                })
                .detach();
                field
            })
            .collect();
        fields[0].read(cx).focus.focus(window);
        self.new_room = Some(NewRoom {
            dm,
            join,
            found: None,
            hits: Vec::new(),
            public: false,
            fields,
            public_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            busy: false,
            error: None,
            picker: Picker::default(),
            switch_focus: cx.focus_handle(),
            term_wanted: String::new(),
            term_sent: None,
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
        let typed = text(0);
        // A highlighted result wins over the raw text; with no list the typed ID is used as before.
        let first = if room.dm {
            room.picker.chosen().unwrap_or(typed)
        } else {
            typed
        };
        if room.busy || first.is_empty() || self.pending_operation_command.is_some() {
            return;
        }
        let command = if room.join && first.starts_with(['#', '!']) {
            OperationCommand::JoinRoom { target: first, via: Vec::new() }
        } else if room.join {
            room.found = None;
            room.hits.clear();
            OperationCommand::SearchDirectory { term: first }
        } else if room.dm {
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

    /// A click (or Enter) on directory hit `i`: join it by alias, or by room ID when it has none.
    fn join_hit(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(room) = self.new_room.as_mut() else {
            return;
        };
        let Some(hit) = room.found.as_ref().and_then(|found| found.get(i)) else {
            return;
        };
        if room.busy || self.pending_operation_command.is_some() {
            return;
        }
        room.busy = true;
        room.error = None;
        self.pending_operation_command = Some(OperationCommand::JoinRoom { target: hit.target().to_owned(), via: vec![hit.origin.clone()] });
        cx.notify();
    }

    /// The directory answer: hits become Tab stops under the field.
    pub(crate) fn directory_found(
        &mut self,
        result: Result<Vec<DirectoryRoom>, OperationError>,
        cx: &mut Context<Self>,
    ) {
        let Some(room) = self.new_room.as_mut().filter(|room| room.busy && room.join) else {
            return;
        };
        room.busy = false;
        match result {
            Ok(found) => {
                room.hits = found.iter().map(|_| cx.focus_handle()).collect();
                room.found = Some(found);
            }
            Err(error) => room.error = Some(error),
        }
        for field in &room.fields {
            field.update(cx, |f, _| f.enabled = true);
        }
        cx.notify();
    }

    /// The field changed: the old results no longer match, so drop them until the engine answers.
    fn new_chat_typed(&mut self, term: String) {
        let Some(room) = self
            .new_room
            .as_mut()
            .filter(|r| r.dm && r.term_wanted != term)
        else {
            return;
        };
        let state = if term.chars().count() < 2 {
            SearchState::Idle
        } else {
            SearchState::Searching
        };
        room.picker.set_state(state, room.picker.all_available());
        room.term_wanted = term;
    }

    /// Sends the wanted term (and filter) to the engine once; called from the operations tick.
    pub(crate) fn flush_user_search(&mut self) {
        let Some(room) = self.new_room.as_mut().filter(|r| r.dm) else {
            return;
        };
        let wanted = (room.term_wanted.clone(), room.picker.all());
        if room.term_sent.as_ref() == Some(&wanted)
            || (room.term_sent.is_none() && wanted.0.is_empty())
        {
            return;
        }
        if let Some((sender, _)) = &mut self.routes.operation
            && sender
                .try_send(OperationCommand::SearchUsers {
                    term: wanted.0.clone(),
                    all: wanted.1,
                })
                .is_ok()
        {
            room.term_sent = Some(wanted);
        }
    }

    pub(crate) fn user_search_result(
        &mut self,
        state: SearchState,
        all_available: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(room) = self.new_room.as_mut().filter(|r| r.dm && !r.busy) {
            room.picker.set_state(state, all_available);
            cx.notify();
        }
    }

    /// Pictures the dialog's results want.
    pub(crate) fn new_chat_avatars(&self) -> Vec<String> {
        self.new_room
            .iter()
            .flat_map(|r| r.picker.entries())
            .filter_map(|e| e.avatar)
            .collect()
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_new_chat(
        &self,
        cx: &gpui::App,
    ) -> matrix_desktop::accessibility::new_chat::NewChatView {
        use matrix_desktop::accessibility::new_chat::{NewChatView, Snapshot};
        match self.new_room.as_ref().filter(|r| r.dm) {
            Some(room) => NewChatView::Open(Box::new(Snapshot {
                text: room.fields[0].read(cx).input.display_text(),
                entries: room.picker.entries(),
                active: room.picker.active(),
                status: room.picker.status(),
                all: room.picker.all_available().then(|| room.picker.all()),
            })),
            None => NewChatView::Closed,
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_new_chat(
        &mut self,
        action: matrix_desktop::accessibility::new_chat::NewChatAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::new_chat::NewChatAction;
        let Some(room) = self.new_room.as_mut().filter(|r| r.dm) else {
            return;
        };
        match action {
            NewChatAction::FocusInput => room.fields[0].read(cx).focus.focus(window),
            NewChatAction::ToggleAll => room.picker.toggle_all(),
            NewChatAction::Cancel => self.close_new_room(window, cx),
            NewChatAction::Choose(user_id) => {
                let entries = room.picker.entries();
                match entries.iter().position(|e| e.user_id == user_id) {
                    Some(at) => room.picker.set_active(at),
                    // The typed ID of «Avvia» when nothing was listed: submit it as typed.
                    None => {}
                }
                self.submit_new_room(cx);
            }

        }
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

    /// Screen-reader view of the open dialog: same controls and order as the Tab ring.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_new_room(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<matrix_desktop::accessibility::new_room::Snapshot> {
        use accesskit::Role;
        use matrix_desktop::accessibility::new_room::{Item, Snapshot};
        let room = self.new_room.as_ref()?;
        let item = |role, label: &str, checked, focus: &FocusHandle, bounds| Item {
            role,
            label: label.to_owned(),
            checked,
            focused: focus.is_focused(window),
            bounds,
        };
        let mut items: Vec<_> = room
            .fields
            .iter()
            .map(|f| {
                let f = f.read(cx);
                item(
                    Role::TextInput,
                    f.placeholder,
                    None,
                    &f.focus,
                    f.semantic_bounds,
                )
            })
            .collect();
        for (hit, focus) in room.found.iter().flatten().zip(&room.hits) {
            items.push(item(Role::Button, &hit.name, None, focus, None));
        }
        if !room.dm && !room.join {
            let focus = &room.public_focus;
            items.push(item(
                Role::CheckBox,
                "Pubblica",
                Some(room.public),
                focus,
                None,
            ));
        }
        items.push(item(
            Role::Button,
            "Annulla",
            None,
            &room.cancel_focus,
            None,
        ));
        items.push(item(Role::Button, room.submit_label(), None, &room.submit_focus, None));
        let none_found = room.found.as_ref().is_some_and(Vec::is_empty);
        if let Some(text) = room.error.map(|e| error_text(room.dm, room.join, e)).or(none_found.then_some("Nessun risultato")) {
            items.push(Item {
                role: Role::Label,
                label: text.to_owned(),
                checked: None,
                focused: false,
                bounds: None,
            });
        }
        Some(Snapshot { items })
    }

    /// Selects the room created by the dialog as soon as it is in the list.
    pub(crate) fn open_started_room(&mut self, cx: &mut Context<Self>) {
        if let Some(scope) = &self.pending_open
            && self.sidebar.is_joined(scope)
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
        } else if let Some(i) = room.hits.iter().position(|h| h.is_focused(window))
            && matches!(key, "enter" | "space")
        {
            self.join_hit(i, cx);
        } else if matches!(key, "enter" | "space") && room.public_focus.is_focused(window) {
            room.public = !room.public;
        } else if room.dm && matches!(key, "up" | "down") && room.picker.key(key) {
        } else if room.dm
            && matches!(key, "enter" | "space")
            && room.switch_focus.is_focused(window)
        {
            room.picker.toggle_all();
        } else if key == "enter" || (key == "space" && room.submit_focus.is_focused(window)) {
            self.submit_new_room(cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn render_new_chat_results(
        &self,
        room: &NewRoom,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = room.picker.active();
        let rows = room
            .picker
            .entries()
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let initial = entry
                    .name
                    .as_deref()
                    .unwrap_or(&entry.user_id)
                    .trim_start_matches('@')
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                let picture = match self.avatars.get(entry.avatar.as_deref()) {
                    Some(image) => avatar_view::round(image, 28.),
                    None => div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(28.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(rgb(t.accent))
                        .text_color(rgb(t.on_accent))
                        .child(initial)
                        .into_any_element(),
                };
                let line =
                    |text: String, color: u32| div().truncate().text_color(rgb(color)).child(text);
                div()
                    .id(("new-chat-hit", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(40.))
                    .cursor_pointer()
                    .when(i == active, |d| d.bg(rgb(t.input)))
                    .child(picture)
                    .child(if entry.direct {
                        div().min_w_0().child(line(entry.label(), t.text))
                    } else {
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(line(
                                entry.name.clone().unwrap_or_else(|| entry.user_id.clone()),
                                t.text,
                            ))
                            .child(line(entry.user_id.clone(), t.disabled).text_size(px(12.)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(room) = &mut this.new_room {
                            room.picker.set_active(i);
                        }
                        this.submit_new_room(cx);
                    }))
            });
        let switch = room.picker.all_available().then(|| {
            let segment = |id: &'static str, label: &'static str, on: bool| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if on { t.on_accent } else { t.text }))
                    .when(on, |d| d.bg(rgb(t.accent)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(room) = &mut this.new_room {
                            if room.picker.all() != on {
                                room.picker.toggle_all();
                            }
                            room.switch_focus.focus(window);
                        }
                        cx.notify();
                    }))
            };
            div()
                .id("new-chat-scope")
                .track_focus(&room.switch_focus)
                .flex()
                .gap_1()
                .border_1()
                .border_color(rgb(if room.switch_focus.is_focused(window) {
                    t.accent
                } else {
                    t.surface
                }))
                .child(segment(
                    "new-chat-same",
                    "Stessa istanza",
                    !room.picker.all(),
                ))
                .child(segment("new-chat-all", "Tutti", room.picker.all()))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(switch)
            .children(rows)
            .children(room.picker.status().map(|status| {
                div()
                    .text_size(px(13.))
                    .text_color(rgb(if status == Status::Error {
                        t.error
                    } else {
                        t.disabled
                    }))
                    .child(status.text())
            }))
            .into_any_element()
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
                                .child(room.title()),
                        )
                        .children(room.fields.iter().cloned())
                        .when(room.found.as_ref().is_some_and(|f| !f.is_empty()), |d| {
                            d.child(
                                div()
                                    .id("new-room-hits")
                                    .max_h(px(220.))
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .children(
                                        room.found.iter().flatten().zip(&room.hits).enumerate().map(
                                            |(i, (hit, focus))| {
                                                div()
                                                    .id(("new-room-hit", i))
                                                    .track_focus(focus)
                                                    .h(px(36.))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .cursor_pointer()
                                                    .border_1()
                                                    .border_color(rgb(if focus.is_focused(window) {
                                                        t.accent
                                                    } else {
                                                        t.surface
                                                    }))
                                                    .text_color(rgb(t.text))
                                                    .child(div().truncate().child(hit.name.clone()))
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .text_color(rgb(t.text))
                                                            .child(hit.members.to_string()),
                                                    )
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.join_hit(i, cx)
                                                    }))
                                            },
                                        ),
                                    ),
                            )
                        })
                        .when(!room.dm && !room.join, |d| {
                            d.child(self.render_new_chat_results(room, t, window, cx))
                        })
                        .when(!room.dm && !room.join, |d| {
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
                                .child(error_text(room.dm, room.join, error))
                        }))
                        .when(room.found.as_ref().is_some_and(Vec::is_empty), |d| {
                            d.child(div().text_size(px(13.)).text_color(rgb(t.disabled)).child("Nessun risultato"))
                        })
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
                                        room.submit_label(),
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
        assert_eq!(error_text(true, false, InvalidInput), "ID utente non valido");
        assert_eq!(error_text(false, false, InvalidInput), "Nome mancante");
        assert_eq!(error_text(true, false, Unavailable), "Utente non trovato");
        assert_eq!(error_text(false, false, Unavailable), "Connessione non riuscita");
        assert_eq!(error_text(true, false, Forbidden), "Non consentito dal server");
        assert_eq!(error_text(true, false, Transport), "Connessione non riuscita");
        assert_eq!(error_text(false, true, InvalidInput), "Alias non valido");
        assert_eq!(error_text(false, true, Unavailable), "Stanza non trovata");
    }
}
