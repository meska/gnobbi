//! Side panel of one thread: the root, then its replies, oldest first, and a composer.
//! The engine answers each request with a snapshot of the SDK thread timeline; a text sent from
//! the composer waits as a local line until the snapshot holds the event the server confirmed.
use crate::*;
use gpui::ScrollHandle;
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    RequestTag, TrySendError, ViewRequests,
    history::{HistoryEvent, ThreadPage},
    operations::{OperationCommand, OperationError},
};

/// Replies asked for at the first read and added by each older page.
const STEP: usize = 25;

#[derive(Default)]
pub(crate) struct ThreadRow {
    id: Option<EventId>,
    who: String,
    text: String,
    time: String,
    media: Option<matrix_engine::history::Media>,
}

/// An attachment without caption is shown by its label.
fn caption_or_label(text: &str, media: Option<&matrix_engine::history::Media>) -> String {
    match media {
        Some(m) if text.is_empty() => m.label.clone(),
        _ => text.to_owned(),
    }
}

impl ThreadRow {
    fn show_caption(&self, has_picture: bool) -> bool {
        !has_picture || self.media.as_ref().is_none_or(|m| m.label != self.text)
    }

    fn from_event(e: &HistoryEvent) -> Self {
        let who = if e.outgoing { "Tu".to_owned() } else { e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone()) };
        let text = caption_or_label(&e.body, e.media.as_ref());
        Self { id: Some(e.event_id.clone()), who, text, time: matrix_desktop::time_fmt::clock_time(e.timestamp_ms), media: e.media.clone() }
    }

    pub(crate) fn from_row(row: &matrix_desktop::DemoEvent) -> Self {
        let who = match (&row.sender, row.outgoing) {
            (Some(s), false) => s.name.clone().unwrap_or_else(|| s.id.clone()),
            _ => "Tu".to_owned(),
        };
        let text = caption_or_label(&row.text, row.media.as_ref());
        let id = match &row.id.id {
            matrix_domain::TimelineId::Server(id) => Some(id.clone()),
            _ => None,
        };
        Self { id, who, text, time: row.time.clone(), media: row.media.clone() }
    }
}

enum Delivery {
    Queued,
    InFlight,
    /// The server has it under this ID; the line goes once a snapshot holds that event.
    Confirmed(EventId, std::time::Instant),
    Failed,
}

struct Outgoing {
    body: String,
    reply_to: Option<EventId>,
    delivery: Delivery,
}

/// How long a confirmed line waits for its event to show up in a snapshot (a server hole may never).
const CONFIRMED_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Minimum gap between two receipt attempts while the panel stays visible: the retry never runs
/// per frame (the tick is 16 ms), and only one attempt is in flight at a time.
const READ_CADENCE: std::time::Duration = std::time::Duration::from_millis(400);

pub(crate) struct ThreadPanel {
    pub(crate) composer: Option<gpui::Entity<ComposerView>>,
    /// The event the next text answers (`send_reply`), with a one-line label for the bar.
    reply_to: Option<(EventId, String)>,
    outgoing: Vec<Outgoing>,
    scope: RoomScope,
    pub(crate) root: EventId,
    /// The room timeline's row, shown whenever the thread read does not carry the root.
    root_row: ThreadRow,
    replies: Vec<ThreadRow>,
    total: u32,
    want: usize,
    pending: bool,
    dirty: bool,
    loaded_at: Option<std::time::Instant>,
    failed: bool,
    /// The page request's lifecycle: a page from a closed or replaced visit is not applied.
    views: ViewRequests,
    /// The receipt request's lifecycle, apart from the page: revoking it stops the send, dropping
    /// the page does not. A tag of another visit or scope never clears the root.
    read_views: ViewRequests,
    /// The server accepted the receipt of this visit: the root is already down, do not re-send.
    read_done: bool,
    /// One receipt is in flight (or queued): send no equivalent until its outcome.
    read_pending: bool,
    /// The last receipt failed: one line in the panel until a later attempt succeeds or is no-op.
    read_failed: bool,
    /// Last attempt: with `loaded_at` it sets the retry cadence, never a per-frame loop.
    read_at: Option<std::time::Instant>,
    /// The last older page brought nothing: the server holds no more of this thread.
    stalled: bool,
    at_start: bool,
    /// Replies before an older page was asked for.
    growing: Option<usize>,
    /// Scroll range before an older page arrived; the position is kept once the range grows.
    anchor: Option<gpui::Pixels>,
    stick_bottom: bool,
    scroll: ScrollHandle,
    /// Mouse selection over one text of the panel; `sel_row` says which row it belongs to (equal texts stay apart).
    selection: matrix_desktop::static_selection::StaticSelection,
    sel_row: u64,
}

