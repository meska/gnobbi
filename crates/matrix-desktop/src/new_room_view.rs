//! «Nuova chat» (direct chat by user ID), «Nuova stanza» (name, topic, public or private) and
//! «Unisciti» (alias or room ID joins, any other text searches the public directory): a small
//! dialog opened from the drawer. The engine does the work; the result closes it or shows one error line.
use crate::*;
use crate::switch_row::switch_row;
use matrix_desktop::directory_servers::{Choice as ServerChoice, SavedServers, ServerPicker};
use matrix_desktop::new_chat::Picker;
use crate::directory_servers_view::render_server_picker;
use matrix_desktop::search_list::{Choice, SearchList};
use matrix_engine::operations::{DirectoryRoom, OperationCommand, OperationError, SearchState};

pub(crate) struct NewRoom {
    dm: bool,
    /// «Unisciti»: one field, directory hits below it.
    join: bool,
    /// «Unisciti»: the public rooms found for the field (shared result list), the server whose
    /// directory is read (`None` when this account's server is not known) and the last request sent.
    rooms: SearchList<DirectoryRoom>,
    servers: Option<ServerPicker>,
    server_focus: FocusHandle,
    /// «Aggiungi nuovo server…»: the name field, while it is open.
    add_field: Option<gpui::Entity<crate::login_field::LoginField>>,
    rooms_sent: Option<(String, Option<String>)>,
    public: bool,
    /// DM: user ID. Room: name, then topic.
    pub(crate) fields: Vec<gpui::Entity<crate::login_field::LoginField>>,
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
pub(crate) fn error_text(dm: bool, join: bool, error: OperationError) -> &'static str {
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
        if self.servers.is_some() {
            ring.push(self.server_focus.clone());
        }
        ring.extend(self.add_field.iter().map(|f| f.read(cx).focus.clone()));
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
            rooms: SearchList::default(),
            servers: join.then(|| self.directory_picker()).flatten(),
            server_focus: cx.focus_handle(),
            add_field: None,
            rooms_sent: None,
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
            match room.rooms.chosen() {
                // From the directory: the room ID, asked through the server that listed it.
                Some(Choice::Hit(hit)) => OperationCommand::JoinRoom {
                    target: hit.room_id.clone(),
                    via: vec![hit.origin.clone()],
                },
                Some(Choice::More) => {
                    self.pending_operation_command = Some(OperationCommand::MoreRooms);
                    return;
                }
                None => return,
            }
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

    /// The server picker of «Unisciti»: this account's server first (the one searched by default).
    pub(crate) fn directory_picker(&self) -> Option<ServerPicker> {
        let server = |user_id: &str| user_id.split_once(':').map(|(_, s)| s.to_owned());
        let mine = self
            .active_id
            .and_then(|id| self.accounts.get(id))
            .and_then(|a| a.user_id.as_deref())
            .or_else(|| self.verification.profile().map(|(id, _)| id.as_str()))?;
        let others = self
            .accounts
            .accounts()
            .iter()
            .filter(|a| Some(a.id) != self.active_id)
            .filter_map(|a| server(a.user_id.as_deref()?))
            .collect();
        let saved = self.prefs_dirs.as_ref().map(SavedServers::load).unwrap_or_default();
        Some(ServerPicker::new(self.active_id.map_or(0, |id| id.as_u64()), &server(mine)?, others, saved))
    }

    /// The server picker of whichever search owns it now: the «Unisciti» dialog, else the quick
    /// search overlay. One picker implementation, one saved list.
    fn dir_servers(&mut self) -> Option<(&mut ServerPicker, &mut Option<gpui::Entity<crate::login_field::LoginField>>)> {
        if self.new_room.as_ref().is_some_and(|r| r.servers.is_some()) {
            let r = self.new_room.as_mut()?;
            return Some((r.servers.as_mut()?, &mut r.add_field));
        }
        let q = self.quick.as_mut()?;
        Some((q.servers.as_mut()?, &mut q.add_field))
    }

