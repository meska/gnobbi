//! Side panel of one thread: the root, then its replies, oldest first, and a composer.
//! The engine answers each request with a snapshot of the SDK thread timeline; a text sent from
//! the composer waits as a local line until the snapshot holds the event the server confirmed.
use crate::*;
use gpui::ScrollHandle;
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    TrySendError,
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
}

/// An attachment without caption is shown by its label.
fn caption_or_label(text: &str, media: Option<&matrix_engine::history::Media>) -> String {
    match media {
        Some(m) if text.is_empty() => m.label.clone(),
        _ => text.to_owned(),
    }
}

impl ThreadRow {
    fn from_event(e: &HistoryEvent) -> Self {
        let who = if e.outgoing { "Tu".to_owned() } else { e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone()) };
        let text = caption_or_label(&e.body, e.media.as_ref());
        Self { id: Some(e.event_id.clone()), who, text, time: matrix_desktop::time_fmt::clock_time(e.timestamp_ms) }
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
        Self { id, who, text, time: row.time.clone() }
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

pub(crate) struct ThreadPanel {
    pub(crate) composer: Option<gpui::Entity<ComposerView>>,
    /// The event the next text answers (`send_reply`), with a one-line label for the bar.
    reply_to: Option<(EventId, String)>,
    outgoing: Vec<Outgoing>,
    scope: RoomScope,
    root: EventId,
    /// The room timeline's row, shown whenever the thread read does not carry the root.
    root_row: ThreadRow,
    replies: Vec<ThreadRow>,
    total: u32,
    want: usize,
    pending: bool,
    dirty: bool,
    loaded_at: Option<std::time::Instant>,
    failed: bool,
    /// A request with the receipt succeeded since the last activity.
    read_done: bool,
    /// The request in flight asked the engine to send the thread's read receipt.
    read_sent: bool,
    /// The last older page brought nothing: the server holds no more of this thread.
    stalled: bool,
    at_start: bool,
    /// Replies before an older page was asked for.
    growing: Option<usize>,
    /// Scroll range before an older page arrived; the position is kept once the range grows.
    anchor: Option<gpui::Pixels>,
    stick_bottom: bool,
    scroll: ScrollHandle,
}

const SIDE_PANEL_WIDTH: f32 = 360.;
/// The chat column must keep at least this much next to a docked panel (usable composer).
const CHAT_MIN_WIDTH: f32 = 360.;

/// Narrowest window that fits rail + room list + chat + panel side by side.
fn dock_min_width(t: matrix_desktop::DesignTokens) -> f32 {
    crate::rail_view::RAIL_WIDTH + t.sidebar + CHAT_MIN_WIDTH + SIDE_PANEL_WIDTH
}

/// Right-hand column shared by the thread and the emoji drawer. Wide window: docked next to the chat. Narrow: a sheet over it (as the room sheet).
pub(crate) fn side_panel(id: &'static str, window: &Window, t: matrix_desktop::DesignTokens) -> gpui::Stateful<gpui::Div> {
    let docked = window.viewport_size().width >= px(dock_min_width(t));
    div().id(id).occlude()
        .when(docked, |d| d.h_full().w(px(SIDE_PANEL_WIDTH)).flex_shrink_0())
        .when(!docked, |d| d.absolute().right_0().top_0().bottom_0().w(window.viewport_size().width.min(px(SIDE_PANEL_WIDTH))))
        .flex().flex_col().bg(rgb(t.surface)).border_l_1().border_color(rgb(t.border))
}

impl ThreadPanel {
    pub(crate) fn new(scope: RoomScope, root: EventId, root_row: ThreadRow, total: u32) -> Self {
        Self {
            composer: None, reply_to: None, outgoing: Vec::new(),
            scope, root, root_row, replies: Vec::new(), total, want: STEP, pending: false, dirty: true,
            loaded_at: None, failed: false, read_done: false, read_sent: false, stalled: false, at_start: false, growing: None, anchor: None,
            stick_bottom: true, scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn activity(&mut self, scope: &RoomScope) {
        self.dirty |= self.scope == *scope;
        self.read_done &= self.scope != *scope;
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
    pub(crate) fn thread_found(&mut self, scope: RoomScope, root: EventId, result: Result<Box<ThreadPage>, OperationError>, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.thread
            && p.scope == scope
            && p.root == root
        {
            let read = p.read_sent;
            p.apply(result.map(|page| *page));
            // The engine sent the receipt with this read: the root's counter drops at once.
            p.read_done |= read && !p.failed;
            if read && !p.failed {
                if let Some(t) = &self.timeline {
                    t.thread_read(&root);
                }
                self.history.settle_soon();
            }
            cx.notify();
        }
    }

    pub(crate) fn close_thread(&mut self) {
        self.thread = None;
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
        let mut panel = ThreadPanel::new(scope, root, row, total);
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
        let (public, focused) = (self.prefs.read_receipts, cx.active_window().is_some());
        if let Some(id) = self.timeline.as_ref().and_then(|t| t.want_thread.borrow_mut().take()) {
            self.open_thread(&id, None, cx);
        }
        let Some(p) = &mut self.thread else { return };
        // Read receipts only while the window is the one in use; asked again until one went through.
        let read = focused.then_some(public);
        p.dirty |= focused && !p.read_done;
        if p.scope != self.active_scope {
            self.thread = None;
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
            let command = OperationCommand::Thread { scope: p.scope.clone(), root: p.root.clone(), want: p.want, read };
            match sender.try_send(command) {
                Ok(()) => (p.pending, p.dirty, p.read_sent) = (true, false, read.is_some()),
                Err(TrySendError::Closed(_)) => (p.failed, p.dirty) = (true, false),
                Err(_) => (),
            }
            cx.notify();
        }
    }

    pub(crate) fn render_thread(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let Some(p) = &mut self.thread else { return div().into_any_element() };
        if std::mem::take(&mut p.stick_bottom) {
            p.scroll.scroll_to_bottom();
        }
        let row = |r: &ThreadRow| {
            div().flex().flex_col().gap_1()
                .child(div().flex().gap_2().text_size(px(12.)).text_color(rgb(t.secondary))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(matrix_desktop::row_text::one_line(&r.who)))
                    .child(r.time.clone()))
                .child(div().child(r.text.clone()))
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
        let status = if p.failed { Some("Impossibile caricare · riprova più tardi") } else if p.pending && p.replies.is_empty() { Some("Caricamento…") } else { None };
        let docked = window.viewport_size().width >= px(dock_min_width(t));
        let panel = side_panel("thread-panel", window, t)
            .child(div().h(px(t.header)).flex_shrink_0().px_4().flex().items_center().justify_between()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Thread"))
                .child(div().id("thread-close").cursor_pointer().text_color(rgb(t.accent)).child("Chiudi")
                    .on_click(cx.listener(|this, _, _, cx| { this.close_thread(); cx.notify(); }))))
            .child(div().id("thread-content").flex_1().min_h_0().overflow_y_scroll().track_scroll(&p.scroll)
                .p_4().flex().flex_col().gap_4()
                .child(row(&p.root_row))
                .child(div().h(px(1.)).w_full().bg(rgb(t.border)))
                .children(status.map(|s| div().text_size(px(12.)).text_color(rgb(t.accent)).child(s)))
                .children(p.replies.iter().enumerate().map(|(i, r)| row(r).children(answer(i, r, cx))))
                .children(p.outgoing.iter().enumerate().map(|(i, o)| {
                    let failed = matches!(o.delivery, Delivery::Failed);
                    div().id(("thread-outgoing", i)).flex().flex_col().gap_1().text_color(rgb(t.secondary))
                        .child(div().text_size(px(12.)).child(if failed { "Non inviato · tocca per riprovare" } else { "Invio…" }))
                        .child(div().child(o.body.clone()))
                        .when(failed, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(o) = this.thread.as_mut().and_then(|p| p.outgoing.get_mut(i)) {
                                o.delivery = Delivery::Queued;
                                cx.notify();
                            }
                        })))
                })))
            .children(p.reply_to.as_ref().map(|(_, label)| div().flex_shrink_0().px_4().py_1().flex().items_center().justify_between().border_t_1().border_color(rgb(t.border)).text_size(px(12.)).text_color(rgb(t.secondary))
                .child(div().min_w_0().overflow_hidden().child(label.clone()))
                .child(div().id("thread-reply-clear").cursor_pointer().px_2().child("×")
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
        assert_eq!(super::dock_min_width(matrix_desktop::LIGHT), 1112.);
        assert_eq!(super::dock_min_width(matrix_desktop::DARK), super::dock_min_width(matrix_desktop::LIGHT));
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
}