const SIDE_PANEL_WIDTH: f32 = 360.;
/// The chat column must keep at least this much next to a docked panel (usable composer).
pub(crate) const CHAT_MIN_WIDTH: f32 = 360.;

/// Narrowest window that fits rail + room list + chat + panel side by side.
fn dock_min_width(list_width: f32) -> f32 {
    crate::rail_view::RAIL_WIDTH + list_width + CHAT_MIN_WIDTH + SIDE_PANEL_WIDTH
}

/// Right-hand column shared by the thread and the emoji drawer. Wide window: docked next to the chat. Narrow: a sheet over it (as the room sheet).
pub(crate) fn side_panel(id: &'static str, window: &Window, t: matrix_desktop::DesignTokens, list_width: f32) -> gpui::Stateful<gpui::Div> {
    let docked = window.viewport_size().width >= px(dock_min_width(list_width));
    div().id(id).occlude()
        .when(docked, |d| d.h_full().w(px(SIDE_PANEL_WIDTH)).flex_shrink_0())
        .when(!docked, |d| d.absolute().right_0().top_0().bottom_0().w(window.viewport_size().width.min(px(SIDE_PANEL_WIDTH))))
        .flex().flex_col().bg(rgb(t.surface)).border_l_1().border_color(rgb(t.border))
}

/// The thread to reopen now: the one a room change parked, once its room is active again and no
/// panel is open (an open panel, even another thread, cancels the resume).
fn resume_due(resume: &mut Option<(RoomScope, EventId)>, active: &RoomScope, open: bool) -> Option<EventId> {
    if open {
        *resume = None;
    }
    resume.as_ref().filter(|(scope, _)| scope == active).map(|(_, root)| root.clone())
}

/// Selection key of an outgoing line; replies are `1 + index`, the root is 0.
const OUTGOING_KEY: u64 = 1 << 32;

impl ThreadPanel {
    fn row_text(&self, key: u64) -> Option<&str> {
        match key {
            0 => Some(&self.root_row.text),
            k if k < OUTGOING_KEY => self.replies.get(k as usize - 1).map(|r| r.text.as_str()),
            k => self.outgoing.get((k - OUTGOING_KEY) as usize).map(|o| o.body.as_str()),
        }
    }

    /// The text the mouse selected in the panel, for Cmd/Ctrl+C.
    pub(crate) fn selected_text(&self) -> Option<String> {
        self.selection.copy(self.row_text(self.sel_row)?).map(str::to_owned)
    }

