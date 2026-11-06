//! Message search of the open room: a field in the chat header (the lens opens it), the results as
//! a list under the header, Enter/click jumps to the message and flashes it. Esc closes the list,
//! then the search, and the history is back where it was. The engine's `MessageSearch` does the
//! work (server `/search` for unencrypted rooms, the decrypted local cache for encrypted ones).
use crate::*;
use gpui::{HighlightStyle, StyledText};
use matrix_desktop::new_chat::Status;
use matrix_domain::EventId;
use matrix_engine::operations::{MessageHit, MessageSearchState, MoreHits, OperationCommand};

/// How long the bubble flashes after a jump.
const FLASH: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone)]
struct Jump {
    /// The row to show: the hit, or the thread root for a reply inside a thread.
    show: EventId,
    open_thread: bool,
    term: String,
}

pub(crate) struct RoomSearch {
    scope: RoomScope,
    field: gpui::Entity<crate::login_field::LoginField>,
    state: MessageSearchState,
    active: usize,
    list_open: bool,
    /// What the field holds now, and the last term the engine was told.
    wanted: String,
    sent: String,
    more_wanted: bool,
    /// The history was at the newest message when the search opened: closing goes back there.
    was_at_bottom: bool,
    /// A hit whose row is not loaded yet: older pages are read until it shows (or the start is reached).
    jump: Option<Jump>,
    scroll: gpui::ScrollHandle,
}

fn term_of(state: &MessageSearchState) -> Option<&str> {
    match state {
        MessageSearchState::Idle => None,
        MessageSearchState::Searching { term }
        | MessageSearchState::Results { term, .. }
        | MessageSearchState::NoResults { term, .. }
        | MessageSearchState::Error { term } => Some(term),
    }
}

impl RoomSearch {
    fn hits(&self) -> &[MessageHit] {
        match &self.state {
            MessageSearchState::Results { hits, .. } => hits,
            _ => &[],
        }
    }

    fn more(&self) -> Option<MoreHits> {
        match &self.state {
            MessageSearchState::Results { more, .. } => Some(*more),
            _ => None,
        }
    }

    /// Something to show under the header: results or a one-line status.
    fn list_visible(&self) -> bool {
        self.list_open && !matches!(self.state, MessageSearchState::Idle)
    }

    fn status(&self) -> Option<Status> {
        match self.state {
            MessageSearchState::Searching { .. } => Some(Status::Searching),
            MessageSearchState::NoResults { .. } => Some(Status::NoResults),
            MessageSearchState::Error { .. } => Some(Status::Error),
            _ => None,
        }
    }

    /// «mittente, data, estratto» for a screen reader.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn label(hit: &MessageHit) -> String {
        format!("{}, {}, {}", hit.sender_name, matrix_desktop::time_fmt::day_or_time(hit.ts), hit.excerpt)
    }
}