    /// What the server picker asked for: a new search, the name field, or the saved list to write.
    pub(crate) fn server_choice(&mut self, choice: ServerChoice, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, add)) = self.dir_servers() else {
            return;
        };
        match choice {
            ServerChoice::AddStarted => {
                let field = cx.new(|cx| {
                    let mut field = crate::login_field::LoginField::new(false, cx);
                    field.enabled = true;
                    field.placeholder = "Nome del server";
                    field
                });
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field.read(cx).focus.focus(window);
                *add = Some(field);
            }
            ServerChoice::Removed => self.save_servers(),
            ServerChoice::Selected | ServerChoice::Nothing => {}
        }
        cx.notify();
    }

    pub(crate) fn focus_dialog_field(&self, window: &mut Window, cx: &gpui::App) {
        if let Some(room) = &self.new_room {
            room.fields[0].read(cx).focus.focus(window);
        } else if let Some(q) = &self.quick {
            q.focus_field(window, cx);
        }
    }

    fn save_servers(&self) {
        let picker = self.new_room.as_ref().and_then(|r| r.servers.as_ref()).or_else(|| self.quick.as_ref().and_then(|q| q.servers.as_ref()));
        if let Some(dirs) = &self.prefs_dirs
            && let Some(picker) = picker
            && let Err(error) = picker.store().save(dirs)
        {
            eprintln!("Gnobbi: cannot save directory servers: {error}");
        }
    }

    /// Enter in the name field: probe the server (the engine answers with `server_probed`).
    pub(crate) fn confirm_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let busy = self.pending_operation_command.is_some();
        let Some((picker, add)) = self.dir_servers() else {
            return;
        };
        let Some(field) = add.clone() else {
            return;
        };
        let name = field.read(cx).input.display_text().trim().to_owned();
        let mut probe = None;
        match picker.confirm_add(&name) {
            Some(server) if !busy => probe = Some(server),
            Some(_) => picker.cancel_add(),
            None => {}
        }
        let done = !picker.is_adding();
        if done {
            *add = None;
        }
        if let Some(server) = probe {
            self.pending_operation_command = Some(OperationCommand::ProbeServer { server });
        }
        if done {
            self.focus_dialog_field(window, cx);
        }
        cx.notify();
    }

    /// Esc in the name field of «Aggiungi nuovo server…».
    pub(crate) fn cancel_server_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((picker, add)) = self.dir_servers() {
            picker.cancel_add();
            *add = None;
        }
        self.focus_dialog_field(window, cx);
        cx.notify();
    }

    pub(crate) fn server_probed(
        &mut self,
        server: String,
        result: Result<(), matrix_engine::search::SearchError>,
        cx: &mut Context<Self>,
    ) {
        let Some((picker, _)) = self.dir_servers() else {
            return;
        };
        if picker.probed(&server, result) {
            self.save_servers();
        }
        if let Some((picker, add)) = self.dir_servers()
            && !picker.is_adding()
            && add.take().is_some()
        {
            self.refocus_join_field = true;
        }
        cx.notify();
    }

    /// An engine answer for the room search; dropped when it answers another term.
    pub(crate) fn directory_result(
        &mut self,
        state: matrix_engine::search::SearchState<DirectoryRoom>,
        cx: &mut Context<Self>,
    ) {
        if let Some(room) = self.new_room.as_mut().filter(|r| r.join) {
            room.rooms.set_state(state);
            cx.notify();
        } else if self.new_room.is_none()
            && let Some(q) = self.quick.as_mut()
        {
            q.set_rooms(state);
            cx.notify();
        }
    }

    /// Sends the wanted term and server to the engine once; called from the operations tick.
    /// Nothing is asked below two characters or for an alias/ID (the engine answers Idle).
    pub(crate) fn flush_directory(&mut self) {
        let Some(room) = self.new_room.as_mut().filter(|r| r.join) else {
            return;
        };
        let term = if room.term_wanted.starts_with(['#', '!']) { String::new() } else { room.term_wanted.clone() };
        let wanted = (term, room.servers.as_ref().and_then(|p| p.value()));
        if room.rooms_sent.as_ref() == Some(&wanted) || (room.rooms_sent.is_none() && wanted.0.is_empty()) {
            return;
        }
        if let Some((sender, _)) = &mut self.routes.operation
            && sender
                .try_send(OperationCommand::SearchRooms { term: wanted.0.clone(), server: wanted.1.clone(), kind: None })
                .is_ok()
        {
            room.rooms.set_term(&wanted.0);
            room.rooms_sent = Some(wanted);
        }
    }

    /// The field changed: the old results no longer match, so drop them until the engine answers.
    fn new_chat_typed(&mut self, term: String) {
        let Some(room) = self
            .new_room
            .as_mut()
            .filter(|r| (r.dm || r.join) && r.term_wanted != term)
        else {
            return;
        };
        if room.join {
            room.rooms.set_term(if term.starts_with(['#', '!']) { "" } else { &term });
            room.term_wanted = term;
            return;
        }
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
        } else if self.new_room.is_none()
            && let Some(q) = self.quick.as_mut()
        {
            q.set_users(state);
            cx.notify();
        }
    }

    /// Pictures the dialog's results want.
    pub(crate) fn new_chat_avatars(&self) -> Vec<String> {
        self.new_room
            .iter()
            .flat_map(|r| {
                let users = r.picker.entries().into_iter().filter_map(|e| e.avatar);
                let rooms = r.rooms.rows().into_iter().filter_map(|row| match row {
                    matrix_desktop::search_list::Row::Hit(h) => h.avatar_url.clone(),
                    _ => None,
                });
                users.chain(rooms).collect::<Vec<_>>()
            })
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
                // The typed ID of «Avvia» when nothing was listed: submit it as typed.
                if let Some(at) = entries.iter().position(|e| e.user_id == user_id) {
                    room.picker.set_active(at);
                }
                self.submit_new_room(cx);
            }
        }
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_new_room(
        &mut self,
        action: matrix_desktop::accessibility::new_room::NewRoomAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::directory_servers::ServerAction;
        use matrix_desktop::accessibility::new_room::NewRoomAction;
        let Some(room) = self.new_room.as_mut().filter(|r| r.join) else {
            return self.apply_semantic_quick(action, window, cx);
        };
        match action {
            NewRoomAction::Server(ServerAction::Toggle) => {
                if let Some(p) = &mut room.servers {
                    if p.is_open() { p.close() } else { p.open() }
                    room.server_focus.focus(window);
                }
            }
            NewRoomAction::Server(ServerAction::Choose(i)) => {
                if let Some(choice) = room.servers.as_mut().map(|p| p.choose(i)) {
                    self.server_choice(choice, window, cx);
                }
            }
            NewRoomAction::Join(i) => {
                room.rooms.set_active(i);
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
        if self.quick_answered(&result, cx) {
            if let Ok(scope) = result {
                self.sidebar.keep_visible(scope.clone());
                self.pending_open = Some(scope);
                self.open_started_room(cx);
            }
            cx.notify();
            return;
        }
        if self.new_room.is_none() && std::mem::take(&mut self.profile_dm) {
            // «Invia messaggio» of a member profile: open the chat; a failure leaves the profile as it is.
            if let Ok(scope) = result {
                self.sidebar.keep_visible(scope.clone());
                self.pending_open = Some(scope);
                self.set_room_details_open(false);
                self.open_started_room(cx);
            }
            cx.notify();
            return;
        }
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
        use matrix_desktop::accessibility::new_room::{Item, Join, Snapshot};
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
        let at = items.len();
        if let Some(field) = &room.add_field {
            let f = field.read(cx);
            items.push(item(Role::TextInput, f.placeholder, None, &f.focus, f.semantic_bounds));
        }
        if !room.dm && !room.join {
            let focus = &room.public_focus;
            items.push(item(
                Role::Switch,
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
        items.push(item(
            Role::Button,
            room.submit_label(),
            None,
            &room.submit_focus,
            None,
        ));
        if let Some(error) = room.error {
            items.push(Item {
                role: Role::Label,
                label: error_text(room.dm, room.join, error).to_owned(),
                checked: None,
                focused: false,
                bounds: None,
            });
        }
        let join = room.servers.as_ref().map(|p| {
            use matrix_desktop::accessibility::directory_servers::ServerMenu;
            Join::new(
                at,
                ServerMenu::of(p),
                room.server_focus.is_focused(window),
                &room.rooms.rows(),
                room.rooms.active(),
                room.rooms.status(),
            )
        });
        Some(Snapshot { items, join })
    }

    /// Selects the room created by the dialog as soon as it is in the list.
    pub(crate) fn open_started_room(&mut self, cx: &mut Context<Self>) {
        if let Some(scope) = &self.pending_open
            && self.sidebar.is_joined(scope)
            && self.sidebar.select(scope)
        {
            self.pending_open = None;
            self.restore.cancel();
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
        } else if let Some(picker) = room.servers.as_mut().filter(|p| {
            room.server_focus.is_focused(window) && (key != "escape" || p.is_open())
        }) && let Some(choice) = picker.key(key)
        {
            self.server_choice(choice, window, cx);
        } else if room.add_field.is_some() && key == "escape" {
            if let Some(picker) = room.servers.as_mut() {
                picker.cancel_add();
            }
            room.add_field = None;
            room.fields[0].read(cx).focus.focus(window);
        } else if key == "enter"
            && room.add_field.as_ref().is_some_and(|f| f.read(cx).focus.is_focused(window))
        {
            self.confirm_server(window, cx);
        } else if key == "escape"
            || (matches!(key, "enter" | "space") && room.cancel_focus.is_focused(window))
        {
            self.close_new_room(window, cx);
        } else if room.join && matches!(key, "up" | "down") && room.rooms.key(key) {
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
        let rows = crate::search_list_view::rows(
            "new-chat-hit",
            room.picker.list(),
            t,
            &self.avatars,
            cx,
            |this, i, cx| {
                if let Some(room) = &mut this.new_room {
                    room.picker.set_active(i);
                }
                this.submit_new_room(cx);
            },
        );
        let switch = room.picker.all_available().then(|| {
            crate::switch_row::switch_row(
                "new-chat-scope",
                "Cerca anche su altre istanze",
                room.picker.all(),
                true,
                input_modality::only_keys(room.switch_focus.is_focused(window), cx),
                t,
            )
            .track_focus(&room.switch_focus)
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(room) = &mut this.new_room {
                    room.picker.toggle_all();
                    room.switch_focus.focus(window);
                }
                cx.notify();
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(switch)
            .children(rows)
            .children(crate::search_list_view::status(room.picker.list(), t))
            .into_any_element()
    }

    /// The server button and menu of «Unisciti», and the name field of «Aggiungi nuovo server…».
    fn render_server_row(
        &self,
        room: &NewRoom,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(picker) = &room.servers else {
            return div().into_any_element();
        };
        let this = cx.entity();
        let (toggle, choose, close) = (this.clone(), this.clone(), this);
        let menu = render_server_picker(
            picker,
            &room.server_focus,
            room.server_focus.is_focused(window),
            t,
            move |window, cx| {
                toggle.update(cx, |this, cx| {
                    if let Some(room) = &mut this.new_room
                        && let Some(p) = &mut room.servers
                    {
                        if p.is_open() { p.close() } else { p.open() }
                        room.server_focus.focus(window);
                    }
                    cx.notify();
                })
            },
            move |i, window, cx| {
                choose.update(cx, |this, cx| {
                    let choice = this.new_room.as_mut().and_then(|r| r.servers.as_mut()).map(|p| p.choose(i));
                    if let Some(choice) = choice {
                        this.server_choice(choice, window, cx);
                    }
                })
            },
            move |_, cx| {
                close.update(cx, |this, cx| {
                    if let Some(p) = this.new_room.as_mut().and_then(|r| r.servers.as_mut()) {
                        p.close();
                    }
                    cx.notify();
                })
            },
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(menu)
            .children(room.add_field.clone())
            .children(picker.add_error().filter(|_| room.add_field.is_some()).map(|e| {
                div().text_size(px(13.)).text_color(rgb(t.error)).child(e)
            }))
            .into_any_element()
    }

    /// Public rooms found for the field: rows, «Altri risultati», then the one status line.
    fn render_room_results(
        &self,
        room: &NewRoom,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = crate::search_list_view::rows(
            "join-hit",
            &room.rooms,
            t,
            &self.avatars,
            cx,
            |this, i, cx| {
                if let Some(room) = &mut this.new_room {
                    room.rooms.set_active(i);
                }
                this.submit_new_room(cx);
            },
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("join-hits")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .children(crate::search_list_view::status(&room.rooms, t))
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
        let kbd = input_modality::is_keyboard(cx);
        let button = |id: &'static str, label: &'static str, color: u32, focus: &FocusHandle| {
            div()
                .id(id)
                .track_focus(focus)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(rgb(if kbd && focus.is_focused(window) {
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
                .occlude()
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
                        .when(room.servers.is_some(), |d| d.child(self.render_server_row(room, t, window, cx)))
                        .when(room.join, |d| d.child(self.render_room_results(room, t, cx)))
                        .when(room.dm, |d| {
                            d.child(self.render_new_chat_results(room, t, window, cx))
                        })
                        .when(!room.dm && !room.join, |d| {
                            d.child(
                                switch_row(
                                    "new-room-public", "Pubblica", public, true,
                                    input_modality::only_keys(room.public_focus.is_focused(window), cx), t,
                                )
                                    .track_focus(&room.public_focus)
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
        assert_eq!(
            error_text(true, false, InvalidInput),
            "ID utente non valido"
        );
        assert_eq!(error_text(false, false, InvalidInput), "Nome mancante");
        assert_eq!(error_text(true, false, Unavailable), "Utente non trovato");
        assert_eq!(
            error_text(false, false, Unavailable),
            "Connessione non riuscita"
        );
        assert_eq!(
            error_text(true, false, Forbidden),
            "Non consentito dal server"
        );
        assert_eq!(
            error_text(true, false, Transport),
            "Connessione non riuscita"
        );
        assert_eq!(error_text(false, true, InvalidInput), "Alias non valido");
        assert_eq!(error_text(false, true, Unavailable), "Stanza non trovata");
    }
}