    /// One row's text, selectable by mouse: press / drag / double and triple click (no links here).
    fn selectable(&self, key: u64, text: &str, t: matrix_desktop::DesignTokens, cx: &mut Context<Shell>) -> gpui::AnyElement {
        let range = (self.sel_row == key).then(|| self.selection.range(text)).flatten();
        let runs = matrix_desktop::static_selection::highlight_runs(text.len(), &[], range);
        let body = gpui::StyledText::new(text.to_owned()).with_highlights(runs.into_iter().map(|(range, _, _)| {
            (range, gpui::HighlightStyle { background_color: Some(rgb(t.selection).into()), ..Default::default() })
        }));
        let (down, drag) = (body.layout().clone(), body.layout().clone());
        let (down_text, drag_text) = (text.to_owned(), text.to_owned());
        div()
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                this.thread_focus.focus(window);
                if let Some(p) = &mut this.thread {
                    p.sel_row = key;
                    p.selection.press(&down_text, down.index_for_position(e.position).unwrap_or_else(|i| i), e.click_count);
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                if e.pressed_button == Some(gpui::MouseButton::Left)
                    && let Some(p) = this.thread.as_mut().filter(|p| p.sel_row == key)
                {
                    p.selection.drag(&drag_text, drag.index_for_position(e.position).unwrap_or_else(|i| i));
                    cx.notify();
                }
            }))
            .child(body)
            .into_any_element()
    }

    pub(crate) fn new(scope: RoomScope, root: EventId, root_row: ThreadRow, total: u32) -> Self {
        Self {
            composer: None, reply_to: None, outgoing: Vec::new(),
            scope, root, root_row, replies: Vec::new(), total, want: STEP, pending: false, dirty: true,
            loaded_at: None, failed: false, views: ViewRequests::new(), read_views: ViewRequests::new(),
            read_done: false, read_pending: false, read_failed: false, read_at: None,
            stalled: false, at_start: false, growing: None, anchor: None,
            stick_bottom: true, scroll: ScrollHandle::new(),
            selection: Default::default(), sel_row: 0,
        }
    }

    /// A closed panel back on screen: keeps the rows, drops what belonged to the old visit.
    fn reopened(mut self) -> Self {
        self.composer = None;
        self.reply_to = None;
        self.outgoing.clear();
        self.pending = false;
        self.dirty = true;
        self.views = ViewRequests::new();
        self.revoke_read();
        self.read_done = false;
        self.read_failed = false;
        self.read_at = None;
        self.growing = None;
        self.anchor = None;
        self.stick_bottom = true;
        self.scroll = ScrollHandle::new();
        self
    }

    pub(crate) fn activity(&mut self, scope: &RoomScope) {
        self.dirty |= self.scope == *scope;
        if self.scope == *scope {
            // A new response is not read by the previous receipt: drop its authorization and let
            // the refreshed page produce a new intent on the target now shown.
            self.read_done = false;
            self.revoke_read();
        }
    }

    /// Cancel the receipt authorization now: no new send and no retry after the visit changed.
    /// `read_at` resets so a later visit (focus, scroll back, reopen) asks again at once.
    pub(crate) fn revoke_read(&mut self) {
        self.read_views.cancel();
        self.read_pending = false;
        self.read_at = None;
    }

    /// The newest server event the panel shows: the receipt points here, never at a hidden row.
    fn receipt_target(&self) -> Option<EventId> {
        self.replies.last().and_then(|r| r.id.clone()).or_else(|| self.root_row.id.clone())
    }

    /// Whether a receipt is due for the page as projected: one attempt per snapshot, a failed
    /// attempt retried on the 400 ms cadence and a no-op only after a new snapshot.
    fn receipt_due(&self) -> bool {
        let Some(loaded) = self.loaded_at else { return false };
        match self.read_at {
            None => true,
            Some(at) => at.elapsed() >= READ_CADENCE && (self.read_failed || at < loaded),
        }
    }

    /// One receipt outcome for this visit. `None` when the tag is stale (another scope, a closed or
    /// replaced visit): the caller applies nothing. `Some(true)` is the server ACK that clears the
    /// root; `Some(false)` a handled no-op or error that leaves rows and unread untouched.
    fn read_result(&mut self, tag: &RequestTag, root: &EventId, result: Result<bool, OperationError>) -> Option<bool> {
        if self.root != *root || self.scope != *tag.scope() || !self.read_views.accepts(tag) {
            return None;
        }
        self.read_pending = false;
        match result {
            Ok(true) => { self.read_done = true; self.read_failed = false; Some(true) }
            Ok(false) => { self.read_failed = false; Some(false) }
            Err(_) => { self.read_failed = true; Some(false) }
        }
    }

    fn more(&self) -> bool {
        !self.stalled && !self.at_start && self.replies.len() < self.total as usize
    }

    fn apply(&mut self, result: Result<ThreadPage, OperationError>) {
        self.pending = false;
        let Ok(page) = result else {
            self.failed = true;
            self.growing = None;
            self.anchor = None;
            return;
        };
        self.failed = false;
        let at_bottom = self.scroll.offset().y <= -self.scroll.max_offset().height + gpui::px(4.);
        let before = self.replies.len();
        if let Some(root) = &page.root {
            self.root_row = ThreadRow::from_event(root);
            if let Some(t) = &root.thread {
                self.total = t.num_replies;
            }
        }
        self.replies = page.replies.iter().map(ThreadRow::from_event).collect();
        let shown = |id: &EventId, since: &std::time::Instant| self.replies.iter().any(|r| r.id.as_ref() == Some(id)) || since.elapsed() > CONFIRMED_GRACE;
        self.outgoing.retain(|o| !matches!(&o.delivery, Delivery::Confirmed(id, since) if shown(id, since)));
        self.at_start = page.at_start;
        self.total = self.total.max(self.replies.len() as u32);
        if let Some(asked) = self.growing.take() {
            // An older page that did not grow the list: Tuwunel holes, or the thread ends here.
            self.stalled = self.replies.len() <= asked;
            self.anchor = (!self.stalled).then(|| self.scroll.max_offset().height);
        }
        self.stick_bottom |= at_bottom && self.replies.len() > before;
        self.loaded_at = Some(std::time::Instant::now());
    }
}