impl Shell {
    /// The lens: opens the field in the header with the keyboard in it (or just focuses it).
    pub(crate) fn open_room_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected || self.sidebar.selected().is_none() {
            return;
        }
        let Some(at_bottom) = self.timeline.as_ref().map(|t| t.at_bottom()) else {
            return;
        };
        if let Some(search) = &self.room_search {
            search.field.read(cx).focus.focus(window);
            return;
        }
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = "Cerca";
            field
        });
        cx.observe(&field, |this, field, cx| {
            this.room_search_typed(field.read(cx).input.display_text().trim().to_owned());
            cx.notify();
        })
        .detach();
        field.read(cx).focus.focus(window);
        self.room_search = Some(RoomSearch {
            scope: self.active_scope.clone(),
            field,
            state: MessageSearchState::Idle,
            active: 0,
            list_open: true,
            wanted: String::new(),
            sent: String::new(),
            more_wanted: false,
            was_at_bottom: at_bottom,
            jump: None,
            scroll: gpui::ScrollHandle::new(),
        });
        cx.notify();
    }

    /// The keyboard is in the search field: plain keys belong to it, not to the shell's shortcuts
    /// (Ctrl/Cmd chords such as Cmd+K still pass).
    pub(crate) fn room_search_typing(&self, modifiers: &gpui::Modifiers, window: &Window, cx: &gpui::App) -> bool {
        !modifiers.control
            && !modifiers.platform
            && self.room_search.as_ref().is_some_and(|s| s.field.read(cx).focus.is_focused(window))
    }

    /// Esc (second), ✕, or the room changed. `restore` puts the history back where it was.
    pub(crate) fn close_room_search(&mut self, restore: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(search) = self.room_search.take() else {
            return;
        };
        if let Some(timeline) = &self.timeline {
            timeline.set_search_mark(None);
            if restore && search.was_at_bottom {
                timeline.jump_to_boundary(true);
            }
            if let Some(window) = window {
                timeline.focus_timeline(window);
            }
        }
        // The engine drops its results; an empty term is the close message.
        if search.sent.is_empty() {
            // Nothing was ever asked.
        } else if self.pending_operation_command.is_none() {
            self.pending_operation_command =
                Some(OperationCommand::SearchMessages { scope: search.scope, term: String::new() });
        }
        cx.notify();
    }

    fn room_search_typed(&mut self, term: String) {
        let Some(search) = self.room_search.as_mut().filter(|s| s.wanted != term) else {
            return;
        };
        search.state = if term.chars().count() < matrix_engine::message_match::MIN_CHARS {
            MessageSearchState::Idle
        } else {
            MessageSearchState::Searching { term: term.clone() }
        };
        search.wanted = term;
        search.active = 0;
        search.list_open = true;
    }

    /// Sends the wanted term (or the next page) to the engine once; called from the operations tick.
    pub(crate) fn flush_room_search(&mut self) {
        let Some(search) = self.room_search.as_mut() else {
            return;
        };
        let Some((sender, _)) = &mut self.routes.operation else {
            return;
        };
        if search.sent != search.wanted {
            if sender
                .try_send(OperationCommand::SearchMessages { scope: search.scope.clone(), term: search.wanted.clone() })
                .is_ok()
            {
                search.sent = search.wanted.clone();
            }
        } else if search.more_wanted && sender.try_send(OperationCommand::MoreMessages).is_ok() {
            search.more_wanted = false;
        }
    }

    pub(crate) fn room_search_result(&mut self, scope: RoomScope, state: MessageSearchState, cx: &mut Context<Self>) {
        let Some(search) = self.room_search.as_mut().filter(|s| s.scope == scope) else {
            return;
        };
        // An answer for a term no longer in the field is stale.
        match term_of(&state) {
            Some(term) if term != search.wanted => return,
            None if search.wanted.chars().count() >= matrix_engine::message_match::MIN_CHARS => return,
            _ => {}
        }
        if term_of(&search.state) != term_of(&state) {
            search.active = 0;
            search.list_open = true;
        }
        search.state = state;
        search.active = search.active.min(search.hits().len().saturating_sub(1));
        cx.notify();
    }

    pub(crate) fn room_search_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let Some(search) = self.room_search.as_mut() else {
            return;
        };
        let len = search.hits().len();
        match key {
            "escape" if search.list_visible() => search.list_open = false,
            "escape" => return self.close_room_search_from_key(window, cx),
            "down" | "up" if len > 0 => {
                if !search.list_open {
                    search.list_open = true;
                } else if key == "down" && search.active + 1 < len {
                    search.active += 1;
                } else if key == "down" && matches!(search.more(), Some(MoreHits::Available | MoreHits::Failed)) {
                    search.more_wanted = true;
                } else if key == "down" {
                    search.active = 0;
                } else {
                    search.active = (search.active + len - 1) % len;
                }
                search.scroll.scroll_to_item(search.active);
            }
            "enter" if len > 0 => {
                if search.list_open {
                    let at = search.active;
                    self.jump_to_hit(at, cx);
                } else {
                    search.list_open = true;
                }
            }
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn close_room_search_from_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_room_search(true, Some(window), cx);
        cx.stop_propagation();
    }

    /// Enter or a click on result `i`: the list closes, the row is shown and flashes. The field
    /// keeps the keyboard and the term.
    fn jump_to_hit(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(search) = self.room_search.as_mut() else {
            return;
        };
        let MessageSearchState::Results { hits, term, .. } = &search.state else {
            return;
        };
        let Some(hit) = hits.get(i) else {
            return;
        };
        search.jump = Some(Jump {
            show: EventId::new(hit.thread_root.as_deref().unwrap_or(&hit.event_id)),
            open_thread: hit.thread_root.is_some(),
            term: term.clone(),
        });
        search.active = i;
        search.list_open = false;
        self.drive_room_search(cx);
        cx.notify();
    }

    /// One tick (and right after a jump): a stale search closes, a pending jump scrolls once the
    /// row is loaded (reading older pages meanwhile), the flash repaints once when it ends.
    pub(crate) fn drive_room_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.room_search.as_mut() else {
            return;
        };
        if search.scope != self.active_scope || self.timeline.is_none() {
            self.close_room_search(false, None, cx);
            return;
        }
        let timeline = self.timeline.as_ref().expect("checked above");
        if timeline.flash_ended() {
            cx.notify();
        }
        let Some(jump) = search.jump.clone() else {
            return;
        };
        if timeline.scroll_to_event(&jump.show) {
            timeline.set_search_mark(Some(crate::timeline_view::SearchMark {
                event: jump.show.clone(),
                term: jump.term,
                flash_until: std::time::Instant::now() + FLASH,
                ended: false,
            }));
            search.jump = None;
            if jump.open_thread {
                self.open_thread(&jump.show, None, cx);
            }
            cx.notify();
        } else if self.pending_history.is_none() {
            match self.history.status() {
                HistoryStatus::More => self.request_older(),
                HistoryStatus::Loading => {}
                // The start was reached (or loading failed) without the row: it is not reachable.
                _ => search.jump = None,
            }
        }
    }

    pub(crate) fn render_room_search_field(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let search = self.room_search.as_ref()?;
        search.field.update(cx, |field, _| field.theme = t);
        Some(
            div()
                .id("room-search")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.room_search_key(event, window, cx)))
                .child(div().flex_1().min_w_0().child(search.field.clone()))
                .child(
                    div()
                        .id("room-search-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .flex_shrink_0()
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(20.))
                        .text_color(rgb(t.secondary))
                        .hover(|s| s.bg(rgb(t.border)))
                        .child("×")
                        .on_click(cx.listener(|this, _, window, cx| this.close_room_search(true, Some(window), cx))),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn render_room_search_results(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let search = self.room_search.as_ref().filter(|s| s.list_visible())?;
        let rows = search.hits().iter().enumerate().map(|(i, hit)| {
            let marks = hit.matches.iter().map(|r| {
                (
                    r.clone(),
                    HighlightStyle {
                        font_weight: Some(gpui::FontWeight::BOLD),
                        color: Some(rgb(t.accent).into()),
                        ..Default::default()
                    },
                )
            });
            div()
                .id(("room-search-hit", i))
                .flex()
                .flex_col()
                .px_6()
                .py_2()
                .cursor_pointer()
                .when(i == search.active, |d| d.bg(rgb(t.input)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().truncate().font_weight(gpui::FontWeight::SEMIBOLD).child(matrix_desktop::row_text::one_line(&hit.sender_name)))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(12.))
                                .text_color(rgb(t.secondary))
                                .child(matrix_desktop::time_fmt::day_or_time(hit.ts)),
                        ),
                )
                .child(div().truncate().child(StyledText::new(hit.excerpt.clone()).with_highlights(marks)))
                .on_click(cx.listener(move |this, _, _, cx| this.jump_to_hit(i, cx)))
        });
        let more = matches!(search.more(), Some(MoreHits::Available | MoreHits::Loading | MoreHits::Failed)).then(|| {
            let label = match search.more() {
                Some(MoreHits::Loading) => "Caricamento",
                Some(MoreHits::Failed) => "Altri risultati non caricati",
                _ => "Altri risultati",
            };
            div()
                .id("room-search-more")
                .px_6()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(t.accent))
                .child(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(search) = &mut this.room_search {
                        search.more_wanted = true;
                    }
                    cx.notify();
                }))
        });
        Some(
            div()
                .id("room-search-results")
                .flex_shrink_0()
                .max_h(px(320.))
                .overflow_y_scroll()
                .track_scroll(&search.scroll)
                .flex()
                .flex_col()
                .bg(rgb(t.surface))
                .border_b_1()
                .border_color(rgb(t.border))
                .children(rows)
                .children(more)
                .children(search.status().map(|status| {
                    div()
                        .px_6()
                        .py_2()
                        .text_size(px(13.))
                        .text_color(rgb(if status == Status::Error { t.error } else { t.disabled }))
                        .child(status.text())
                }))
                .into_any_element(),
        )
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_room_search(&self, window: &Window, cx: &gpui::App) -> matrix_desktop::accessibility::room_search::RoomSearchView {
        use matrix_desktop::accessibility::room_search::{RoomSearchView, Snapshot};
        match &self.room_search {
            Some(search) => RoomSearchView::Open(Box::new(Snapshot {
                text: search.field.read(cx).input.display_text(),
                labels: search.hits().iter().map(RoomSearch::label).collect(),
                active: search.active,
                list_open: search.list_visible(),
                status: search.status(),
                more: matches!(search.more(), Some(MoreHits::Available | MoreHits::Failed)),
                bounds: search.field.read(cx).semantic_bounds,
                focused: search.field.read(cx).focus.is_focused(window),
            })),
            None => RoomSearchView::Closed,
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_room_search(
        &mut self,
        action: matrix_desktop::accessibility::room_search::RoomSearchAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::room_search::RoomSearchAction;
        match action {
            RoomSearchAction::FocusInput => {
                if let Some(search) = &self.room_search {
                    search.field.read(cx).focus.focus(window);
                }
            }
            RoomSearchAction::Close => self.close_room_search(true, Some(window), cx),
            RoomSearchAction::Choose(i) => self.jump_to_hit(i, cx),
            RoomSearchAction::More => {
                if let Some(search) = &mut self.room_search {
                    search.more_wanted = true;
                }
            }
        }
        cx.notify();
    }
}

/// Drives the real Shell through the search on a synthetic room (called by the opener GUI fixture):
/// lens opens the field, arrows and Enter jump to a loaded row and flash it, an unreachable row is
/// given up, Esc closes the list and then the search and the history returns to the newest message.
#[cfg(test)]
pub(crate) fn exercise(this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
    use crate::timeline_view::{SearchMark, TimelineView};
    use matrix_engine::history::{HistoryEvent, HistorySender};
    use matrix_engine::operations::MessageSource;
    let event = |n: usize| HistoryEvent {
        redacted: false,
        event_id: EventId::new(format!("$m{n}:t")),
        body: format!("riga {n} ciao"),
        outgoing: false,
        timestamp_ms: 1_000 * n as u64,
        sender: HistorySender::new("@a:t", None),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    };
    let hit = |n: usize| MessageHit {
        event_id: format!("$m{n}:t"),
        sender: "@a:t".into(),
        sender_name: "A".into(),
        ts: 1_000 * n as u64,
        excerpt: format!("riga {n} ciao"),
        matches: std::iter::once(8..12).collect(),
        thread_root: None,
    };
    let key = |this: &mut Shell, name: &str, window: &mut Window, cx: &mut Context<Shell>| {
        let event = KeyDownEvent { keystroke: gpui::Keystroke::parse(name).unwrap(), is_held: false };
        this.room_search_key(&event, window, cx);
    };
    let timeline = TimelineView::connected(this.active_scope.clone(), cx.focus_handle());
    timeline.append_server((1..=60).map(event).collect());
    this.timeline = Some(timeline);
    assert!(this.room_search.is_none());
    this.open_room_search(window, cx);
    assert!(this.room_search.is_some(), "the lens opens the field in the header");
    assert!(this.room_search_typing(&gpui::Modifiers::default(), window, cx), "the field has the keyboard");
    assert!(!this.room_search_typing(&gpui::Modifiers::command(), window, cx), "chords still reach the shell");

    // One character: nothing is searched. Two: «Ricerca in corso» until the engine answers.
    this.room_search_typed("c".into());
    assert!(matches!(this.room_search.as_ref().unwrap().state, MessageSearchState::Idle));
    this.room_search_typed("ciao".into());
    assert!(this.room_search.as_ref().unwrap().status() == Some(Status::Searching));
    // A late answer for another term is dropped.
    let scope = this.active_scope.clone();
    let results = |term: &str, hits: Vec<MessageHit>| MessageSearchState::Results {
        term: term.into(),
        source: MessageSource::LocalCache,
        hits,
        more: MoreHits::None,
    };
    this.room_search_result(scope.clone(), results("altro", vec![hit(1)]), cx);
    assert!(this.room_search.as_ref().unwrap().hits().is_empty());
    this.room_search_result(scope, results("ciao", vec![hit(40), hit(7), hit(9999)]), cx);
    let search = this.room_search.as_ref().unwrap();
    assert!(search.list_visible() && search.hits().len() == 3 && search.active == 0);

    // Arrows wrap; Enter jumps; the list closes; the term stays in the field.
    key(this, "up", window, cx);
    assert_eq!(this.room_search.as_ref().unwrap().active, 2, "Up from the first wraps to the last");
    key(this, "down", window, cx);
    key(this, "down", window, cx);
    assert_eq!(this.room_search.as_ref().unwrap().active, 1);
    key(this, "enter", window, cx);
    let timeline = this.timeline.as_ref().unwrap();
    assert_eq!(timeline.first_visible_row(), 6, "row of $m7 is at the top");
    assert!(!this.room_search.as_ref().unwrap().list_visible());
    assert!(this.room_search.as_ref().unwrap().jump.is_none());
    assert!(!timeline.flash_ended(), "the flash is still running");
    // Enter or Down with the list closed only reopens it.
    key(this, "down", window, cx);
    assert!(this.room_search.as_ref().unwrap().list_visible());

    // A row that is not loaded and cannot be reached (no older page) is given up, the view stays.
    this.room_search.as_mut().unwrap().active = 2;
    key(this, "enter", window, cx);
    assert!(this.room_search.as_ref().unwrap().jump.is_none(), "unreachable row: no endless paging");
    assert_eq!(this.timeline.as_ref().unwrap().first_visible_row(), 6);

    // Esc: the list first (search stays), then the search; the history is back at the newest.
    // The operations tick told the engine about the term; closing then tells it to forget.
    this.room_search.as_mut().unwrap().sent = "ciao".into();
    key(this, "down", window, cx);
    assert!(this.room_search.as_ref().unwrap().list_visible());
    key(this, "escape", window, cx);
    assert!(this.room_search.as_ref().is_some_and(|s| !s.list_visible()), "first Esc closes the list only");
    key(this, "escape", window, cx);
    assert!(this.room_search.is_none(), "second Esc closes the search");
    let timeline = this.timeline.as_ref().unwrap();
    assert!(timeline.first_visible_row() >= 59, "the history returned to the newest message");
    assert!(matches!(this.pending_operation_command, Some(OperationCommand::SearchMessages { ref term, .. }) if term.is_empty()));
    this.pending_operation_command = None;

    // The flash repaints exactly once when it runs out.
    timeline.set_search_mark(Some(SearchMark {
        event: EventId::new("$m7:t"),
        term: "ciao".into(),
        flash_until: std::time::Instant::now() - std::time::Duration::from_millis(1),
        ended: false,
    }));
    assert!(timeline.flash_ended() && !timeline.flash_ended());
    timeline.set_search_mark(None);
}
