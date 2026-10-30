//! Read-only side panel of one thread: the root, then its replies, oldest first.
//! The engine answers each request with a snapshot of the SDK thread timeline.
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

pub(crate) struct ThreadRow {
    who: String,
    text: String,
    time: String,
}

impl ThreadRow {
    fn from_event(e: &HistoryEvent) -> Self {
        let who = if e.outgoing { "Tu".to_owned() } else { e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone()) };
        let text = if matrix_engine::history::is_image_body(&e.body) { "Immagine".to_owned() } else { e.body.clone() };
        let ms = e.timestamp_ms;
        Self { who, text, time: format!("{:02}:{:02} UTC", ms / 3_600_000 % 24, ms / 60_000 % 60) }
    }

    pub(crate) fn from_row(row: &matrix_desktop::DemoEvent) -> Self {
        let who = match (&row.sender, row.outgoing) {
            (Some(s), false) => s.name.clone().unwrap_or_else(|| s.id.clone()),
            _ => "Tu".to_owned(),
        };
        let text = if matrix_engine::history::is_image_body(&row.text) { "Immagine".to_owned() } else { row.text.clone() };
        Self { who, text, time: row.time.clone() }
    }
}

pub(crate) struct ThreadPanel {
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

impl ThreadPanel {
    pub(crate) fn new(scope: RoomScope, root: EventId, root_row: ThreadRow, total: u32) -> Self {
        Self {
            scope, root, root_row, replies: Vec::new(), total, want: STEP, pending: false, dirty: true,
            loaded_at: None, failed: false, stalled: false, at_start: false, growing: None, anchor: None,
            stick_bottom: true, scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn activity(&mut self, scope: &RoomScope) {
        self.dirty |= self.scope == *scope;
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
            p.apply(result.map(|page| *page));
            cx.notify();
        }
    }

    pub(crate) fn close_thread(&mut self) {
        self.thread = None;
    }

    /// One tick: open on a click on a summary, read when asked, load older at the top.
    pub(crate) fn drive_thread(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.timeline.as_ref().and_then(|t| t.want_thread.borrow_mut().take())
            && let Some((row, total)) = self.timeline.as_ref().and_then(|t| t.thread_root(&id))
        {
            self.thread = Some(ThreadPanel::new(self.active_scope.clone(), id, row, total));
            cx.notify();
        }
        let Some(p) = &mut self.thread else { return };
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
        if p.dirty
            && !p.pending
            && let Some((sender, _)) = &mut self.routes.operation
        {
            let command = OperationCommand::Thread { scope: p.scope.clone(), root: p.root.clone(), want: p.want };
            match sender.try_send(command) {
                Ok(()) => (p.pending, p.dirty) = (true, false),
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
        let status = if p.failed { Some("Impossibile caricare · riprova più tardi") } else if p.pending && p.replies.is_empty() { Some("Caricamento…") } else { None };
        // Wide window: a column next to the chat. Narrow: a sheet over it (as the room sheet).
        let docked = window.viewport_size().width >= px(800.);
        let panel = div().id("thread-panel").occlude()
            .when(docked, |d| d.h_full().w(px(360.)).flex_shrink_0())
            .when(!docked, |d| d.absolute().right_0().top_0().bottom_0().w(window.viewport_size().width.min(px(360.))))
            .flex().flex_col().bg(rgb(t.surface)).border_l_1().border_color(rgb(t.border))
            .child(div().h(px(t.header)).flex_shrink_0().px_4().flex().items_center().justify_between()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Thread"))
                .child(div().id("thread-close").cursor_pointer().text_color(rgb(t.accent)).child("Chiudi")
                    .on_click(cx.listener(|this, _, _, cx| { this.close_thread(); cx.notify(); }))))
            .child(div().id("thread-content").flex_1().min_h_0().overflow_y_scroll().track_scroll(&p.scroll)
                .p_4().flex().flex_col().gap_4()
                .child(row(&p.root_row))
                .child(div().h(px(1.)).w_full().bg(rgb(t.border)))
                .children(status.map(|s| div().text_size(px(12.)).text_color(rgb(t.accent)).child(s)))
                .children(p.replies.iter().map(row)));
        if docked { panel.into_any_element() } else { gpui::deferred(panel).with_priority(1).into_any_element() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};
    use matrix_engine::history::HistorySender;

    fn event(n: usize) -> HistoryEvent {
        HistoryEvent {
            event_id: EventId::new(format!("$r{n}")),
            body: format!("risposta {n}"),
            outgoing: false,
            timestamp_ms: 0,
            sender: HistorySender::new("@a:t", None),
            video: None,
            styles: Vec::new(),
            thread: None,
        }
    }

    fn page(replies: usize) -> ThreadPage {
        ThreadPage { root: None, replies: (1..=replies).map(event).collect(), reactions: Vec::new(), at_start: false }
    }

    fn panel(total: u32) -> ThreadPanel {
        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:t") };
        let root = ThreadRow { who: "x".into(), text: "radice".into(), time: String::new() };
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
}