impl Shell {
    pub(crate) fn thread_found(&mut self, tag: RequestTag, root: EventId, result: Result<Box<ThreadPage>, OperationError>, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.thread
            && p.scope == *tag.scope()
            && p.root == root
            && p.views.accepts(&tag)
        {
            p.apply(result.map(|page| *page));
            cx.notify();
        }
    }

    pub(crate) fn close_thread(&mut self) {
        // The panel is kept for the next visit: revoke the receipt now, a stale tag must not send.
        self.revoke_thread_read();
        self.thread_last = self.thread.take();
    }

    /// Cancel the thread receipt's authorization right now, wherever the visit is changing.
    pub(crate) fn revoke_thread_read(&mut self) {
        if let Some(p) = &mut self.thread {
            p.revoke_read();
        }
    }

    /// One receipt outcome from the engine: only a current `Ok(true)` clears the root.
    pub(crate) fn thread_read_result(&mut self, tag: RequestTag, root: EventId, result: Result<bool, OperationError>, cx: &mut Context<Self>) {
        let Some(p) = &mut self.thread else { return };
        match p.read_result(&tag, &root, result) {
            // A stale tag (other visit/scope) or a result for another root: nothing to apply.
            None => return,
            Some(ack) => {
                if ack && let Some(t) = &self.timeline {
                    t.thread_read(&root);
                }
                // ACK and no-op let the store projection settle; an error must not trigger a
                // refresh (a failing receipt would then fetch the page on every retry).
                if ack || !p.read_failed {
                    self.history.settle_soon();
                }
            }
        }
        cx.notify();
    }

    /// Open the panel on the thread `id` belongs to; the composer is ready for the first text.
    pub(crate) fn open_thread(&mut self, id: &EventId, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some((root, row, total)) = self.timeline.as_ref().and_then(|t| t.thread_target(id)) else { return };
        let tokens = self.theme.tokens();
        let scope = self.active_scope.clone();
        let composer = cx.new(|cx| {
            let mut editor = ComposerView::new(scope.clone(), cx);
            editor.set_theme(tokens, cx);
            editor.set_text_only();
            editor
        });
        cx.subscribe(&composer, |this: &mut Shell, _, sent: &Submitted, cx| {
            if let Some(p) = &mut this.thread {
                let reply_to = p.reply_to.take().map(|(id, _)| id);
                p.outgoing.push(Outgoing { body: sent.0.clone(), reply_to, delivery: Delivery::Queued });
                p.stick_bottom = true;
                cx.notify();
            }
        })
        .detach();
        if let Some(window) = window {
            composer.update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
        }
        // Same thread as the last one closed: its replies are shown now and the read below only refreshes them.
        let mut panel = match self.thread_last.take() {
            Some(old) if old.scope == scope && old.root == root => old.reopened(),
            _ => ThreadPanel::new(scope, root, row, total),
        };
        panel.composer = Some(composer);
        self.thread = Some(panel);
        cx.notify();
    }

    pub(crate) fn thread_sent(&mut self, scope: RoomScope, root: EventId, result: Result<EventId, OperationError>, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.thread
            && p.scope == scope
            && p.root == root
            && let Some(o) = p.outgoing.iter_mut().find(|o| matches!(o.delivery, Delivery::InFlight))
        {
            o.delivery = match result {
                Ok(id) => Delivery::Confirmed(id, std::time::Instant::now()),
                Err(_) => Delivery::Failed,
            };
            p.dirty = true;
            cx.notify();
        }
    }

    /// One tick: open on a click on a summary, read when asked, load older at the top.
    pub(crate) fn drive_thread(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.timeline.as_ref().and_then(|t| t.want_thread.borrow_mut().take()) {
            self.open_thread(&id, None, cx);
        }
        // Back on the room whose panel a room change parked: reopen it (an explicit close never parks).
        if let Some(root) = resume_due(&mut self.thread_resume, &self.active_scope, self.thread.is_some()) {
            self.open_thread(&root, None, cx);
            // The room's timeline may not hold the thread yet: stay armed until the panel is up.
            if self.thread.is_some() { self.thread_resume = None; }
        }
        let Some(p) = &mut self.thread else { return };
        if p.scope != self.active_scope {
            p.revoke_read();
            self.thread_resume = Some((p.scope.clone(), p.root.clone()));
            self.thread_last = self.thread.take();
            cx.notify();
            return;
        }
        if let Some(old) = p.anchor {
            let now = p.scroll.max_offset().height;
            if now > old {
                p.scroll.set_offset(gpui::point(gpui::px(0.), old - now));
                p.anchor = None;
            }
        } else if p.more()
            && !p.pending
            && p.growing.is_none()
            && p.loaded_at.is_some_and(|at| at.elapsed() > std::time::Duration::from_millis(400))
            && p.scroll.offset().y >= -gpui::px(2.)
        {
            p.growing = Some(p.replies.len());
            p.want += STEP;
            p.dirty = true;
        }
        // One text at a time: the engine answers in order.
        if !p.outgoing.iter().any(|o| matches!(o.delivery, Delivery::InFlight))
            && let Some(o) = p.outgoing.iter_mut().find(|o| matches!(o.delivery, Delivery::Queued))
            && let Some((sender, _)) = &mut self.routes.operation
        {
            let command = OperationCommand::ThreadSend { scope: p.scope.clone(), root: p.root.clone(), reply_to: o.reply_to.clone(), body: o.body.clone() };
            o.delivery = match sender.try_send(command) {
                Ok(()) => Delivery::InFlight,
                Err(TrySendError::Closed(_)) => Delivery::Failed,
                Err(_) => Delivery::Queued,
            };
            cx.notify();
        }
        if p.dirty
            && !p.pending
            && let Some((sender, _)) = &mut self.routes.operation
        {
            let Ok(tag) = p.views.start(p.scope.clone()) else {
                p.failed = true;
                return;
            };
            let command = OperationCommand::Thread { tag, root: p.root.clone(), want: p.want };
            match sender.try_send(command) {
                Ok(()) => (p.pending, p.dirty) = (true, false),
                Err(TrySendError::Closed(_)) => (p.failed, p.dirty) = (true, false),
                Err(_) => (),
            }
            cx.notify();
        }
        // Separate receipt: only while the panel's tail is really on screen, at most one in flight.
        let focused = cx.active_window().is_some();
        let at_tail = p.scroll.offset().y <= -p.scroll.max_offset().height + gpui::px(4.);
        if !(focused && at_tail) {
            // Focus lost or scrolled away: revoke now, a later visit asks again with a fresh tag.
            p.revoke_read();
        } else if !p.read_done
            && !p.read_pending
            && !p.pending
            && p.receipt_due()
            && let Some(target) = p.receipt_target()
            && let Some((sender, _)) = &mut self.routes.operation
            && let Ok(tag) = p.read_views.start(p.scope.clone())
        {
            let public = self.prefs.read_receipts;
            p.read_at = Some(std::time::Instant::now());
            let command = OperationCommand::ThreadRead { tag, root: p.root.clone(), target, public };
            match sender.try_send(command) {
                Ok(()) => (p.read_pending, p.read_failed) = (true, false),
                // Closed route or a full queue: not sent, so the panel says so; the cadence above
                // keeps the retry at 400 ms, never per frame.
                Err(_) => p.read_failed = true,
            }
            cx.notify();
        }
    }

    pub(crate) fn render_thread(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let list_width = self.list_width(window);
        let Some(p) = &mut self.thread else { return div().into_any_element() };
        if std::mem::take(&mut p.stick_bottom) {
            p.scroll.scroll_to_bottom();
        }
        let p: &ThreadPanel = p;
        let timeline = self.timeline.as_ref();
        let root = p.root.clone();
        let row = |key: u64, r: &ThreadRow, cx: &mut Context<Self>| {
            // An image root: the bubbles' picture (the room timeline loads it); the label stands in only without one.
            // shortcut: replies that are images keep the label, the timeline loads thumbnails of its own rows only.
            let image = r.media.as_ref().is_some_and(|m| m.kind == matrix_engine::history::MediaKind::Image);
            let picture = (image && r.id.as_ref() == Some(&root)).then(|| timeline.and_then(|tl| tl.picture_for(&root, t))).flatten();
            let caption = r.show_caption(picture.is_some());
            div().flex().flex_col().gap_1()
                .child(div().flex().gap_2().text_size(px(12.)).text_color(rgb(t.secondary))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(matrix_desktop::row_text::one_line(&r.who)))
                    .child(r.time.clone()))
                .children(picture)
                .children(caption.then(|| p.selectable(key, &r.text, t, cx)))
        };
        let answer = |index: usize, r: &ThreadRow, cx: &mut Context<Self>| {
            let target = r.id.clone()?;
            let label = format!("{}: {}", r.who, matrix_desktop::row_text::one_line(&r.text));
            Some(div().id(("thread-reply", index)).cursor_pointer().text_size(px(12.)).text_color(rgb(t.accent)).child("Rispondi")
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(p) = &mut this.thread {
                        p.reply_to = Some((target.clone(), label.clone()));
                        if let Some(c) = &p.composer {
                            c.update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
                        }
                        cx.notify();
                    }
                })))
        };
        let status = if p.failed { Some("Impossibile caricare · riprova più tardi") } else if p.pending && p.replies.is_empty() { Some("Caricamento…") } else if p.read_failed { Some("Lettura non confermata") } else { None };
        let docked = window.viewport_size().width >= px(dock_min_width(list_width));
        let panel = side_panel("thread-panel", window, t, list_width)
            .child(div().h(px(t.header)).flex_shrink_0().px_4().flex().items_center().justify_between()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Thread"))
                .child(div().id("thread-close").w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer().hover(|s| s.bg(rgb(t.border))).child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                    .on_click(cx.listener(|this, _, _, cx| { this.close_thread(); cx.notify(); }))))
            .child(div().id("thread-content").track_focus(&self.thread_focus).flex_1().min_h_0().overflow_y_scroll().track_scroll(&p.scroll)
                .p_4().flex().flex_col().gap_4()
                .child(row(0, &p.root_row, cx))
                .child(div().h(px(1.)).w_full().bg(rgb(t.border)))
                .children(status.map(|s| div().text_size(px(12.)).text_color(rgb(t.accent)).child(s)))
                .children(p.replies.iter().enumerate().map(|(i, r)| row(1 + i as u64, r, cx).children(answer(i, r, cx))))
                .children(p.outgoing.iter().enumerate().map(|(i, o)| {
                    let failed = matches!(o.delivery, Delivery::Failed);
                    div().id(("thread-outgoing", i)).flex().flex_col().gap_1().text_color(rgb(t.secondary))
                        .child(div().text_size(px(12.)).child(if failed { "Non inviato · tocca per riprovare" } else { "Invio…" }))
                        .child(p.selectable(OUTGOING_KEY + i as u64, &o.body, t, cx))
                        .when(failed, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(o) = this.thread.as_mut().and_then(|p| p.outgoing.get_mut(i)) {
                                o.delivery = Delivery::Queued;
                                cx.notify();
                            }
                        })))
                })))
            .children(p.reply_to.as_ref().map(|(_, label)| div().flex_shrink_0().px_4().py_1().flex().items_center().justify_between().border_t_1().border_color(rgb(t.border)).text_size(px(12.)).text_color(rgb(t.secondary))
                .child(div().min_w_0().overflow_hidden().child(label.clone()))
                .child(div().id("thread-reply-clear").cursor_pointer().size(px(24.)).flex().items_center().justify_center().child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.secondary))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(p) = &mut this.thread { p.reply_to = None; }
                        cx.notify();
                    })))))
            .children(p.composer.clone());
        if docked { panel.into_any_element() } else { gpui::deferred(panel).with_priority(1).into_any_element() }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn side_panel_docks_only_when_chat_keeps_360px() {
        // Measured on the layout: rail 72 + list 320 + chat 360 + panel 360.
        assert_eq!(super::dock_min_width(matrix_desktop::LIGHT.sidebar), 1112.);
        assert_eq!(super::dock_min_width(matrix_desktop::DARK.sidebar), super::dock_min_width(matrix_desktop::LIGHT.sidebar));
    }

    use super::*;
    use matrix_domain::{AccountId, RoomId};
    use matrix_engine::history::HistorySender;

    fn event(n: usize) -> HistoryEvent {
        HistoryEvent {
            poll: None,
            event_id: EventId::new(format!("$r{n}")),
            body: format!("risposta {n}"),
            outgoing: false,
            redacted: false,
            timestamp_ms: 0,
            sender: HistorySender::new("@a:t", None),
            video: None,
            audio: None,
            attachment_name: None,
            attachment_size: None, media: None,
            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
            thread: None,
            thread_root: None,
            status: false,
            utd: false,
        }
    }

    fn page(replies: usize) -> ThreadPage {
        ThreadPage { root: None, replies: (1..=replies).map(event).collect(), reactions: Vec::new(), at_start: false }
    }

    fn panel(total: u32) -> ThreadPanel {
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:t") };
        let root = ThreadRow { who: "x".into(), text: "radice".into(), ..Default::default() };
        ThreadPanel::new(scope, EventId::new("$root"), root, total)
    }

    #[test]
    fn selection_copies_from_its_row_and_not_from_a_missing_one() {
        let mut p = panel(0);
        p.replies = vec![
            ThreadRow { text: "ok".into(), ..Default::default() },
            ThreadRow { text: "ok".into(), ..Default::default() },
        ];
        assert_eq!(p.selected_text(), None);
        p.sel_row = 2;
        p.selection.select_all("ok");
        assert_eq!(p.selected_text().as_deref(), Some("ok"));
        p.sel_row = 0; // the root's text is not the selected one
        assert_eq!(p.selected_text(), None);
        p.sel_row = 99; // row gone (list changed)
        assert_eq!(p.selected_text(), None);
    }

    #[test]
    fn a_caption_less_image_root_keeps_its_media_and_a_text_root_none() {
        let mut e = event(0);
        e.body = "testo".into();
        let text = ThreadRow::from_event(&e);
        assert_eq!((text.text.as_str(), text.media.is_none()), ("testo", true));
        e.body.clear();
        e.media = Some(matrix_engine::history::Media { kind: matrix_engine::history::MediaKind::Image, label: "Immagine".into() });
        let image = ThreadRow::from_event(&e);
        assert_eq!(image.text, "Immagine");
        assert!(image.show_caption(false), "an unloaded image keeps its label visible");
        assert!(!image.show_caption(true), "a loaded image does not repeat its label");
        assert!(image.media.is_some_and(|m| m.kind == matrix_engine::history::MediaKind::Image));
    }

    #[test]
    fn root_stays_when_the_read_does_not_carry_it_and_older_pages_load_until_the_count() {
        let mut p = panel(60);
        assert!(p.more());
        p.apply(Ok(page(26)));
        assert_eq!((p.root_row.text.as_str(), p.replies.len()), ("radice", 26));
        assert!(p.more(), "26 of 60: older replies exist");
        p.apply(Ok(page(60)));
        assert!(!p.more(), "all 60 loaded");
    }

    #[test]
    fn an_older_page_that_brings_nothing_ends_the_loading() {
        let mut p = panel(60);
        p.apply(Ok(page(26)));
        p.growing = Some(26);
        p.apply(Ok(page(26)));
        assert!(p.stalled && !p.more(), "a server hole must not make the panel ask forever");
    }

    #[test]
    fn a_reopened_panel_keeps_its_rows_and_asks_for_a_refresh() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        p.outgoing.push(Outgoing { body: "x".into(), reply_to: None, delivery: Delivery::Queued });
        let p = p.reopened();
        assert_eq!(p.replies.len(), 2, "shown at once, before any read");
        assert!(p.dirty && !p.pending && p.outgoing.is_empty());
    }

    #[test]
    fn a_thread_parked_by_a_room_change_resumes_on_return_only() {
        let (a, b) = (RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:t") }, RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!b:t") });
        let mut resume = Some((a.clone(), EventId::new("$root")));
        assert_eq!(resume_due(&mut resume, &b, false), None, "in another room nothing opens");
        assert_eq!(resume_due(&mut resume, &a, false), Some(EventId::new("$root")), "back in A the parked thread is due");
        assert_eq!(resume_due(&mut resume, &a, true), None, "a panel already open cancels it");
        assert!(resume.is_none());
    }

    #[test]
    fn an_explicit_close_leaves_nothing_to_resume() {
        let a = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:t") };
        let mut shell_resume = None; // close_thread never sets it: only the room-change park does
        assert_eq!(resume_due(&mut shell_resume, &a, false), None);
    }

    #[test]
    fn a_failed_read_keeps_what_is_shown() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        p.apply(Err(OperationError::Transport));
        assert!(p.failed);
        assert_eq!(p.replies.len(), 2);
    }

    #[test]
    fn a_confirmed_text_shows_once_and_leaves_when_the_snapshot_holds_its_event() {
        let mut p = panel(3);
        p.outgoing.push(Outgoing { body: "ciao".into(), reply_to: None, delivery: Delivery::Confirmed(EventId::new("$r2"), std::time::Instant::now()) });
        p.apply(Ok(page(1)));
        assert_eq!(p.outgoing.len(), 1, "the event is not in the snapshot yet: the local line stays");
        p.apply(Ok(page(2)));
        assert!(p.outgoing.is_empty(), "the snapshot has it: only the real row is left");
    }

    #[test]
    fn a_failed_text_stays_for_a_retry() {
        let mut p = panel(3);
        p.outgoing.push(Outgoing { body: "ciao".into(), reply_to: None, delivery: Delivery::Failed });
        p.apply(Ok(page(2)));
        assert_eq!(p.outgoing.len(), 1);
    }

    /// A current-tag receipt intent, as `drive_thread` would start it.
    fn read_tag(p: &mut ThreadPanel) -> RequestTag {
        p.read_views.start(p.scope.clone()).unwrap()
    }

    #[test]
    fn a_page_before_the_ack_projects_rows_without_clearing_the_root() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        assert_eq!(p.replies.len(), 2, "the page is shown at once");
        assert!(!p.read_done, "a successful page proves nothing about the receipt");
        assert!(!p.read_failed && !p.read_pending);
    }

    #[test]
    fn only_a_current_ack_clears_the_root() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        let tag = read_tag(&mut p);
        p.read_pending = true;
        let root = p.root.clone();
        assert_eq!(p.read_result(&tag, &root, Ok(true)), Some(true));
        assert!(p.read_done && !p.read_pending && !p.read_failed);
    }

    #[test]
    fn a_no_op_receipt_frees_the_intent_without_a_fake_confirmation() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        let tag = read_tag(&mut p);
        p.read_pending = true;
        let root = p.root.clone();
        assert_eq!(p.read_result(&tag, &root, Ok(false)), Some(false));
        assert!(!p.read_done, "Ok(false) is no send, not a confirmed read");
        assert!(!p.read_pending && !p.read_failed);
    }

    #[test]
    fn a_failed_receipt_keeps_the_rows_and_marks_one_line() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        let tag = read_tag(&mut p);
        let root = p.root.clone();
        assert_eq!(p.read_result(&tag, &root, Err(OperationError::Transport)), Some(false));
        assert!(p.read_failed && !p.read_done);
        assert_eq!(p.replies.len(), 2, "a receipt error never touches the page");
    }

    #[test]
    fn an_ack_is_stale_after_activity_or_another_scope() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        let tag = read_tag(&mut p);
        let scope = p.scope.clone();
        p.activity(&scope);
        assert!(!p.read_done && !p.read_views.accepts(&tag), "activity revokes the old tag");
        let root = p.root.clone();
        assert_eq!(p.read_result(&tag, &root, Ok(true)), None, "a revoked ACK applies nothing");
        // A tag minted for another room is never accepted here, even at the same generation.
        let foreign = ViewRequests::new()
            .start(RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!other:t") })
            .unwrap();
        assert_eq!(p.read_result(&foreign, &root, Ok(true)), None);
    }

    #[test]
    fn the_receipt_points_at_the_newest_shown_event() {
        let mut p = panel(3);
        assert_eq!(p.receipt_target(), None, "nothing projected: no target, no receipt");
        p.apply(Ok(page(2)));
        assert_eq!(p.receipt_target(), Some(EventId::new("$r2")));
    }

    #[test]
    fn a_no_op_waits_for_a_new_page_and_an_error_retries_on_the_cadence() {
        let mut p = panel(3);
        p.apply(Ok(page(2)));
        let base = std::time::Instant::now() - std::time::Duration::from_secs(5);
        p.loaded_at = Some(base);
        p.read_at = Some(base + std::time::Duration::from_secs(1)); // one attempt, after the snapshot
        assert!(!p.receipt_due(), "a no-op waits for the next snapshot, never a per-frame loop");
        p.read_failed = true;
        assert!(p.receipt_due(), "a failed attempt retries once the 400 ms cadence passed");
        p.read_failed = false;
        p.apply(Ok(page(3)));
        assert!(p.receipt_due(), "a new snapshot makes a fresh intent due");
    }
}
