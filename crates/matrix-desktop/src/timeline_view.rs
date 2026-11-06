//! Native viewport-only presentation of offline fixtures or scoped server rows.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{
    Bounds, ClipboardItem, FocusHandle, HighlightStyle, IntoElement, ListAlignment, ListOffset,
    ListState, MouseButton, Pixels, StyledText, TextLayout, canvas, div, list, prelude::*, px,
    relative, rgb,
};
use matrix_desktop::{
    DemoHistory, DemoSendState, DesignTokens, LinkConfirmation, TextSelection, message_links,
};
use matrix_domain::{RoomScope, ScopedTimelineId};

/// The body's text layout seen through the avatar gaps of its mention pills: positions and
/// indices are those of the message text, not of the drawn string.
#[derive(Clone)]
struct PillLayout {
    layout: TextLayout,
    gaps: Rc<matrix_desktop::mention_gaps::Gaps>,
}

impl PillLayout {
    fn index_for_position(&self, position: gpui::Point<Pixels>) -> Result<usize, usize> {
        self.layout.index_for_position(position).map(|i| self.gaps.text_index(i)).map_err(|i| self.gaps.text_index(i))
    }
    fn position_for_index(&self, index: usize) -> Option<gpui::Point<Pixels>> {
        self.layout.position_for_index(self.gaps.start(index))
    }
    fn line_height(&self) -> Pixels {
        self.layout.line_height()
    }
}

type RowBounds = Rc<RefCell<Option<Bounds<Pixels>>>>;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
type ConfirmationBounds = Rc<RefCell<(u64, [Option<Bounds<Pixels>>; 3])>>;
type RowLayout = (
    usize,
    TextLayout,
    TextLayout,
    String,
    RowBounds,
    &'static str,
    bool,
);

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
type SemanticBounds = Rc<RefCell<std::collections::HashMap<ScopedTimelineId, Bounds<Pixels>>>>;

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
type SelectionBounds = Rc<
    RefCell<
        std::collections::HashMap<
            (
                ScopedTimelineId,
                matrix_desktop::accessibility::selection::Control,
            ),
            Bounds<Pixels>,
        >,
    >,
>;

fn offline_delivery_callback(
    sources: Rc<RefCell<matrix_desktop::OfflineDelivery>>,
    history: Rc<RefCell<DemoHistory>>,
    target: matrix_desktop::OfflineDeliveryTarget,
    live: bool,
) -> impl Fn() -> Option<usize> {
    move || {
        if live {
            return None;
        }
        sources
            .borrow_mut()
            .apply(&target, &mut history.borrow_mut())
    }
}

fn close_viewer(
    viewing: &RefCell<Option<matrix_domain::EventId>>,
    focus: &FocusHandle,
    key: &str,
    window: &mut gpui::Window,
) -> bool {
    if key != "escape" || viewing.borrow_mut().take().is_none() {
        return false;
    }
    focus.focus(window);
    true
}

/// Painted bounds of the parts of a video bubble, by event and part name.
type VideoParts =
    std::collections::HashMap<(matrix_domain::TimelineId, &'static str), Bounds<Pixels>>;

type PollParts = std::collections::HashMap<(matrix_domain::TimelineId, crate::poll_view::PollPart), Bounds<Pixels>>;

const SENDER_AVATAR_PX: f32 = 32.;

/// Avatar or name of a bubble: a click asks the shell to open that member's profile.
fn profile_target(
    base: gpui::Stateful<gpui::Div>,
    user_id: String,
    want: Rc<RefCell<Option<String>>>,
    painted: impl Fn(Bounds<Pixels>) + 'static,
    child: impl IntoElement,
) -> gpui::Stateful<gpui::Div> {
    base.relative().cursor_pointer()
        .on_click(move |_, window, _| {
            *want.borrow_mut() = Some(user_id.clone());
            window.refresh();
        })
        .child(child)
        .child(canvas(|_, _, _| (), move |bounds, _, _, _| painted(bounds)).absolute().inset_0())
}
const STATUS_AVATAR_PX: f32 = 12.;

/// Where a Tab press over the «Scarica» icons went.
pub(crate) enum IconStep {
    Moved,
    /// Past the last icon (or before the first): the key goes on to the next stop of the ring.
    Left,
    Ignored,
}

/// The message a room search jumped to: its bubble flashes until `flash_until`, the term stays
/// marked in the text while the search is open.
#[derive(Clone)]
pub struct SearchMark {
    pub event: matrix_domain::EventId,
    pub term: String,
    pub flash_until: std::time::Instant,
    pub ended: bool,
}

/// The accent over the bubble colour, for the flash after a jump.
fn flash(base: u32, accent: u32) -> u32 {
    let mix = |shift: u32| (((base >> shift) & 0xff) * 7 + ((accent >> shift) & 0xff) * 3) / 10;
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

pub struct TimelineView {
    search_mark: Rc<RefCell<Option<SearchMark>>>,
    offline_delivery: Rc<RefCell<matrix_desktop::OfflineDelivery>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    offline_cursor: Rc<Cell<Option<u64>>>,
    event_id_copies: Rc<RefCell<matrix_desktop::EventIdCopies>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    event_id_cursor: Rc<Cell<Option<u64>>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    event_id_bounds: Rc<RefCell<std::collections::HashMap<u64, Bounds<Pixels>>>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    confirmation_bounds: ConfirmationBounds,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    confirmation_focus: Rc<Cell<(u64, bool)>>,
    link_bounds: Rc<RefCell<std::collections::HashMap<u64, Bounds<Pixels>>>>,
    link_sources: Rc<RefCell<matrix_desktop::MessageLinkRequests>>,
    link_cursor: Rc<Cell<Option<u64>>>,
    /// Keyboard stop on a user mention pill: its message and the pill's start in the text.
    pill_cursor: Rc<RefCell<Option<(ScopedTimelineId, usize)>>>,
    pill_bounds: Rc<RefCell<std::collections::HashMap<(ScopedTimelineId, usize), Bounds<Pixels>>>>,
    link_hover: Rc<Cell<Option<u64>>>,
    pub(crate) viewing: Rc<RefCell<Option<matrix_domain::EventId>>>,
    /// Full-resolution picture of the open large view (loading, ready or failed).
    pub(crate) original:
        Rc<RefCell<Option<(matrix_domain::EventId, crate::media_view::InlineImage)>>>,
    /// Set by a click or «Riprova»; the shell turns it into the original read.
    pub(crate) want_original: Rc<RefCell<Option<matrix_domain::EventId>>>,
    /// A video whose download for the system player was asked for and not yet sent.
    pub(crate) want_video: Rc<RefCell<Option<matrix_domain::EventId>>>,
    /// Set by a click on a thread summary; the shell opens the panel.
    pub(crate) want_thread: Rc<RefCell<Option<matrix_domain::EventId>>>,
    /// Click on a bubble's avatar or name: the shell opens that member's profile in the room sheet.
    pub(crate) want_profile: Rc<RefCell<Option<String>>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    profile_painted: Rc<RefCell<std::collections::HashMap<matrix_domain::ScopedTimelineId, Bounds<Pixels>>>>,
    /// «Scarica»: what each save is doing, the click waiting for the file picker, and a stop request.
    pub(crate) saves: Rc<RefCell<matrix_desktop::Saves>>,
    pub(crate) want_save: Rc<RefCell<Option<(matrix_domain::EventId, String)>>>,
    pub(crate) stop_save: Rc<Cell<bool>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    save_painted: Rc<RefCell<std::collections::HashMap<matrix_domain::EventId, Bounds<Pixels>>>>,
    link_menu: Rc<RefCell<matrix_desktop::LinkMenu>>,
    link_menu_at: Rc<Cell<gpui::Point<Pixels>>>,
    /// Last painted bounds of each video bubble part, by event and part name.
    pub(crate) video_painted: Rc<RefCell<VideoParts>>,
    /// Last painted bounds of each poll bubble part, by event.
    pub(crate) poll_painted: Rc<RefCell<PollParts>>,
    pub(crate) images: Rc<
        RefCell<std::collections::HashMap<matrix_domain::EventId, crate::media_view::InlineImage>>,
    >,
    requested_link: Rc<RefCell<Option<matrix_desktop::MessageLinkTarget>>>,
    /// Sender pictures by mxc, copied from the shell's avatar cache.
    pub(crate) sender_pictures:
        Rc<RefCell<std::collections::HashMap<String, std::sync::Arc<gpui::RenderImage>>>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    selection_cursor: Rc<
        RefCell<
            Option<(
                ScopedTimelineId,
                matrix_desktop::accessibility::selection::Control,
            )>,
        >,
    >,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    selection_bounds: SelectionBounds,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    actions_bounds: SemanticBounds,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    actions_cursor: RefCell<Option<ScopedTimelineId>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    retry_bounds: SemanticBounds,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    retry_cursor: RefCell<Option<ScopedTimelineId>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    poll_cursor: RefCell<Option<(ScopedTimelineId, usize)>>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    semantic_bounds: SemanticBounds,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    semantic_cursor: RefCell<Option<ScopedTimelineId>>,
    history: Rc<RefCell<DemoHistory>>,
    /// Joined members of the shown room; sender names appear only above 2.
    member_count: Cell<u64>,
    peer_receipts: Rc<RefCell<Vec<matrix_engine::receipts::PublicReadReceipt>>>,
    debug: Cell<bool>,
    live: bool,
    state: ListState,
    scrollbar: crate::scrollbar_view::Scrollbar,
    extent: crate::scrollbar_view::HistoryExtent,
    probe: bool,
    selection: Rc<RefCell<TextSelection>>,
    focus: FocusHandle,
    dragging: Rc<Cell<Option<u64>>>,
    link: Rc<RefCell<LinkConfirmation>>,
    click_origin: Rc<RefCell<Option<(ScopedTimelineId, usize)>>>,
}

impl TimelineView {
    fn selection_key_callback(
        &self,
        mac: bool,
    ) -> impl Fn(&gpui::KeyDownEvent, &mut gpui::Window, &mut gpui::App) + use<> {
        let key_selection = self.selection.clone();
        let key_history = self.history.clone();
        let key_link = self.link.clone();
        let key_menu = self.link_menu.clone();
        let generation = key_selection.borrow().generation();
        move |event, window, cx| {
            if key_selection.borrow().generation() != generation {
                return;
            }
            let command = crate::editing_command::command_modifier(&event.keystroke.modifiers, mac);
            if command && event.keystroke.key == "c" {
                let history = key_history.borrow();
                if let Some(text) = history
                    .events()
                    .iter()
                    .find_map(|e| key_selection.borrow().copy(&e.id, &e.text))
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                }
                cx.stop_propagation();
            } else if !event.keystroke.modifiers.alt
                && (!event.keystroke.modifiers.platform || (mac && command))
                && let Some(key) = if command {
                    (event.keystroke.key == "a" && !event.keystroke.modifiers.shift)
                        .then_some("select-all")
                } else {
                    matches!(
                        event.keystroke.key.as_str(),
                        "left" | "right" | "home" | "end"
                    )
                    .then_some(event.keystroke.key.as_str())
                }
                && key_history.borrow().events().iter().any(|e| {
                    key_selection.borrow_mut().key(
                        &e.id,
                        &e.text,
                        key,
                        event.keystroke.modifiers.shift,
                    )
                })
            {
                window.refresh();
                cx.stop_propagation();
            } else if event.keystroke.key == "escape" {
                key_selection.borrow_mut().clear();
                key_link.borrow_mut().cancel();
                key_menu.borrow_mut().close();
                window.refresh();
                cx.stop_propagation();
            }
        }
    }

    pub fn new(scope: RoomScope, focus: FocusHandle) -> Self {
        // These opt-in settings select only deterministic offline fixture workloads.
        let count = if std::env::var("MATRIX_DESKTOP_DEMO_EVENTS").as_deref() == Ok("10000") {
            10_000
        } else {
            2_000
        };
        Self::with_history(DemoHistory::new(scope, count), focus, false)
    }

    pub fn set_debug(&self, on: bool) {
        self.debug.set(on);
    }

    pub fn set_member_count(&self, count: u64) {
        self.member_count.set(count);
    }

    pub fn set_peer_receipts(&self, receipts: &[matrix_engine::receipts::PublicReadReceipt]) {
        *self.peer_receipts.borrow_mut() = receipts.to_vec();
    }

    pub fn connected(scope: RoomScope, focus: FocusHandle) -> Self {
        Self::with_history(DemoHistory::new(scope, 0), focus, true)
    }

    fn with_history(history: DemoHistory, focus: FocusHandle, live: bool) -> Self {
        let focus = focus.tab_stop(true);
        let count = history.events().len();
        Self {
            offline_delivery: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            offline_cursor: Rc::default(),
            event_id_copies: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            event_id_cursor: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            event_id_bounds: Rc::default(),
            link_bounds: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            confirmation_bounds: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            confirmation_focus: Rc::default(),
            link_sources: Rc::default(),
            link_cursor: Rc::default(),
            pill_cursor: Rc::default(),
            pill_bounds: Rc::default(),
            link_hover: Rc::default(),
            viewing: Rc::default(),
            original: Rc::default(),
            want_original: Rc::default(),
            want_video: Rc::default(),
            want_thread: Rc::default(),
            want_profile: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            profile_painted: Rc::default(),
            saves: Rc::default(),
            want_save: Rc::default(),
            stop_save: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            save_painted: Rc::default(),
            link_menu: Rc::default(),
            link_menu_at: Rc::default(),
            images: Rc::default(),
            sender_pictures: Rc::default(),
            video_painted: Rc::default(),
            poll_painted: Rc::default(),
            requested_link: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            selection_cursor: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            selection_bounds: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            actions_bounds: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            actions_cursor: RefCell::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            retry_bounds: Rc::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            retry_cursor: RefCell::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            poll_cursor: RefCell::default(),
            history: Rc::new(RefCell::new(history)),
            // The offline fixture stands for a group chat.
            member_count: Cell::new(if live { 0 } else { 3 }),
            peer_receipts: Rc::default(),
            debug: Cell::new(false),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            semantic_cursor: RefCell::default(),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            semantic_bounds: Rc::default(),
            live,
            state: ListState::new(count, ListAlignment::Bottom, px(80.)),
            scrollbar: Default::default(),
            extent: Rc::new(RefCell::new((
                count,
                0.,
                matrix_desktop::VirtualScrollExtent::new(count),
            ))),
            probe: std::env::var("MATRIX_DESKTOP_TIMELINE_PROBE").as_deref() == Ok("1"),
            selection: Rc::default(),
            search_mark: Rc::default(),
            focus,
            dragging: Rc::default(),
            link: Rc::default(),
            click_origin: Rc::default(),
        }
    }

    /// Offline-only benchmark driver: same list scroll path, not OS input.
    #[cfg(feature = "perf-probe")]
    pub fn scroll_performance_fixture(&self) {
        if !self.live {
            self.state.scroll_by(px(48.));
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.history.borrow().events().len()
    }

    #[cfg(test)]
    pub(crate) fn viewport_centre(&self) -> gpui::Point<gpui::Pixels> {
        self.state.viewport_bounds().center()
    }

    #[cfg(test)]
    pub(crate) fn thumb_centre(&self) -> Option<gpui::Point<gpui::Pixels>> {
        self.scrollbar.thumb_centre()
    }

    #[cfg(test)]
    pub(crate) fn scroll_to_row(&self, item_ix: usize, offset_in_item: gpui::Pixels) {
        self.state.scroll_to(ListOffset {
            item_ix,
            offset_in_item,
        });
    }

    /// Event ID and pixel offset of the row at the top of the viewport.
    #[cfg(test)]
    pub(crate) fn top_row(&self) -> (matrix_domain::TimelineId, gpui::Pixels) {
        let top = self.state.logical_scroll_top();
        (
            self.history.borrow().events()[top.item_ix].id.id.clone(),
            top.offset_in_item,
        )
    }

    #[cfg(test)]
    pub(crate) fn sidebar_history_probe(&self) -> impl Fn(&Self) -> bool + use<> {
        let history = self.history.clone();
        let ids: Vec<_> = history
            .borrow()
            .events()
            .iter()
            .map(|event| event.id.clone())
            .collect();
        move |current| {
            Rc::ptr_eq(&history, &current.history)
                && current
                    .history
                    .borrow()
                    .events()
                    .iter()
                    .map(|event| &event.id)
                    .eq(ids.iter())
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn scrollbar_snapshot(
        &self,
        scope: matrix_domain::RoomScope,
        scale: f64,
    ) -> Option<matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot> {
        self.scrollbar.semantic_snapshot(
            crate::scrollbar_view::ScrollTarget::History(self.state.clone(), self.extent.clone()),
            matrix_desktop::accessibility::scrollbars::Pane::History(scope),
            scale,
        )
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn set_scroll_fraction(&self, fraction: f64) {
        self.scrollbar.set_fraction(
            crate::scrollbar_view::ScrollTarget::History(self.state.clone(), self.extent.clone()),
            fraction,
        );
    }

    /// Scrolls the row of `event` to the top; false when it is not among the loaded rows.
    pub fn scroll_to_event(&self, event: &matrix_domain::EventId) -> bool {
        let Some(item_ix) = self.history.borrow().events().iter().position(|e| matches!(&e.id.id, matrix_domain::TimelineId::Server(id) if id == event)) else {
            return false;
        };
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        self.semantic_cursor.replace(None);
        self.state.scroll_to(ListOffset { item_ix, offset_in_item: px(0.) });
        true
    }

    pub fn set_search_mark(&self, mark: Option<SearchMark>) {
        *self.search_mark.borrow_mut() = mark;
    }

    /// True once, when the flash of the mark has just run out (the shell repaints).
    pub fn flash_ended(&self) -> bool {
        match self.search_mark.borrow_mut().as_mut() {
            Some(m) if !m.ended && m.flash_until <= std::time::Instant::now() => {
                m.ended = true;
                true
            }
            _ => false,
        }
    }

    pub fn jump_to_boundary(&self, last: bool) {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        self.semantic_cursor.replace(None);
        self.state.scroll_to(ListOffset {
            item_ix: if last {
                self.history.borrow().events().len()
            } else {
                0
            },
            offset_in_item: px(0.),
        });
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn semantic_snapshot(
        &self,
        nodes: &mut matrix_desktop::accessibility::HistoryNodes,
        window: &gpui::Window,
    ) -> matrix_desktop::accessibility::HistorySnapshot {
        let history = self.history.borrow();
        let start = self
            .semantic_cursor
            .borrow()
            .as_ref()
            .and_then(|id| history.events().iter().position(|event| event.id == *id))
            .unwrap_or_else(|| self.state.logical_scroll_top().item_ix);
        let mut snapshot = nodes.project(&history, start, self.focus.is_focused(window));
        if !self.live {
            snapshot.mark_offline();
        }
        let scale = f64::from(window.scale_factor());
        for message in &mut snapshot.messages {
            if !self.live {
                message.offline_delivery = history
                    .events()
                    .iter()
                    .find(|e| e.id == message.event)
                    .and_then(|e| self.offline_delivery.borrow_mut().target(e))
                    .map(
                        |target| matrix_desktop::accessibility::offline_delivery::Snapshot {
                            focused: self.focus.is_focused(window)
                                && self.offline_cursor.get() == Some(target.instance()),
                            bounds: self.retry_bounds.borrow().get(target.event()).map(|b| {
                                accesskit::Rect::new(
                                    f64::from(f32::from(b.left())) * scale,
                                    f64::from(f32::from(b.top())) * scale,
                                    f64::from(f32::from(b.right())) * scale,
                                    f64::from(f32::from(b.bottom())) * scale,
                                )
                            }),
                            target,
                        },
                    );
            }
            let at = history.events().iter().position(|e| e.id == message.event);
            if let Some(at) = at
                && let Some(name) = history.sender_label(at, self.member_count.get())
                && let Some(sender) = &history.events()[at].sender
            {
                message.profile = Some(matrix_desktop::accessibility::ProfileLink {
                    user_id: sender.id.clone(),
                    name,
                    bounds: self.profile_painted.borrow().get(&message.event).map(|b| accesskit::Rect::new(
                        f64::from(f32::from(b.left())) * scale,
                        f64::from(f32::from(b.top())) * scale,
                        f64::from(f32::from(b.right())) * scale,
                        f64::from(f32::from(b.bottom())) * scale,
                    )),
                });
            }
            if self.live {
                message.event_id_copy = self
                    .event_id_copies
                    .borrow_mut()
                    .target(&message.event, &message.text)
                    .map(
                        |target| matrix_desktop::accessibility::event_id_copy::Snapshot {
                            bounds: self.event_id_bounds.borrow().get(&target.instance()).map(
                                |b| {
                                    accesskit::Rect::new(
                                        f64::from(f32::from(b.left())) * scale,
                                        f64::from(f32::from(b.top())) * scale,
                                        f64::from(f32::from(b.right())) * scale,
                                        f64::from(f32::from(b.bottom())) * scale,
                                    )
                                },
                            ),
                            focused: self.focus.is_focused(window)
                                && self.event_id_cursor.get() == Some(target.instance()),
                            target,
                        },
                    );
            }
            if let Some(at) = at {
                let event = &history.events()[at];
                message.pills = matrix_desktop::mention_gaps::fitting(&event.text, &event.mentions)
                    .filter_map(|m| match &m.kind {
                        matrix_engine::rich_text::MentionKind::User { sender, .. } => Some((m.range.start, sender)),
                        _ => None,
                    })
                    .take(matrix_desktop::accessibility::PILLS)
                    .map(|(start, sender)| matrix_desktop::accessibility::PillLink {
                        user_id: sender.id.clone(),
                        name: sender.name.clone().unwrap_or_else(|| sender.id.clone()),
                        start,
                        bounds: self.pill_bounds.borrow().get(&(message.event.clone(), start)).map(|b| accesskit::Rect::new(
                            f64::from(f32::from(b.left())) * scale,
                            f64::from(f32::from(b.top())) * scale,
                            f64::from(f32::from(b.right())) * scale,
                            f64::from(f32::from(b.bottom())) * scale,
                        )),
                        focused: self.focus.is_focused(window)
                            && self.pill_cursor.borrow().as_ref() == Some(&(message.event.clone(), start)),
                    })
                    .collect();
            }
            message.links = self
                .link_sources
                .borrow_mut()
                .targets(&message.event, &message.text)
                .into_iter()
                .map(|target| matrix_desktop::accessibility::links::Snapshot {
                    focused: self.focus.is_focused(window)
                        && self.link_cursor.get() == Some(target.instance()),
                    requested: self.link.borrow().pending().is_some()
                        && self
                            .requested_link
                            .borrow()
                            .as_ref()
                            .is_some_and(|l| l.instance() == target.instance()),
                    bounds: self.link_bounds.borrow().get(&target.instance()).map(|b| {
                        accesskit::Rect::new(
                            f64::from(f32::from(b.left())) * scale,
                            f64::from(f32::from(b.top())) * scale,
                            f64::from(f32::from(b.right())) * scale,
                            f64::from(f32::from(b.bottom())) * scale,
                        )
                    }),
                    target,
                })
                .collect();
            let selection = self.selection.borrow();
            message.selection.generation = selection.generation();
            message.selection.text =
                selection.is_active(&message.event, &message.text).then(|| {
                    selection
                        .copy(&message.event, &message.text)
                        .unwrap_or("")
                        .to_owned()
                });
            message.selection.selected = selection.range(&message.event, &message.text).is_some();
            if self.focus.is_focused(window) {
                message.selection.focused = self
                    .selection_cursor
                    .borrow()
                    .as_ref()
                    .filter(|(id, _)| *id == message.event)
                    .map(|(_, c)| *c);
            }
            for control in matrix_desktop::accessibility::selection::Control::ALL {
                if let Some(b) = self
                    .selection_bounds
                    .borrow()
                    .get(&(message.event.clone(), control))
                {
                    message.selection.bounds[control as usize] = Some(accesskit::Rect::new(
                        f64::from(f32::from(b.left())) * scale,
                        f64::from(f32::from(b.top())) * scale,
                        f64::from(f32::from(b.right())) * scale,
                        f64::from(f32::from(b.bottom())) * scale,
                    ));
                }
            }
            if message.actions.is_some() {
                message.actions_focused = self.focus.is_focused(window)
                    && self.actions_cursor.borrow().as_ref() == Some(&message.event);
                if let Some(bounds) = self.actions_bounds.borrow().get(&message.event) {
                    message.actions_bounds = Some(accesskit::Rect::new(
                        f64::from(f32::from(bounds.left())) * scale,
                        f64::from(f32::from(bounds.top())) * scale,
                        f64::from(f32::from(bounds.right())) * scale,
                        f64::from(f32::from(bounds.bottom())) * scale,
                    ));
                }
            }
            if message.play.is_some()
                && let matrix_domain::TimelineId::Server(id) = &message.event.id
                && let Some(bounds) = self
                    .video_painted
                    .borrow()
                    .get(&(matrix_domain::TimelineId::Server(id.clone()), "picture"))
            {
                message.play_bounds = Some(accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                ));
            }
            if message.poll.is_some() {
                message.poll_focus = self.poll_cursor.borrow().as_ref().filter(|(e, _)| *e == message.event && self.focus.is_focused(window)).map(|(_, ix)| *ix);
                let parts = self.poll_painted.borrow();
                let rect = |b: &Bounds<Pixels>| accesskit::Rect::new(
                    f64::from(f32::from(b.left())) * scale,
                    f64::from(f32::from(b.top())) * scale,
                    f64::from(f32::from(b.right())) * scale,
                    f64::from(f32::from(b.bottom())) * scale,
                );
                let at = |part| parts.get(&(message.event.id.clone(), part)).map(rect);
                message.poll_bounds = at(crate::poll_view::PollPart::Bubble);
                message.poll_option_bounds = (0..message.poll.as_ref().map_or(0, |p| p.answers.len()))
                    .map(|ix| at(crate::poll_view::PollPart::Option(ix)))
                    .collect();
            }
            if message.save.is_some()
                && let matrix_domain::TimelineId::Server(id) = &message.event.id
                && let Some(bounds) = self.save_painted.borrow().get(id)
            {
                message.save_bounds = Some(accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                ));
            }
            if message.retry.is_some() {
                message.retry_focused = self.focus.is_focused(window)
                    && self.retry_cursor.borrow().as_ref() == Some(&message.event);
                if let Some(bounds) = self.retry_bounds.borrow().get(&message.event) {
                    message.retry_bounds = Some(accesskit::Rect::new(
                        f64::from(f32::from(bounds.left())) * scale,
                        f64::from(f32::from(bounds.top())) * scale,
                        f64::from(f32::from(bounds.right())) * scale,
                        f64::from(f32::from(bounds.bottom())) * scale,
                    ));
                }
            }
            if let Some(bounds) = self.semantic_bounds.borrow().get(&message.event) {
                message.bounds = Some(accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                ));
            }
        }
        use matrix_desktop::accessibility::links::{ConfirmationControl, ConfirmationSnapshot};
        snapshot.confirmation = self
            .link
            .borrow()
            .capture()
            .filter(|r| self.link_sources.borrow().is_current(r.source()))
            .map(|request| {
                let generation = request.generation();
                let focused = self.focus.is_focused(window).then_some(
                    if self.confirmation_focus.get() == (generation, true) {
                        ConfirmationControl::Open
                    } else {
                        ConfirmationControl::Cancel
                    },
                );
                let bounds = self.confirmation_bounds.borrow();
                ConfirmationSnapshot {
                    request,
                    focused,
                    bounds: if bounds.0 == generation {
                        bounds.1.map(|b| {
                            b.map(|b| {
                                accesskit::Rect::new(
                                    f64::from(f32::from(b.left())) * scale,
                                    f64::from(f32::from(b.top())) * scale,
                                    f64::from(f32::from(b.right())) * scale,
                                    f64::from(f32::from(b.bottom())) * scale,
                                )
                            })
                        })
                    } else {
                        [None; 3]
                    },
                }
            });
        snapshot
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn focus_semantic_message(&self, id: &ScopedTimelineId, window: &mut gpui::Window) -> bool {
        self.event_id_cursor.set(None);
        self.offline_cursor.set(None);
        self.link_cursor.set(None);
        self.pill_cursor.replace(None);
        self.requested_link.replace(None);
        self.link.borrow_mut().cancel();
        self.selection_cursor.replace(None);
        self.retry_cursor.replace(None);
        self.poll_cursor.replace(None);
        self.actions_cursor.replace(None);
        let Some(index) = self
            .history
            .borrow()
            .events()
            .iter()
            .position(|event| event.id == *id)
        else {
            return false;
        };
        self.link_sources.borrow_mut().retain_event(id);
        self.semantic_cursor.replace(Some(id.clone()));
        self.state.scroll_to_reveal_item(index);
        self.focus.focus(window);
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_offline_delivery(
        &self,
        intent: &matrix_desktop::accessibility::offline_delivery::Intent,
        window: &mut gpui::Window,
    ) -> bool {
        if self.live
            || !self
                .offline_delivery
                .borrow()
                .is_current(&intent.target, &self.history.borrow())
        {
            return false;
        }
        if !self.focus_semantic_message(intent.target.event(), window) {
            return false;
        }
        let target = if intent.focus_only {
            intent.target.clone()
        } else {
            let Some(index) = offline_delivery_callback(
                self.offline_delivery.clone(),
                self.history.clone(),
                intent.target.clone(),
                self.live,
            )() else {
                return false;
            };
            self.state.splice(index..index + 1, 1);
            self.offline_delivery
                .borrow_mut()
                .target(&self.history.borrow().events()[index])
                .unwrap()
        };
        self.offline_cursor.set(Some(target.instance()));
        true
    }

    /// Screen-reader click or Enter on a video message: same path as a click on its picture.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn play_video(&self, target: &matrix_domain::ScopedTimelineId, window: &mut gpui::Window) {
        if let matrix_domain::TimelineId::Server(id) = &target.id {
            *self.viewing.borrow_mut() = Some(id.clone());
            *self.original.borrow_mut() = None;
            *self.want_video.borrow_mut() = Some(id.clone());
            self.focus.focus(window);
        }
    }

    /// The save ended: clear it or show its error line (the row gets taller, so it is measured again).
    /// Size the sender declared for the attachment of `id`.
    pub(crate) fn attachment_size(&self, id: &matrix_domain::EventId) -> Option<u64> {
        let history = self.history.borrow();
        let event = history.events().iter().find(|e| matches!(&e.id.id, matrix_domain::TimelineId::Server(s) if s == id));
        event.and_then(|e| e.attachment_size)
    }

    /// Tab/Shift-Tab over the «Scarica» icons painted now. GPUI has no Tab key of its own here, but
    /// it keeps the tab stops of the frame: the timeline handle is the first, the icons follow.
    pub(crate) fn step_icons(&self, shift: bool, window: &mut gpui::Window, cx: &gpui::App) -> IconStep {
        let on_icon = self.focus.contains_focused(window, cx) && !self.focus.is_focused(window);
        if self.viewing().is_some() {
            return IconStep::Ignored;
        }
        if !on_icon && (shift || !self.focus.is_focused(window)) {
            return IconStep::Ignored;
        }
        if shift { window.focus_prev() } else { window.focus_next() }
        match (self.focus.is_focused(window), on_icon) {
            (false, _) => IconStep::Moved,
            (true, true) => IconStep::Left,
            (true, false) => IconStep::Ignored,
        }
    }

    /// Enter from outside: the first icon, or the last one when `backwards`; false when none is painted.
    pub(crate) fn enter_icons(&self, backwards: bool, window: &mut gpui::Window) -> bool {
        if self.viewing().is_some() {
            return false;
        }
        self.focus.focus(window);
        if backwards { window.focus_prev() } else { window.focus_next() }
        !self.focus.is_focused(window)
    }

    pub(crate) fn finish_save(&self, id: &matrix_domain::EventId, result: Result<(), matrix_engine::media::MediaError>) {
        self.saves.borrow_mut().finish(id, result);
        self.image_changed(id);
    }

    /// Screen-reader click on «Scarica»: same path as a click on the icon.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn save_attachment(&self, target: &matrix_domain::ScopedTimelineId) {
        let matrix_domain::TimelineId::Server(id) = &target.id else {
            return;
        };
        let name = self.history.borrow().events().iter().find(|e| e.id == *target).and_then(|e| e.attachment_name.clone());
        if let Some(name) = name
            && self.saves.borrow().state(id) != Some(matrix_desktop::SaveState::Running)
        {
            self.saves.borrow_mut().begin(id);
            *self.want_save.borrow_mut() = Some((id.clone(), name));
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_event_id_copy(
        &self,
        intent: &matrix_desktop::accessibility::event_id_copy::Intent,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> bool {
        if !self.live {
            return false;
        }
        let Some(id) =
            event_id_copy_callback(self.event_id_copies.clone(), intent.target.clone())()
        else {
            return false;
        };
        if !self.focus_semantic_message(intent.target.event(), window) {
            return false;
        }
        self.event_id_cursor.set(Some(intent.target.instance()));
        if !intent.focus_only {
            cx.write_to_clipboard(ClipboardItem::new_string(id));
        }
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_selection(
        &self,
        intent: &matrix_desktop::accessibility::selection::Intent,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> bool {
        use matrix_desktop::accessibility::selection::Control;
        if !intent.is_current(&self.selection.borrow())
            || !self
                .history
                .borrow()
                .events()
                .iter()
                .any(|e| e.id == intent.target && e.text == intent.text)
            || (intent.control != Control::Select
                && self
                    .selection
                    .borrow()
                    .range(&intent.target, &intent.text)
                    .is_none())
        {
            return false;
        }
        if !self.focus_semantic_message(&intent.target, window) {
            return false;
        }
        let mut focused = intent.control;
        if !intent.focus_only {
            match intent.control {
                Control::Select => {
                    self.selection
                        .borrow_mut()
                        .begin(intent.target.clone(), &intent.text, 0);
                    self.selection.borrow_mut().key(
                        &intent.target,
                        &intent.text,
                        "select-all",
                        false,
                    );
                    focused = Control::Copy;
                }
                Control::Copy => {
                    if let Some(text) = self.selection.borrow().copy(&intent.target, &intent.text) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                    }
                }
                Control::Cancel => {
                    self.selection.borrow_mut().clear();
                    return true;
                }
            }
        }
        self.selection_cursor
            .replace(Some((intent.target.clone(), focused)));
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_confirmation(
        &self,
        intent: &matrix_desktop::accessibility::links::ConfirmationIntent,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> bool {
        use matrix_desktop::accessibility::links::{ConfirmationControl, Intent};
        let request = &intent.request;
        if self
            .link
            .borrow()
            .capture()
            .is_none_or(|r| r.generation() != request.generation())
            || !self.link_sources.borrow().is_current(request.source())
            || !self
                .history
                .borrow()
                .events()
                .iter()
                .any(|e| e.id == *request.source().event() && e.text == request.source().text())
        {
            return false;
        }
        if intent.focus_only {
            self.confirmation_focus.set((
                request.generation(),
                intent.control == ConfirmationControl::Open,
            ));
            self.focus.focus(window);
        } else {
            let (cancel, confirm) = confirmation_callbacks(
                self.link.clone(),
                self.link_sources.clone(),
                request.clone(),
            );
            match intent.control {
                ConfirmationControl::Cancel => {
                    cancel();
                }
                ConfirmationControl::Open => {
                    if let Some(url) = confirm() {
                        cx.open_url(url.as_str());
                    }
                }
            }
            self.apply_link(
                &Intent {
                    target: request.source().clone(),
                    focus_only: true,
                },
                window,
            );
        }
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_pill(&self, id: &ScopedTimelineId, start: usize, window: &mut gpui::Window) -> bool {
        if !self.focus_semantic_message(id, window) {
            return false;
        }
        self.pill_cursor.replace(Some((id.clone(), start)));
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_link(
        &self,
        intent: &matrix_desktop::accessibility::links::Intent,
        window: &mut gpui::Window,
    ) -> bool {
        if !self
            .history
            .borrow()
            .events()
            .iter()
            .any(|e| e.id == *intent.target.event() && e.text == intent.target.text())
            || !self.link_sources.borrow_mut().focus(&intent.target)
        {
            return false;
        }
        if !self.focus_semantic_message(intent.target.event(), window) {
            return false;
        }
        self.link_cursor.set(Some(intent.target.instance()));
        if !intent.focus_only {
            self.link_sources
                .borrow_mut()
                .request(&intent.target, &mut self.link.borrow_mut());
            self.requested_link.replace(Some(intent.target.clone()));
        }
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn focus_semantic_retry(&self, id: &ScopedTimelineId, window: &mut gpui::Window) -> bool {
        if !self.live
            || !self.history.borrow().events().iter().any(|event| {
                event.id == *id
                    && event.send_state == Some(DemoSendState::Failed)
                    && matches!(event.id.id, matrix_domain::TimelineId::Pending(_))
            })
        {
            return false;
        }
        if !self.focus_semantic_message(id, window) {
            return false;
        }
        self.retry_cursor.replace(Some(id.clone()));
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn focus_semantic_poll(&self, id: &ScopedTimelineId, ix: usize, window: &mut gpui::Window) -> bool {
        if !self.live
            || !self.history.borrow().events().iter().any(|event| {
                event.id == *id
                    && matches!(id.id, matrix_domain::TimelineId::Server(_))
                    && event.poll.as_ref().is_some_and(|p| !p.ended && ix < p.answers.len())
            })
            || !self.focus_semantic_message(id, window)
        {
            return false;
        }
        self.poll_cursor.replace(Some((id.clone(), ix)));
        true
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn focus_semantic_actions(&self, id: &ScopedTimelineId, window: &mut gpui::Window) -> bool {
        if !self.live
            || !self.history.borrow().events().iter().any(|event| {
                event.id == *id
                    && event.send_state.is_none()
                    && matches!(event.id.id, matrix_domain::TimelineId::Server(_))
            })
        {
            return false;
        }
        if !self.focus_semantic_message(id, window) {
            return false;
        }
        self.actions_cursor.replace(Some(id.clone()));
        true
    }

    pub fn poll_of(&self, id: &ScopedTimelineId) -> Option<matrix_domain::Poll> {
        self.history.borrow().events().iter().find(|e| e.id == *id)?.poll.clone()
    }

    pub fn keyboard_menu_target(&self) -> Option<ScopedTimelineId> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused = self.actions_cursor.borrow().clone();
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused = None;
        self.history.borrow().keyboard_menu_target(focused.as_ref())
    }

    pub(crate) fn viewing(&self) -> Option<matrix_domain::EventId> {
        self.viewing.borrow().clone()
    }

    /// Open the large view of a loaded image; keyboard focus stays on the timeline, which handles Escape.
    #[cfg(test)]
    pub(crate) fn open_image(&self, id: matrix_domain::EventId, window: &mut gpui::Window) {
        if matches!(
            self.images.borrow().get(&id),
            Some(crate::media_view::InlineImage::Ready(_))
        ) {
            *self.viewing.borrow_mut() = Some(id);
            self.focus.focus(window);
        }
    }

    /// Escape closes the large view and returns focus to the timeline; true when consumed.
    pub(crate) fn viewer_key(&self, key: &str, window: &mut gpui::Window) -> bool {
        close_viewer(&self.viewing, &self.focus, key, window)
    }

    pub fn focus_timeline(&self, window: &mut gpui::Window) {
        self.focus.focus(window);
    }

    /// Re-measure the row of an image whose state changed.
    pub(crate) fn image_changed(&self, id: &matrix_domain::EventId) {
        let index = self
            .history
            .borrow()
            .events()
            .iter()
            .position(|e| matches!(&e.id.id, matrix_domain::TimelineId::Server(s) if s == id));
        if let Some(index) = index {
            self.state.splice(index..index + 1, 1);
        }
    }

    /// A sender picture arrived: redraw the rows that use it.
    pub(crate) fn avatar_changed(&self, mxc: &str) {
        let rows: Vec<usize> = {
            let history = self.history.borrow();
            let rows = history.events().iter().enumerate();
            rows.filter(|(_, e)| e.sender.as_ref().and_then(|s| s.avatar.as_deref()) == Some(mxc))
                .map(|(i, _)| i)
                .collect()
        };
        for index in rows {
            self.state.splice(index..index + 1, 1);
        }
    }

    pub(crate) fn sender_avatar_mxcs(&self) -> Vec<String> {
        let history = self.history.borrow();
        let unique: std::collections::BTreeSet<_> = history.sender_avatar_mxcs().collect();
        unique.into_iter().map(str::to_owned).collect()
    }

    /// Video rows of this room: their picture is the thumbnail, when there is one.
    pub(crate) fn video_events(&self) -> Vec<matrix_domain::EventId> {
        self.history
            .borrow()
            .events()
            .iter()
            .filter(|e| e.video.is_some())
            .filter_map(|e| match &e.id.id {
                matrix_domain::TimelineId::Server(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// Image and video rows of this room, newest first, for loading on arrival.
    pub(crate) fn image_events(&self) -> Vec<matrix_domain::EventId> {
        self.history
            .borrow()
            .events()
            .iter()
            .rev()
            .filter(|e| e.video.is_some() || matrix_engine::history::is_image_body(&e.text))
            .filter_map(|e| match &e.id.id {
                matrix_domain::TimelineId::Server(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// The thread an event belongs to: its root (the event itself when it is no reply), the root's
    /// row as the room timeline holds it (empty when not loaded) and the reply count.
    pub(crate) fn thread_target(&self, id: &matrix_domain::EventId) -> Option<(matrix_domain::EventId, crate::thread_view::ThreadRow, u32)> {
        let history = self.history.borrow();
        let root = history.row(id)?.thread_root.clone().unwrap_or_else(|| id.clone());
        Some(match history.row(&root) {
            Some(row) => (root, crate::thread_view::ThreadRow::from_row(row), row.thread.as_ref().map_or(0, |t| t.num_replies)),
            None => (root, crate::thread_view::ThreadRow::default(), 0),
        })
    }

    /// The thread was read in its panel: the root's counter goes now (the row changes height-free
    /// but its controls are retired, as for any same-ID change).
    pub(crate) fn thread_read(&self, root: &matrix_domain::EventId) {
        if self.history.borrow_mut().clear_thread_unread(root) {
            let (offset, count) = (self.state.logical_scroll_top(), self.history.borrow().events().len());
            self.state.splice(0..count, count);
            self.state.scroll_to(offset);
        }
    }

    pub fn first_visible_row(&self) -> usize {
        self.state.logical_scroll_top().item_ix
    }

    pub fn prepend_older(&self) {
        let offset = self.state.logical_scroll_top();
        let anchor = self
            .history
            .borrow()
            .events()
            .get(offset.item_ix)
            .map(|event| event.id.clone());
        self.history.borrow_mut().prepend_older(25);
        // Keep cached heights of unchanged rows; never reset or measure the entire list.
        self.state.splice(0..0, 25);
        if let Some(anchor) = anchor {
            let position = self
                .history
                .borrow()
                .events()
                .iter()
                .position(|event| event.id == anchor)
                .expect("prepend retains anchor");
            self.state.scroll_to(ListOffset {
                item_ix: position,
                offset_in_item: offset.offset_in_item,
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn event_stamp_probe(&self) -> impl Fn(bool) -> bool + use<> {
        let history = self.history.borrow();
        let event = history.events().first().expect("native stamp fixture");
        let id = event.id.clone();
        let body = event.text.clone();
        let copies = self.event_id_copies.clone();
        let target = copies.borrow_mut().target(&id, &body).unwrap();
        let captured = event_id_copy_callback(copies.clone(), target);
        move |reacquire| {
            if reacquire {
                let current = copies.borrow_mut().target(&id, &body).unwrap();
                event_id_copy_callback(copies.clone(), current)().is_some()
            } else {
                captured().is_some()
            }
        }
    }

    pub fn retire_offline_delivery(&self) {
        self.offline_delivery.borrow_mut().clear();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        self.offline_cursor.set(None);
    }

    pub fn retire_event_id_copies(&self) {
        self.event_id_copies.borrow_mut().clear();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            self.event_id_cursor.set(None);
            self.event_id_bounds.borrow_mut().clear();
        }
    }

    pub fn retire_selection(&self) {
        self.retire_offline_delivery();
        self.retire_event_id_copies();
        self.link_sources.borrow_mut().clear();
        self.link_cursor.set(None);
        self.pill_cursor.replace(None);
        self.requested_link.replace(None);
        self.link.borrow_mut().cancel();
        self.selection.borrow_mut().clear();
        self.dragging.set(None);
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        self.selection_cursor.replace(None);
    }

    fn retain_selection(&self) {
        let history = self.history.borrow();
        self.offline_delivery.borrow_mut().retain_current(&history);
        self.event_id_copies
            .borrow_mut()
            .retain_current(history.events().iter().map(|e| (&e.id, e.text.as_str())));
        self.link_sources
            .borrow_mut()
            .retain_current(history.events().iter().map(|e| (&e.id, e.text.as_str())));
        if self
            .requested_link
            .borrow()
            .as_ref()
            .is_some_and(|l| !self.link_sources.borrow().is_current(l))
        {
            self.requested_link.replace(None);
            self.link.borrow_mut().cancel();
        }
        self.selection
            .borrow_mut()
            .retain_current(history.events().iter().map(|e| (&e.id, e.text.as_str())));
    }

    pub fn append_live(&self) {
        let count = self.history.borrow().events().len();
        self.history.borrow_mut().append_live(1);
        self.state.splice(count..count, 1);
    }

    /// Preserve the first visible scoped server ID and pixel offset on prepend.
    pub fn prepend_server(&self, events: Vec<matrix_engine::history::HistoryEvent>) {
        if events.is_empty() {
            return;
        }
        let offset = self.state.logical_scroll_top();
        let anchor = self
            .history
            .borrow()
            .events()
            .get(offset.item_ix)
            .map(|event| event.id.clone());
        let count = self.history.borrow_mut().prepend_server(events);
        self.retain_selection();
        self.state.splice(0..0, count);
        if let Some(anchor) = anchor {
            let position = self
                .history
                .borrow()
                .events()
                .iter()
                .position(|event| event.id == anchor)
                .expect("prepend retains server anchor");
            self.state.scroll_to(ListOffset {
                item_ix: position,
                offset_in_item: offset.offset_in_item,
            });
        } else {
            self.jump_to_boundary(true);
        }
    }

    pub fn append_server(&self, events: Vec<matrix_engine::history::HistoryEvent>) {
        if events.is_empty() {
            return;
        }
        let follow = self.at_bottom();
        let offset = self.state.logical_scroll_top();
        let old = self.history.borrow().events().len();
        let changed_body = self.history.borrow_mut().refresh_server(events);
        self.retain_selection();
        let new = self.history.borrow().events().len();
        // Same-ID edits change height and retire source-bound controls too.
        if changed_body {
            self.state.splice(0..old, new);
        } else {
            self.state.splice(old..old, new - old);
        }
        if follow {
            self.jump_to_boundary(true);
        } else {
            self.state.scroll_to(offset);
        }
    }

    pub(crate) fn at_bottom(&self) -> bool {
        self.state.max_offset_for_scrollbar().height + self.state.scroll_px_offset_for_scrollbar().y
            <= px(2.)
    }

    pub fn submit_text(&self, text: String) {
        let count = self.history.borrow().events().len();
        self.history.borrow_mut().submit_text(text);
        self.state.splice(count..count, 1);
        // An explicit own submission reveals its local bubble, unlike a simulated arrival.
        self.jump_to_boundary(true);
    }

    pub fn project_delivery(&self, items: &[matrix_domain::TimelineItem], reveal: bool) {
        let offset = self.state.logical_scroll_top();
        let anchor = self
            .history
            .borrow()
            .events()
            .get(offset.item_ix)
            .map(|row| row.id.clone());
        let follow = self.at_bottom();
        let old = self.history.borrow().events().len();
        self.history.borrow_mut().project_delivery(items);
        self.retain_selection();
        let new = self.history.borrow().events().len();
        self.state.splice(0..old, new);
        if reveal || follow {
            self.jump_to_boundary(true);
        } else if let Some(index) = anchor.and_then(|id| {
            self.history
                .borrow()
                .events()
                .iter()
                .position(|row| row.id == id)
        }) {
            self.state.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: offset.offset_in_item,
            });
        }
    }

    /// Idempotent: applies «Nascondi messaggi eliminati» to the rows already loaded, keeping the scroll place.
    pub fn set_hide_redacted(&self, hide: bool) {
        let offset = self.state.logical_scroll_top();
        let old = self.history.borrow().events().len();
        if !self.history.borrow_mut().set_hide_redacted(hide) {
            return;
        }
        self.retain_selection();
        let new = self.history.borrow().events().len();
        self.state.splice(0..old, new);
        if old > 0 {
            self.state.scroll_to(offset);
        }
    }

    /// Idempotent: applies «Nascondi messaggi di stato» (this room) to the rows already loaded, keeping the scroll place.
    pub fn set_hide_status(&self, hide: bool) {
        let offset = self.state.logical_scroll_top();
        let old = self.history.borrow().events().len();
        if !self.history.borrow_mut().set_hide_status(hide) {
            return;
        }
        self.retain_selection();
        let new = self.history.borrow().events().len();
        self.state.splice(0..old, new);
        if old > 0 {
            self.state.scroll_to(offset);
        }
    }

    pub fn project_operations(&self, actions: &matrix_desktop::LiveOperations) {
        // Room-state diffs (including Space links) reapply the projection. A
        // whole-list splice otherwise resets the reader to row zero even when
        // the selected conversation and every message are unchanged.
        let offset = self.state.logical_scroll_top();
        let old = self.history.borrow().events().len();
        self.history
            .borrow_mut()
            .project_operations(actions.confirmed());
        self.retain_selection();
        let new = self.history.borrow().events().len();
        self.state.splice(0..old, new);
        if old > 0 {
            self.state.scroll_to(offset);
        }
    }

    pub fn render(
        &self,
        t: DesignTokens,
        window: &gpui::Window,
        delivery: Option<Rc<RefCell<matrix_desktop::LiveDelivery>>>,
        operations: Option<Rc<RefCell<matrix_desktop::LiveOperations>>>,
    ) -> impl IntoElement {
        let history = self.history.clone();
        let live = self.live;
        let probe = self.probe;
        let members = self.member_count.get();
        let peer_receipts = self.peer_receipts.clone();
        let debug = self.debug.get();
        let constructed = Rc::new(RefCell::new(Vec::<usize>::new()));
        let layouts = Rc::new(RefCell::new(Vec::<RowLayout>::new()));
        let row_indices = constructed.clone();
        let row_layouts = layouts.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let painted: SemanticBounds = Rc::default();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let row_painted = painted.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let retry_painted: SemanticBounds = Rc::default();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let row_retry_painted = retry_painted.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let actions_painted: SemanticBounds = Rc::default();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let row_actions_painted = actions_painted.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let selection_painted = Rc::new(RefCell::new(std::collections::HashMap::new()));
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let row_selection_painted = selection_painted.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let selection_cursor = self.selection_cursor.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_selection = self
            .focus
            .is_focused(window)
            .then(|| self.selection_cursor.borrow().clone())
            .flatten();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_actions = self
            .focus
            .is_focused(window)
            .then(|| self.actions_cursor.borrow().clone())
            .flatten();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_poll = self
            .focus
            .is_focused(window)
            .then(|| self.poll_cursor.borrow().clone())
            .flatten();
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused_poll: Option<(ScopedTimelineId, usize)> = None;
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_retry = self
            .focus
            .is_focused(window)
            .then(|| self.retry_cursor.borrow().clone())
            .flatten();
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let _ = window;
        let state = self.state.clone();
        let retained = history.borrow().events().len();
        let row_extent = self.extent.clone();
        let delivery_state = self.state.clone();
        let offline_delivery = self.offline_delivery.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_offline = self
            .focus
            .is_focused(window)
            .then(|| self.offline_cursor.get())
            .flatten();
        let selection = self.selection.clone();
        let search_mark = self.search_mark.clone();
        let focus = self.focus.clone();
        let dragging = self.dragging.clone();
        let released = self.dragging.clone();
        let released_out = self.dragging.clone();
        let link = self.link.clone();
        let click_origin = self.click_origin.clone();
        let pending = self.link.borrow().capture();
        let menu_target = self.link_menu.borrow().target().cloned();
        let link_sources = self.link_sources.clone();
        let link_cursor = self.link_cursor.clone();
        let pill_cursor = self.pill_cursor.clone();
        let self_pill_bounds = self.pill_bounds.clone();
        let link_hover = self.link_hover.clone();
        let link_menu = self.link_menu.clone();
        let link_menu_at = self.link_menu_at.clone();
        let images = self.images.clone();
        let sender_pictures = self.sender_pictures.clone();
        let video_painted = self.video_painted.clone();
        let poll_painted = self.poll_painted.clone();
        let viewing = self.viewing.clone();
        let (original, want_original) = (self.original.clone(), self.want_original.clone());
        let want_video = self.want_video.clone();
        let want_thread = self.want_thread.clone();
        let want_profile = self.want_profile.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let profile_painted = self.profile_painted.clone();
        let (saves, want_save, stop_save) = (self.saves.clone(), self.want_save.clone(), self.stop_save.clone());
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let save_painted = self.save_painted.clone();
        let requested_link = self.requested_link.clone();
        let link_painted = Rc::new(RefCell::new(std::collections::HashMap::new()));
        let row_link_painted = link_painted.clone();
        let published_links = self.link_bounds.clone();
        let event_id_copies = self.event_id_copies.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let event_id_painted = Rc::new(RefCell::new(std::collections::HashMap::new()));
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let row_event_id_painted = event_id_painted.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let published_event_ids = self.event_id_bounds.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_event_id = self
            .focus
            .is_focused(window)
            .then(|| self.event_id_cursor.get())
            .flatten();
        div()
            .id("timeline-text")
            .track_focus(&self.focus)
            .on_key_down(self.selection_key_callback(cfg!(target_os = "macos")))
            .on_key_down({
                let (viewing, focus) = (self.viewing.clone(), self.focus.clone());
                move |event, window, cx| {
                    if close_viewer(&viewing, &focus, &event.keystroke.key, window) {
                        window.refresh();
                        cx.stop_propagation();
                    }
                }
            })
            .when(cfg!(any(target_os = "macos", test)), |element| {
                let key = self.selection_key_callback(true);
                let focus = self.focus.clone();
                crate::editing_command::with_actions(element, false, move |event, window, cx| {
                    if focus.is_focused(window) {
                        key(event, window, cx);
                    }
                })
            })
            .on_mouse_up(MouseButton::Left, move |_, _, _| released.set(None))
            .on_mouse_up_out(MouseButton::Left, move |_, _, _| released_out.set(None))
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(t.timeline))
            .child(
                list(self.state.clone(), move |index, _, _| {
                    let row_extent = row_extent.clone();
                    let delivery = delivery.clone();
                    let operations = operations.clone();
                    let delivery_history = history.clone();
                    let delivery_state = delivery_state.clone();
                    let history = history.borrow();
                    let event = &history.events()[index];
                    if event.status {
                        // Quiet centred line like Element: small round avatar + grey text, no bubble.
                        let avatar = event.sender.as_ref().map(|s| {
                            let initials = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                            match s.avatar.as_deref().and_then(|m| sender_pictures.borrow().get(m).cloned()) {
                                Some(image) => crate::avatar_view::round(image, STATUS_AVATAR_PX),
                                None => div().flex().items_center().justify_center().size(px(STATUS_AVATAR_PX)).flex_shrink_0().rounded_full().bg(rgb(matrix_desktop::AVATAR_FALLBACK)).text_color(rgb(t.avatar_text)).text_size(px(7.)).font_weight(gpui::FontWeight::SEMIBOLD).child(initials).into_any_element(),
                            }
                        });
                        return div().id(gpui::SharedString::from(format!("status-{index}"))).w_full().px_4().py(px(1.)).flex().justify_center().items_center().gap_1p5().overflow_hidden()
                            .text_size(px(11.)).text_color(rgb(t.secondary))
                            // min_w_0 + text_center: long bridge texts wrap inside the column instead of overflowing it.
                            .children(avatar).child(div().min_w_0().text_center().child(matrix_desktop::row_text::status_with_time(&event.text, &event.time))).into_any_element();
                    }
                    let sender_label = history.sender_label(index, members);
                    let sender_user = event.sender.as_ref().map(|s| s.id.clone());
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    let profile_bounds = { let (map, id) = (profile_painted.clone(), event.id.clone()); move |b: Bounds<Pixels>| { map.borrow_mut().insert(id.clone(), b); } };
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    let profile_bounds = |_: Bounds<Pixels>| ();
                    let sender_avatar = history.sender_avatar(index, members).map(|(mxc, initials)| {
                        let avatar = match mxc.and_then(|m| sender_pictures.borrow().get(m).cloned()) {
                            Some(image) => crate::avatar_view::round(image, SENDER_AVATAR_PX),
                            None => div().flex().items_center().justify_center().size(px(SENDER_AVATAR_PX)).flex_shrink_0().rounded_full().bg(rgb(matrix_desktop::AVATAR_FALLBACK)).text_color(rgb(t.avatar_text)).text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).child(initials).into_any_element(),
                        };
                        match &sender_user {
                            Some(user) => profile_target(div().id(("sender-avatar", index)).flex_shrink_0(), user.clone(), want_profile.clone(), profile_bounds.clone(), avatar).into_any_element(),
                            None => avatar,
                        }
                    });
                    // Groups only: the room header already shows the one other person of a chat.
                    let avatar_gutter = members > 2 && !event.outgoing && event.sender.is_some();
                    let offline_target = (!live).then(|| offline_delivery.borrow_mut().target(event)).flatten();
                    let offline_callback = offline_target.clone().map(|target| offline_delivery_callback(offline_delivery.clone(), delivery_history.clone(), target, live));
                    let video_picture = event.video.as_ref().map(|facts| {
                        let state = match &event.id.id {
                            matrix_domain::TimelineId::Server(id) => images.borrow().get(id).cloned(),
                            _ => None,
                        };
                        let (parts, id) = (video_painted.clone(), event.id.clone());
                        let picture = crate::media_view::video_picture(state, facts.duration.clone(), facts.size.clone(), t, std::rc::Rc::new(move |part, bounds| { parts.borrow_mut().insert((id.id.clone(), part), bounds); }));
                        match &event.id.id {
                            matrix_domain::TimelineId::Server(id) => {
                                let (id, viewing, focus) = (id.clone(), viewing.clone(), focus.clone());
                                let (original, want_video) = (original.clone(), want_video.clone());
                                div().id(gpui::SharedString::from(id.as_str().to_owned())).cursor_pointer()
                                    .on_click(move |_, window, _| {
                                        *viewing.borrow_mut() = Some(id.clone());
                                        *original.borrow_mut() = None;
                                        *want_video.borrow_mut() = Some(id.clone());
                                        focus.focus(window);
                                        window.refresh();
                                    })
                                    .child(picture).into_any_element()
                            }
                            _ => picture,
                        }
                    });
                    let poll_bubble = event.poll.as_ref().map(|poll| {
                        let (parts, id) = (poll_painted.clone(), event.id.id.clone());
                        let target = match &event.id.id { matrix_domain::TimelineId::Server(e) => Some(e.clone()), _ => None };
                        let failed = target.as_ref().zip(operations.as_ref()).is_some_and(|(e, o)| o.borrow().vote_failed(e));
                        let focus_ix = focused_poll.as_ref().filter(|(m, _)| *m == event.id).map(|(_, ix)| *ix);
                        let on_vote: crate::poll_view::PollVote = match (target, operations.clone()) {
                            #[cfg(feature = "live")]
                            (Some(target), Some(ops)) => {
                                let (scope, poll) = (event.id.scope.clone(), poll.clone());
                                Rc::new(move |ix, window, _| {
                                    ops.borrow_mut().vote(scope.clone(), target.clone(), &poll, ix, matrix_engine::sending::fresh_transaction_id());
                                    window.refresh();
                                })
                            }
                            _ => Rc::new(|_, _, _| {}),
                        };
                        crate::poll_view::poll_bubble(poll, t, index, focus_ix, failed, on_vote, Rc::new(move |part, bounds| { parts.borrow_mut().insert((id.clone(), part), bounds); }))
                    });
                    let picture = poll_bubble.or(video_picture).or_else(|| matrix_engine::history::is_image_body(&event.text).then(|| {
                        let state = match &event.id.id {
                            matrix_domain::TimelineId::Server(id) => images.borrow().get(id).cloned(),
                            _ => None,
                        };
                        let opened = matches!(state, Some(crate::media_view::InlineImage::Ready(_)));
                        let picture = crate::media_view::inline_picture(state, t);
                        match (&event.id.id, opened) {
                            (matrix_domain::TimelineId::Server(id), true) => {
                                let (id, viewing, focus) = (id.clone(), viewing.clone(), focus.clone());
                                let (original, want_original) = (original.clone(), want_original.clone());
                                div().id(gpui::SharedString::from(id.as_str().to_owned())).cursor_pointer()
                                    .on_click(move |_, window, _| {
                                        *viewing.borrow_mut() = Some(id.clone());
                                        *original.borrow_mut() = None;
                                        *want_original.borrow_mut() = Some(id.clone());
                                        focus.focus(window);
                                        window.refresh();
                                    })
                                    .child(picture).into_any_element()
                            }
                            _ => picture,
                        }
                    }));
                    let (save_icon, save_failure) = match &event.id.id {
                        matrix_domain::TimelineId::Server(id) if matrix_desktop::offers_save(event, live) => {
                            let state = saves.borrow().state(id);
                            let (sid, name) = (id.clone(), event.attachment_name.clone().unwrap_or_default());
                            let (saves, want_save, stop_save) = (saves.clone(), want_save.clone(), stop_save.clone());
                            let click: crate::media_view::SaveClick = Rc::new(move |window, _| {
                                let running = saves.borrow().state(&sid) == Some(matrix_desktop::SaveState::Running);
                                if running {
                                    stop_save.set(true);
                                    saves.borrow_mut().cancel(&sid);
                                } else {
                                    saves.borrow_mut().begin(&sid);
                                    *want_save.borrow_mut() = Some((sid.clone(), name.clone()));
                                }
                                window.refresh();
                            });
                            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                            let painted = { let (map, id) = (save_painted.clone(), id.clone()); move |bounds| { map.borrow_mut().insert(id.clone(), bounds); } };
                            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                            let painted = |_| ();
                            (
                                Some(crate::media_view::save_icon(index, state == Some(matrix_desktop::SaveState::Running), t, click.clone(), painted)),
                                match state {
                                    Some(matrix_desktop::SaveState::Failed(error)) => Some(crate::media_view::save_failure(index, error, t, click)),
                                    _ => None,
                                },
                            )
                        }
                        _ => (None, None),
                    };
                    let (gaps, drawn) = matrix_desktop::mention_gaps::Gaps::new(&event.text, &event.mentions);
                    let gaps = Rc::new(gaps);
                    let mut body = StyledText::new(drawn);
                    let pills: Vec<_> = matrix_desktop::mention_gaps::fitting(&event.text, &event.mentions).collect();
                    let pill_paint: Vec<_> = pills.iter().map(|m| {
                        use matrix_engine::rich_text::MentionKind;
                        let (bg, picture, user, uid) = match &m.kind {
                            MentionKind::User { sender, me } => (if *me { t.error } else { t.border }, sender.avatar.as_deref().and_then(|mxc| sender_pictures.borrow().get(mxc).cloned()), true, Some(sender.id.clone())),
                            MentionKind::AtRoom => (t.error, None, false, None),
                            MentionKind::Room => (t.border, None, false, None),
                        };
                        let ring = uid.is_some() && pill_cursor.borrow().as_ref() == Some(&(event.id.clone(), m.range.start));
                        (m.range.clone(), bg, picture, user, uid, ring)
                    }).collect();
                    let links = message_links(&event.text);
                    let targets = link_sources.borrow_mut().targets(&event.id, &event.text);
                    let selected = selection.borrow().range(&event.id, &event.text);
                    let mark = search_mark.borrow().as_ref().filter(|m| matches!(&event.id.id, matrix_domain::TimelineId::Server(id) if *id == m.event)).cloned();
                    let found = mark.as_ref().map(|m| matrix_engine::message_match::find_ranges(&event.text, &m.term)).unwrap_or_default();
                    let flashing = mark.as_ref().is_some_and(|m| m.flash_until > std::time::Instant::now());
                    let mut cuts = vec![0, event.text.len()];
                    for r in &found { cuts.extend([r.start, r.end]); }
                    // Only spans that fit the text: a stale range must never slice inside a character.
                    let styles: Vec<_> = event.styles.iter().filter(|s| s.range.start < s.range.end && s.range.end <= event.text.len() && event.text.is_char_boundary(s.range.start) && event.text.is_char_boundary(s.range.end)).collect();
                    for style in &styles { cuts.extend([style.range.start, style.range.end]); }
                    for link in &links { cuts.extend([link.range.start, link.range.end]); }
                    for pill in &pills { cuts.extend([pill.range.start, pill.range.end]); }
                    if let Some(range) = &selected { cuts.extend([range.start, range.end]); }
                    cuts.sort_unstable();
                    cuts.dedup();
                    body = body.with_highlights(cuts.windows(2).map(|pair| {
                        let range = pair[0]..pair[1];
                        let is_link = links.iter().any(|link| link.range.contains(&range.start));
                        let pill = pill_paint.iter().find(|(r, ..)| r.contains(&range.start));
                        use matrix_engine::rich_text::Style;
                        let has = |kind| styles.iter().any(|s| s.style == kind && s.range.contains(&range.start));
                        let style = HighlightStyle {
                            font_weight: has(Style::Bold).then_some(gpui::FontWeight::BOLD),
                            font_style: has(Style::Italic).then_some(gpui::FontStyle::Italic),
                            strikethrough: has(Style::Strike).then(|| gpui::StrikethroughStyle { thickness: px(1.), color: None }),
                            color: if let Some((_, bg, ..)) = pill { Some(rgb(if *bg == t.error { t.surface } else { t.text }).into()) } else if is_link { Some(rgb(t.accent).into()) } else { has(Style::Quote).then(|| rgb(t.secondary).into()) },
                            underline: is_link.then(|| gpui::UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                            background_color: if found.iter().any(|r| r.contains(&range.start)) || selected.as_ref().is_some_and(|r| r.contains(&range.start))
                                || targets.iter().any(|l| link_cursor.get() == Some(l.instance()) && l.range().contains(&range.start)) { Some(rgb(t.selection).into()) } else { has(Style::Code).then(|| rgb(t.input).into()) },
                            ..Default::default()
                        };
                        (gaps.start(range.start)..gaps.end(range.end), style)
                    }));
                    let down_layout = PillLayout { layout: body.layout().clone(), gaps: gaps.clone() };
                    let move_layout = PillLayout { layout: body.layout().clone(), gaps: gaps.clone() };
                    let down_generation = selection.borrow().generation();
                    let down_selection = selection.clone();
                    let move_selection = selection.clone();
                    let delivery_id = event.id.clone();
                    let transaction_id = event.transaction_id.clone();
                    let down_id = event.id.clone();
                    let move_id = event.id.clone();
                    let down_text = event.text.clone();
                    let move_text = event.text.clone();
                    let down_focus = focus.clone();
                    let down_drag = dragging.clone();
                    let move_drag = dragging.clone();
                    let up_layout = PillLayout { layout: body.layout().clone(), gaps: gaps.clone() };
                    let up_selection = selection.clone();
                    let up_id = event.id.clone();
                    let up_text = event.text.clone();
                    let down_origin = click_origin.clone();
                    let up_origin = click_origin.clone();
                    let down_link = link.clone();
                    let up_link = link.clone();
                    let down_sources = link_sources.clone();
                    let up_sources = link_sources.clone();
                    let up_targets = targets.clone();
                    let move_targets = targets.clone();
                    let move_hover = link_hover.clone();
                    let hovered_url = targets.iter().find(|target| link_hover.get() == Some(target.instance())).map(|target| target.url().as_str().to_owned());
                    let (tip_surface, tip_border, tip_text) = (t.surface, t.border, t.text);
                    let down_cursor = link_cursor.clone();
                    let down_menu = link_menu.clone();
                    let menu_layout = PillLayout { layout: body.layout().clone(), gaps: gaps.clone() };
                    let menu_targets = targets.clone();
                    let menu_state = link_menu.clone();
                    let menu_at = link_menu_at.clone();
                    let paint_targets = targets.clone();
                    let paint_layout = PillLayout { layout: body.layout().clone(), gaps: gaps.clone() };
                    let paint_links = row_link_painted.clone();
                    let pill_layout = paint_layout.clone();
                    let (pill_bounds, pill_event, accent) = (self_pill_bounds.clone(), event.id.clone(), t.accent);
                    let up_pills: Vec<_> = pill_paint.iter().filter_map(|(r, _, _, _, uid, _)| uid.clone().map(|u| (r.clone(), u))).collect();
                    let up_want = want_profile.clone();
                    let down_requested = requested_link.clone();
                    let up_requested = requested_link.clone();
                    let status = match event.send_state {
                        Some(DemoSendState::Pending) => "pending",
                        Some(DemoSendState::Failed) => "failed",
                        None => if live { "server" } else { "fixture" },
                    };
                    let stamp = StyledText::new(if live || event.send_state.is_some() {
                        // Only a 1:1 shows "read": in a group the check would claim too much.
                        let read = members == 2
                            && history.peer_read_index(&peer_receipts.borrow()).is_some_and(|i| index <= i);
                        event.stamp_read(read)
                    } else {
                        format!("{} · demo locale", event.time)
                    });
                    let row_bounds: RowBounds = Rc::new(RefCell::new(None));
                    if probe {
                        row_indices.borrow_mut().push(index);
                        row_layouts.borrow_mut().push((index, body.layout().clone(), stamp.layout().clone(), event.text.clone(), row_bounds.clone(), status, !event.annotation.is_empty()));
                    }
                    div()
                        .relative()
                        .w_full()
                        .px_6()
                        .py_2()
                        .flex()
                        .when(event.outgoing, |d| d.justify_end())
                        .when(avatar_gutter, |d| d.gap_2().items_start().child(match sender_avatar {
                            Some(avatar) => avatar,
                            None => div().size(px(SENDER_AVATAR_PX)).flex_shrink_0().into_any_element(),
                        }))
                        .child(
                            div()
                                .w(relative(0.85))
                                .max_w(px(520.))
                                .min_w_0()
                                .px_4()
                                .py_3()
                                .rounded_lg()
                                .relative()
                                .bg(rgb(if flashing { flash(if event.outgoing { t.outgoing } else if event.highlighted { t.mention_row } else { t.surface }, t.accent) } else if event.outgoing { t.outgoing } else if event.highlighted { t.mention_row } else { t.surface }))
                                .when(event.highlighted, |d| d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).rounded_l_lg().bg(rgb(t.error))))
                                .when(event.menu_available(live), |d| {
                                    let id = event.id.clone();
                                    let operations = operations.clone();
                                    let d = d.on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                        if let Some(actions) = &operations { actions.borrow_mut().select(id.clone()); window.refresh(); cx.stop_propagation(); }
                                    });
                                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                    {
                                        let painted = row_actions_painted.clone();
                                        let id = event.id.clone();
                                        d.when(focused_actions.as_ref() == Some(&id), |d| d.border_2().border_color(rgb(t.accent)))
                                            .child(canvas(|_,_,_| (), move |bounds,_,_,_| { painted.borrow_mut().insert(id.clone(),bounds); }).absolute().inset_0())
                                    }
                                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                    { d }
                                })
                                .when_some(sender_label, |d, name| d.child(div().flex().mb_1().child({
                                    let base = div().id(("sender-name", index)).text_size(px(12.)).line_height(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.accent));
                                    match &sender_user {
                                        Some(user) => profile_target(base, user.clone(), want_profile.clone(), profile_bounds.clone(), name),
                                        None => base.child(name),
                                    }
                                })))
                                .child(div().id(("message-text", index))
                                    .cursor(if hovered_url.is_some() { gpui::CursorStyle::PointingHand } else { gpui::CursorStyle::IBeam })
                                    .when_some(hovered_url, |d, url| d.tooltip(move |_, cx| cx.new(|_| LinkTooltip { url: url.clone(), surface: tip_surface, border: tip_border, text: tip_text }).into()))
                                    .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                                        let ix = down_layout.index_for_position(e.position).unwrap_or_else(|ix| ix);
                                        if !down_selection.borrow_mut().begin_current(down_generation, down_id.clone(), &down_text, ix) { return; }
                                        down_sources.borrow_mut().retain_event(&down_id);
                                        down_cursor.set(None);
                                        down_menu.borrow_mut().close();
                                        down_requested.replace(None);
                                        *down_origin.borrow_mut() = Some((down_id.clone(), ix));
                                        down_link.borrow_mut().cancel();
                                        down_drag.set(Some(down_selection.borrow().generation()));
                                        down_focus.focus(window);
                                        window.refresh();
                                        cx.stop_propagation();
                                    })
                                    .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                                        if let Ok(ix) = menu_layout.index_for_position(e.position)
                                            && let Some(target) = matrix_desktop::link_at(&menu_targets, ix) {
                                            menu_state.borrow_mut().open(target.clone());
                                            menu_at.set(e.position);
                                            window.refresh();
                                            cx.stop_propagation();
                                        }
                                    })
                                    .on_mouse_up(MouseButton::Left, move |e, window, _| {
                                        let origin = up_origin.borrow_mut().take();
                                        if up_selection.borrow().range(&up_id, &up_text).is_none()
                                            && let Some((id, start)) = origin
                                            && id == up_id
                                            && up_layout.index_for_position(e.position) == Ok(start) {
                                            // A user pill opens the profile drawer; a link asks for confirmation.
                                            if let Some((_, user)) = up_pills.iter().find(|(r, _)| r.contains(&start)) {
                                                *up_want.borrow_mut() = Some(user.clone());
                                                window.refresh();
                                            } else if let Some(target) = up_targets.iter().find(|target| target.range().contains(&start))
                                                && up_sources.borrow_mut().request(target, &mut up_link.borrow_mut()) {
                                                up_requested.replace(Some(target.clone()));
                                                window.refresh();
                                            }
                                        }
                                    })
                                    .relative()
                                    .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                                        // Pills first, behind the text: a rounded fill per line and the avatar in the gap.
                                        let lh = pill_layout.line_height();
                                        for (range, bg, picture, user, uid, ring) in &pill_paint {
                                            let from = if *user { pill_layout.layout.position_for_index(pill_layout.gaps.gap_start(range.start)) } else { pill_layout.position_for_index(range.start) };
                                            let (Some(a), Some(b)) = (from, pill_layout.layout.position_for_index(pill_layout.gaps.end(range.end))) else { continue };
                                            let left = a.x - if *user { px(2.) } else { px(3.) };
                                            let lines = if b.y == a.y { vec![(a.y, left, b.x + px(3.))] } else { vec![(a.y, left, bounds.right()), (b.y, bounds.left(), b.x + px(3.))] };
                                            for (y, l, r) in lines {
                                                let mut quad = gpui::fill(Bounds::from_corners(gpui::point(l, y + px(1.)), gpui::point(r, y + lh - px(1.))), rgb(*bg));
                                                quad.corner_radii = ((lh - px(2.)) / 2.).into();
                                                if *ring { quad.border_widths = px(2.).into(); quad.border_color = rgb(accent).into(); }
                                                window.paint_quad(quad);
                                            }
                                            if uid.is_some() {
                                                let right = if b.y == a.y { b.x } else { bounds.right() };
                                                pill_bounds.borrow_mut().insert((pill_event.clone(), range.start), Bounds::from_corners(a, gpui::point(right, a.y + lh)));
                                            }
                                            if *user {
                                                let icon = Bounds::new(gpui::point(a.x, a.y + (lh - px(16.)) / 2.), gpui::size(px(16.), px(16.)));
                                                match picture {
                                                    Some(image) => { let _ = window.paint_image(icon, gpui::Corners::all(px(8.)), image.clone(), 0, false); }
                                                    None => { let mut quad = gpui::fill(icon, rgb(matrix_desktop::AVATAR_FALLBACK)); quad.corner_radii = px(8.).into(); window.paint_quad(quad); }
                                                }
                                            }
                                        }
                                        // Accessible link bounds come from the text itself: first line only when a link wraps.
                                        for target in &paint_targets {
                                            let r = target.range();
                                            if let (Some(a), Some(b)) = (paint_layout.position_for_index(r.start), paint_layout.position_for_index(r.end)) {
                                                let lh = paint_layout.line_height();
                                                let right = if b.y == a.y { b.x } else { bounds.right() };
                                                paint_links.borrow_mut().insert(target.instance(), Bounds::from_corners(a, gpui::point(right, a.y + lh)));
                                            }
                                        }
                                        let layout = move_layout.clone();
                                        let selection = move_selection.clone();
                                        let id = move_id.clone();
                                        let text = move_text.clone();
                                        let dragging = move_drag.clone();
                                        let targets = move_targets.clone();
                                        let hover = move_hover.clone();
                                        let hover_layout = move_layout.clone();
                                        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, window, _| {
                                            if phase == gpui::DispatchPhase::Bubble {
                                                let over = hover_layout.index_for_position(e.position).ok().and_then(|ix| matrix_desktop::link_at(&targets, ix)).map(|t| t.instance());
                                                if over != hover.get() && (over.is_some() || targets.iter().any(|t| hover.get() == Some(t.instance()))) {
                                                    hover.set(over);
                                                    window.refresh();
                                                }
                                            }
                                            if phase == gpui::DispatchPhase::Bubble && e.pressed_button == Some(MouseButton::Left) && let Some(generation) = dragging.get() {
                                                let ix = layout.index_for_position(e.position).unwrap_or_else(|ix| ix);
                                                if selection.borrow().generation() == generation {
                                                    selection.borrow_mut().extend_current(generation, &id, &text, ix);
                                                    dragging.set(Some(selection.borrow().generation()));
                                                }
                                                window.refresh();
                                            }
                                        });
                                    }).absolute().inset_0())
                                    .text_size(px(14.)).line_height(px(20.)).whitespace_normal().child(match picture {
                                        // The mouse handlers read the text layout, which only exists once the text was painted: keep it painted, invisible.
                                        Some(picture) => div().child(picture).child(div().size_0().overflow_hidden().child(body)).into_any_element(),
                                        None => body.into_any_element(),
                                    }))
                                .when_some(event.thread_label(), |d, (label, initials, mxc)| d.child(
                                    div().id(("thread-summary", index)).mt_2().flex().items_center().gap_2().cursor_pointer()
                                        .on_click({
                                            let (want_thread, id) = (want_thread.clone(), event.id.id.clone());
                                            move |_, window, _| {
                                                if let matrix_domain::TimelineId::Server(id) = &id { *want_thread.borrow_mut() = Some(id.clone()); window.refresh(); }
                                            }
                                        })
                                        .child(match mxc.and_then(|m| sender_pictures.borrow().get(m).cloned()) {
                                            Some(image) => crate::avatar_view::round(image, 20.),
                                            None => div().flex().items_center().justify_center().size(px(20.)).flex_shrink_0().rounded_full().bg(rgb(matrix_desktop::AVATAR_FALLBACK)).text_color(rgb(t.avatar_text)).text_size(px(9.)).font_weight(gpui::FontWeight::SEMIBOLD).child(initials).into_any_element(),
                                        })
                                        .child(div().text_size(px(12.)).line_height(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.accent)).child(label))))
                                .child(div().w_full().flex().justify_end().items_center().gap_1().mt_1()
                                    .children(save_icon)
                                    .child(
                                    div()
                                        .text_right()
                                        .whitespace_normal()
                                        .text_size(px(10.))
                                        .line_height(px(14.))
                                        .text_color(rgb(if event.send_state == Some(DemoSendState::Failed) { t.error } else { t.secondary }))
                                        .id(("event-identity", index))
                                        .when_some(live.then(|| event_id_copies.borrow_mut().target(&event.id, &event.text)).flatten(), |d, target| {
                                            let copy = event_id_copy_callback(event_id_copies.clone(), target.clone());
                                            d.cursor_pointer().map(|d| {
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                {
                                                    let painted = row_event_id_painted.clone();
                                                    let instance = target.instance();
                                                    d.relative().border_1().border_color(rgb(if focused_event_id == Some(instance) { t.accent } else if event.outgoing { t.outgoing } else { t.surface }))
                                                        .child(canvas(|_,_,_| (), move |bounds,_,_,_| { painted.borrow_mut().insert(instance,bounds); }).absolute().inset_0())
                                                }
                                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                                { d }
                                            }).on_click(move |_, _, cx| {
                                                if let Some(id) = copy() {
                                                    cx.write_to_clipboard(ClipboardItem::new_string(id));
                                                }
                                            })
                                        })
                                        .child(stamp),
                                ))
                                .children(save_failure)
                                .when_some((debug && live).then(|| event.debug_id().map(str::to_owned)).flatten(), |d, id| {
                                    let copied = id.clone();
                                    d.child(div().id(("event-debug-id", index)).w_full().text_right().text_size(px(10.)).line_height(px(14.))
                                        .text_color(rgb(t.secondary)).cursor_pointer().child(id)
                                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))))
                                })
                                .map(|d| {
                                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                    {
                                        use matrix_desktop::accessibility::selection::Control;
                                        let mut controls = div().flex().justify_end().gap_2().h(px(24.)).items_center();
                                        for control in Control::ALL {
                                            if control != Control::Select && selected.is_none() { continue; }
                                            let painted = row_selection_painted.clone();
                                            let id = event.id.clone();
                                            let painted_id = id.clone();
                                            let text = event.text.clone();
                                            let generation = selection.borrow().generation();
                                            let selected = selection.clone();
                                            let cursor = selection_cursor.clone();
                                            let focus = focus.clone();
                                            let focused = focused_selection.as_ref() == Some(&(id.clone(),control));
                                            controls = controls.child(div().id(("selection-control", index*3+control as usize))
                                                .relative().border_1().border_color(rgb(if focused { t.accent } else if event.outgoing { t.outgoing } else { t.surface }))
                                                .px_1().text_size(px(12.)).text_color(rgb(t.accent)).cursor_pointer()
                                                .child(match control { Control::Select => "Seleziona", Control::Copy => "Copia", Control::Cancel => "Annulla" })
                                                .child(canvas(|_,_,_| (), move |bounds,_,_,_| { painted.borrow_mut().insert((painted_id.clone(),control),bounds); }).absolute().inset_0())
                                                .on_click(move |_,window,cx| {
                                                    if selected.borrow().generation() != generation { return; }
                                                    focus.focus(window);
                                                    let next = match control {
                                                        Control::Select => { selected.borrow_mut().begin(id.clone(),&text,0); selected.borrow_mut().key(&id,&text,"select-all",false); Some((id.clone(),Control::Copy)) }
                                                        Control::Copy => { if let Some(text) = selected.borrow().copy(&id,&text) { cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned())); } Some((id.clone(),Control::Copy)) }
                                                        Control::Cancel => { selected.borrow_mut().clear(); None }
                                                    };
                                                    cursor.replace(next); window.refresh();
                                                }));
                                        }
                                        d.child(controls)
                                    }
                                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                    { d }
                                })
                                .when(!event.annotation.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(rgb(t.accent)).child(event.annotation.clone())))
                                .when_some(event.send_state.filter(|s| !live || *s == DemoSendState::Failed), |d, status| {
                                    let (cancel_delivery, cancel_history, cancel_state, cancel_id, cancel_tx) = (delivery.clone(), delivery_history.clone(), delivery_state.clone(), delivery_id.clone(), transaction_id.clone());
                                    let refused = live.then(|| delivery.as_ref().zip(transaction_id.as_ref())
                                        .filter(|(delivery, tx)| delivery.borrow().is_final_failure(&delivery_id.scope, tx))
                                        .and_then(|(delivery, tx)| delivery.borrow().failure_label(&delivery_id.scope, tx))).flatten();
                                    d.child(
                                    div().flex().justify_end().gap_3().h(px(24.)).items_center()
                                        .when_some(refused, |d, label| d.child(div().id(("delivery-refused", index)).text_size(px(12.)).text_color(rgb(t.accent)).child(label)))
                                        .child(div().id(("demo-delivery", index)).when(refused.is_some(), |d| d.hidden())
                                            .map(|d| {
                                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                                {
                                                    let painted = row_retry_painted.clone();
                                                    let id = event.id.clone();
                                                    d.relative().border_1().border_color(rgb(if focused_retry.as_ref() == Some(&id) || offline_target.as_ref().is_some_and(|s| focused_offline == Some(s.instance())) { t.accent } else if event.outgoing { t.outgoing } else { t.surface }))
                                                        .child(canvas(|_,_,_| (), move |bounds,_,_,_| { painted.borrow_mut().insert(id.clone(),bounds); }).absolute().inset_0())
                                                }
                                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                                { d }
                                            })
                                            .cursor_pointer().text_size(px(12.)).text_color(rgb(t.accent))
                                            .child(if live { "Riprova" } else if status == DemoSendState::Pending { "Simula errore" } else { "Riprova (demo)" })
                                            .on_click(move |_, window, _| {
                                                if live {
                                                    if let (Some(delivery), Some(tx)) = (&delivery, &transaction_id) {
                                                        let mut delivery = delivery.borrow_mut();
                                                        if delivery.retry(&delivery_id.scope, tx) {
                                                            delivery_history.borrow_mut().project_delivery(delivery.items(&delivery_id.scope));
                                                            delivery_state.splice(index..index + 1, 1);
                                                            window.refresh();
                                                        }
                                                    }
                                                    return;
                                                }
                                                if let Some(index) = offline_callback.as_ref().and_then(|callback| callback()) {
                                                    delivery_state.splice(index..index + 1, 1);
                                                    window.refresh();
                                                }
                                            }))
                                        .when(live, |d| d.child(div().id(("demo-cancel", index)).cursor_pointer().text_size(px(12.)).text_color(rgb(t.accent)).child("Annulla")
                                            .on_click(move |_, window, _| {
                                                if let (Some(delivery), Some(tx)) = (&cancel_delivery, &cancel_tx) {
                                                    let mut delivery = delivery.borrow_mut();
                                                    if delivery.cancel(&cancel_id.scope, tx) {
                                                        cancel_history.borrow_mut().project_delivery(delivery.items(&cancel_id.scope));
                                                        cancel_state.splice(index..index + 1, 1);
                                                        window.refresh();
                                                    }
                                                }
                                            })))
                                    )}),
                        )
                        .child(canvas(move |bounds, _, _| {
                            *row_bounds.borrow_mut() = Some(bounds);
                            let width = f32::from(bounds.size.width);
                            let mut extent = row_extent.borrow_mut();
                            if extent.0 != retained || extent.1 != width {
                                *extent = (retained, width, matrix_desktop::VirtualScrollExtent::new(retained));
                            }
                            extent.2.measure(index, f32::from(bounds.size.height));
                        }, |_, _, _, _| {}).absolute().inset_0())
                        .map(|d| {
                            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                            {
                                let painted = row_painted.clone();
                                let id = event.id.clone();
                                d.child(canvas(|_, _, _| (), move |bounds, _, _, _| {
                                    painted.borrow_mut().insert(id.clone(), bounds);
                                }).absolute().inset_0())
                            }
                            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                            { d }
                        })
                        .into_any()
                })
                .size_full(),
            )
            .children(self.viewing().and_then(|id| {
                let is_video = self.video_events().contains(&id);
                let thumbnail = match self.images.borrow().get(&id) {
                    Some(crate::media_view::InlineImage::Ready(image)) => Some(image.clone()),
                    _ => None,
                };
                if thumbnail.is_none() && !is_video {
                    return None;
                }
                let (viewing, focus) = (self.viewing.clone(), self.focus.clone());
                let original = self.original.borrow().clone().filter(|(o, _)| *o == id).map(|(_, state)| state);
                let (retry_original, retry_id) = (self.original.clone(), id.clone());
                let retry_want = if is_video { self.want_video.clone() } else { self.want_original.clone() };
                Some(crate::media_view::large_view(thumbnail, original, is_video, t, move |window| {
                    close_viewer(&viewing, &focus, "escape", window);
                    window.refresh();
                }, move |window| {
                    *retry_original.borrow_mut() = None;
                    *retry_want.borrow_mut() = Some(retry_id.clone());
                    window.refresh();
                }))
            }))
            .map(|d| {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    let published = self.semantic_bounds.clone();
                    let published_retry = self.retry_bounds.clone();
                    let published_actions = self.actions_bounds.clone();
                    let published_selection = self.selection_bounds.clone();
                    let state = self.state.clone();
                    // GPUI bounds_for_item omits rows above a bottom-aligned logical
                    // scroll top. Use actual painted row geometry, scoped by event ID.
                    d.child(canvas(|_, _, _| (), move |_, _, window, _| {
                        let viewport = state.viewport_bounds();
                        selection_painted.borrow_mut().retain(|_, bounds| bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom());
                        if *published_selection.borrow() != *selection_painted.borrow() {
                            *published_selection.borrow_mut() = selection_painted.borrow().clone();
                            window.on_next_frame(|window, _| window.refresh());
                        }
                        painted.borrow_mut().retain(|_, bounds| bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom());
                        retry_painted.borrow_mut().retain(|_, bounds| bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom());
                        actions_painted.borrow_mut().retain(|_, bounds| bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom());
                        if *published_actions.borrow() != *actions_painted.borrow() {
                            *published_actions.borrow_mut() = actions_painted.borrow().clone();
                            window.on_next_frame(|window, _| window.refresh());
                        }
                        if *published_retry.borrow() != *retry_painted.borrow() {
                            *published_retry.borrow_mut() = retry_painted.borrow().clone();
                            window.on_next_frame(|window, _| window.refresh());
                        }
                        if *published.borrow() != *painted.borrow() {
                            *published.borrow_mut() = painted.borrow().clone();
                            window.on_next_frame(|window, _| window.refresh());
                        }
                    }).absolute().size_full())
                }
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                { d }
            })
            .child(self.scrollbar.render("history-scrollbar", crate::scrollbar_view::ScrollTarget::History(self.state.clone(), self.extent.clone()), t))
            .child(canvas(|_, _, _| (), move |_, _, window, _| {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                if *published_event_ids.borrow() != *event_id_painted.borrow() {
                    *published_event_ids.borrow_mut() = event_id_painted.borrow().clone();
                    window.on_next_frame(|window, _| window.refresh());
                }
                if *published_links.borrow() != *link_painted.borrow() {
                    *published_links.borrow_mut() = link_painted.borrow().clone();
                    window.on_next_frame(|window, _| window.refresh());
                }
            }).absolute().size_full())
            .when_some(menu_target, |d, target| {
                let (surface, border, text_color) = (t.surface, t.border, t.text);
                let item = |label: &'static str, i: usize| div().id(("link-menu-item", i)).px_4().py_2().cursor_pointer().text_size(px(13.)).text_color(rgb(text_color)).hover(|s| s.bg(rgb(border))).child(label);
                let (menu, sources, confirmation, requested) = (self.link_menu.clone(), self.link_sources.clone(), self.link.clone(), self.requested_link.clone());
                let (menu_out, menu_copy) = (self.link_menu.clone(), self.link_menu.clone());
                let url = target.url().as_str().to_owned();
                d.child(gpui::deferred(gpui::anchored().position(self.link_menu_at.get()).snap_to_window_with_margin(px(8.)).child(
                    div().id("link-menu").occlude().flex().flex_col().py_1().rounded_md().border_1().border_color(rgb(border)).bg(rgb(surface)).min_w(px(140.))
                        .on_mouse_down_out(move |_, window, _| { menu_out.borrow_mut().close(); window.refresh(); })
                        .child(item(matrix_desktop::LinkMenu::ITEMS[0], 0).on_click(move |_, window, cx| {
                            menu_copy.borrow_mut().close();
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            window.refresh();
                        }))
                        .child(item(matrix_desktop::LinkMenu::ITEMS[1], 1).on_click(move |_, window, _| {
                            if menu.borrow_mut().open_link(&mut sources.borrow_mut(), &mut confirmation.borrow_mut()) {
                                requested.replace(Some(target.clone()));
                            }
                            window.refresh();
                        })),
                )).with_priority(2))
            })
            .when_some(pending, |d, request| {
                let (cancel, confirm) = confirmation_callbacks(self.link.clone(), self.link_sources.clone(), request.clone());
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let open_focused = self.confirmation_focus.get() == (request.generation(), true);
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let confirmation_focused = self.focus.is_focused(window);
                let observe = |d: gpui::Div, index: usize| {
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    {
                        let published = self.confirmation_bounds.clone();
                        let generation = request.generation();
                        d.relative().child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                            let mut current = published.borrow_mut();
                            if current.0 != generation { *current = (generation, [None; 3]); }
                            if current.1[index] != Some(bounds) {
                                current.1[index] = Some(bounds);
                                window.on_next_frame(|window, _| window.refresh());
                            }
                        }).absolute().inset_0())
                    }
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    { let _ = index; d }
                };
                d.child(div().id("link-confirmation").occlude().absolute().bottom_0().left_0().right_0()
                    .min_h(px(132.)).px_6().py_3().bg(rgb(t.surface)).border_t_2().border_color(rgb(t.accent))
                    .flex().flex_col().gap_2()
                    .child("Aprire nel browser esterno? Controlla l'indirizzo.")
                    .child(observe(div().text_size(px(12.)).text_color(rgb(t.secondary)).whitespace_normal().child(request.url().as_str().to_owned()), 0))
                    .child(div().flex().justify_end().gap_4()
                        .child(observe(div().px_4().py_2().border_1().border_color(rgb({
                            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                            { if confirmation_focused && !open_focused { t.accent } else { t.surface } }
                            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                            { t.surface }
                        })).cursor_pointer().child("Annulla"), 1).id("link-cancel")
                            .on_click(move |_, window, _| { cancel(); window.refresh(); }))
                        .child(observe(div().px_4().py_2().border_1().border_color(rgb({
                            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                            { if confirmation_focused && open_focused { t.text } else { t.accent } }
                            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                            { t.accent }
                        })).rounded_lg().bg(rgb(t.accent)).text_color(rgb(t.on_accent)).cursor_pointer().child("Apri link"), 2).id("link-open")
                            .on_click(move |_, window, cx| {
                                if let Some(url) = confirm() { cx.open_url(url.as_str()); }
                                window.refresh();
                            }))))
            })
            .when(probe, |d| {
                // Post-list paint observation only, not a test command channel. No files,
                // body text, IDs or account data leave this explicit fixture diagnostic.
                d.child(canvas(|_, _, _| (), move |_, _, _, _| {
                    let viewport = state.viewport_bounds();
                    let indices = constructed.borrow();
                    let mut rows = Vec::new();
                    for (index, body, stamp, source, row_bounds, status, annotated) in layouts.borrow().iter() {
                        let Some(row) = *row_bounds.borrow() else { continue };
                        if row.bottom() <= viewport.top() || row.top() >= viewport.bottom() { continue; }
                        let points = matrix_desktop::message_links(source).iter().filter_map(|link| {
                            body.position_for_index(link.range.start).map(|p| format!("[{},{}]", f32::from(p.x), f32::from(p.y)))
                        }).collect::<Vec<_>>().join(",");
                        rows.push(format!(
                            "{{\"index\":{index},\"send_state\":\"{status}\",\"annotated\":{annotated},\"links\":[{points}],\"row\":{},\"body\":{},\"stamp\":{},\"lines\":{},\"line_height\":{},\"complete\":{}}}",
                            rectangle(row), rectangle(body.bounds()), rectangle(stamp.bounds()),
                            body.wrapped_text().lines().count(), f32::from(body.line_height()), body.text() == *source,
                        ));
                    }
                    let diagnostic = format!(
                        "Matrix Desktop timeline {{\"retained\":{retained},\"items\":{},\"constructed\":{:?},\"viewport\":{},\"rows\":[{}]}}",
                        state.item_count(), *indices, rectangle(viewport), rows.join(","),
                    );
                    eprintln!("{diagnostic}");
                }).absolute().size_full())
            })
    }
}

// Capture at paint, not dispatch: a queued callback must never read a newer request.
fn event_id_copy_callback(
    copies: Rc<RefCell<matrix_desktop::EventIdCopies>>,
    target: matrix_desktop::EventIdCopyTarget,
) -> impl Fn() -> Option<String> {
    move || copies.borrow().resolve(&target).map(str::to_owned)
}

// Capture at paint, not dispatch: a queued callback must never read a newer request.
fn confirmation_callbacks(
    confirmation: Rc<RefCell<LinkConfirmation>>,
    sources: Rc<RefCell<matrix_desktop::MessageLinkRequests>>,
    request: matrix_desktop::LinkConfirmationRequest,
) -> (
    impl Fn() -> bool,
    impl Fn() -> Option<matrix_desktop::SafeWebUrl>,
) {
    let cancel = {
        let (confirmation, sources, request) =
            (confirmation.clone(), sources.clone(), request.clone());
        move || {
            confirmation
                .borrow_mut()
                .cancel_current(&request, &sources.borrow())
        }
    };
    let confirm = move || {
        confirmation
            .borrow_mut()
            .confirm_current(&request, &sources.borrow())
    };
    (cancel, confirm)
}

fn rectangle(bounds: Bounds<Pixels>) -> String {
    format!(
        "[{},{},{},{}]",
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height)
    )
}

struct LinkTooltip {
    url: String,
    surface: u32,
    border: u32,
    text: u32,
}

impl gpui::Render for LinkTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.border))
            .bg(rgb(self.surface))
            .text_color(rgb(self.text))
            .text_size(px(12.))
            .max_w(px(480.))
            .child(self.url.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_desktop::MessageLinkRequests;
    use matrix_domain::{AccountId, EventId, RoomId, TimelineId};

    struct MenuHost(TimelineView);
    impl Render for MenuHost {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .size_full()
                .child(self.0.render(matrix_desktop::LIGHT, window, None, None))
        }
    }

    struct ShotHost(TimelineView);
    impl Render for ShotHost {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().flex().flex_col().child(self.0.render(
                matrix_desktop::LIGHT,
                window,
                None,
                None,
            ))
        }
    }

    /// Screenshot driver for the sender-name rule: same fixture with `members` members.
    fn sender_names_screenshot(members: u64) {
        gpui::Application::new().run(move |cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi sender names");
                    cx.new(|cx| {
                        let history = DemoHistory::new(
                            RoomScope {
                                account_id: AccountId::new(1),
                                room_id: RoomId::new("!g:fixture.invalid"),
                            },
                            12,
                        );
                        let v = TimelineView::with_history(history, cx.focus_handle(), false);
                        v.set_member_count(members);
                        ShotHost(v)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(1500)).await;
                let shot = format!("/tmp/gui-out/p230-members-{members}.png");
                assert!(
                    std::process::Command::new("grim")
                        .arg(&shot)
                        .status()
                        .unwrap()
                        .success()
                );
                view.update(cx, |_, _, cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn sender_names_shot_group() {
        sender_names_screenshot(3);
    }

    #[test]
    #[ignore = "requires private display"]
    fn sender_names_shot_pair() {
        sender_names_screenshot(2);
    }

    #[test]
    #[ignore = "requires private display"]
    fn edit_menu_copies_only_focused_message_selection() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Dime private timeline menu");
                    cx.new(|cx| {
                        let mut history = DemoHistory::new(
                            RoomScope {
                                account_id: AccountId::new(1),
                                room_id: RoomId::new("!menu:fixture.invalid"),
                            },
                            0,
                        );
                        history.submit_text("caffè 🦀\nseconda riga".into());
                        history.submit_text("caffè 🦀\nseconda riga".into());
                        let v = TimelineView::with_history(history, cx.focus_handle(), false);
                        let id = v.history.borrow().events()[1].id.clone();
                        let text = v.history.borrow().events()[1].text.clone();
                        v.selection.borrow_mut().begin(id.clone(), &text, 0);
                        v.selection
                            .borrow_mut()
                            .extend(&id, &text, "caffè 🦀".len());
                        v.focus.focus(window);
                        MenuHost(v)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                    assert!(!window.is_action_available(&crate::editing_command::Cut, cx));
                    assert!(!window.is_action_available(&crate::editing_command::Paste, cx));
                    window.dispatch_action(crate::editing_command::tests::menu_action(1), cx);
                })
                .unwrap();
                executor.timer(std::time::Duration::from_millis(80)).await;
                view.update(cx, |v, _, cx| {
                    let copied = cx.read_from_clipboard().and_then(|i| i.text());
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                    assert_eq!(copied.as_deref(), Some("caffè 🦀"));
                    assert_eq!(v.0.history.borrow().events().len(), 2);
                })
                .unwrap();
                for eligibility in ["current", "queued-blur", "queued-retire"] {
                    for index in [1, 3] {
                        view.update(cx, |v, window, cx| {
                            let id = v.0.history.borrow().events()[1].id.clone();
                            let text = v.0.history.borrow().events()[1].text.clone();
                            v.0.selection.borrow_mut().begin(id.clone(), &text, 0);
                            v.0.selection
                                .borrow_mut()
                                .extend(&id, &text, "caffè 🦀".len());
                            v.0.focus.focus(window);
                            cx.notify();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        cx.update(|cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                            assert!(cx.active_window().is_some());
                            cx.dispatch_action(
                                crate::editing_command::tests::menu_action(index).as_ref(),
                            );
                            view.update(cx, |v, window, cx| {
                                if eligibility == "queued-blur" {
                                    cx.focus_handle().focus(window);
                                }
                                if eligibility == "queued-retire" {
                                    v.0.retire_selection();
                                    let id = v.0.history.borrow().events()[1].id.clone();
                                    let text = v.0.history.borrow().events()[1].text.clone();
                                    v.0.selection.borrow_mut().begin(id.clone(), &text, 0);
                                    v.0.selection
                                        .borrow_mut()
                                        .extend(&id, &text, "caffè 🦀".len());
                                }
                            })
                            .unwrap();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        view.update(cx, |v, window, cx| {
                            let history = v.0.history.borrow();
                            let event = &history.events()[1];
                            let copied = cx.read_from_clipboard().and_then(|i| i.text());
                            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                            assert_eq!(
                                copied.as_deref(),
                                Some(if eligibility == "current" && index == 1 {
                                    "caffè 🦀"
                                } else {
                                    "sentinel"
                                })
                            );
                            assert_eq!(
                                v.0.selection.borrow().range(&event.id, &event.text),
                                Some(
                                    0..if eligibility == "current" && index == 3 {
                                        event.text.len()
                                    } else {
                                        "caffè 🦀".len()
                                    }
                                )
                            );
                            assert!(
                                v.0.selection
                                    .borrow()
                                    .range(&history.events()[0].id, &event.text)
                                    .is_none()
                            );
                            assert_eq!(history.events().len(), 2);
                            if eligibility == "queued-blur" {
                                assert!(!v.0.focus.is_focused(window));
                            }
                        })
                        .unwrap();
                    }
                }
                eprintln!("selected-message menu source/focus and read-only availability passed");
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    /// Reading position when older server rows arrive: the row being read keeps its place.
    #[test]
    #[ignore = "requires private display"]
    fn older_rows_keep_the_row_being_read_in_place() {
        use matrix_engine::history::{HistoryEvent, HistorySender};
        gpui::Application::new().run(|cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi anchoring");
                    cx.new(|cx| {
                        let scope = RoomScope {
                            account_id: AccountId::new(1),
                            room_id: RoomId::new("!a:fixture.invalid"),
                        };
                        ShotHost(TimelineView::with_history(
                            DemoHistory::new(scope, 60),
                            cx.focus_handle(),
                            false,
                        ))
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                let pause = || executor.timer(std::time::Duration::from_millis(700));
                pause().await;
                let (id, offset) = view
                    .update(cx, |host, _, _| {
                        host.0.state.scroll_to(ListOffset {
                            item_ix: 20,
                            offset_in_item: px(7.),
                        });
                        (host.0.history.borrow().events()[20].id.clone(), px(7.))
                    })
                    .unwrap();
                pause().await;
                assert!(
                    std::process::Command::new("grim")
                        .arg("/tmp/gui-out/p177-before.png")
                        .status()
                        .unwrap()
                        .success()
                );
                view.update(cx, |host, _, cx| {
                    let older = (0..25)
                        .map(|n| HistoryEvent {
                            poll: None,
                            redacted: false,
                            event_id: EventId::new(format!("$old{n}:fixture.invalid")),
                            body: format!("older {n}"),
                            outgoing: false,
                            timestamp_ms: n,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                            thread: None,
                            thread_root: None,
                            status: false,
                            attachment_name: None,
                            attachment_size: None,
                        })
                        .collect();
                    host.0.prepend_server(older);
                    cx.notify();
                })
                .unwrap();
                pause().await;
                assert!(
                    std::process::Command::new("grim")
                        .arg("/tmp/gui-out/p177-after.png")
                        .status()
                        .unwrap()
                        .success()
                );
                view.update(cx, |host, _, cx| {
                    let top = host.0.state.logical_scroll_top();
                    assert_eq!(
                        host.0.history.borrow().events()[top.item_ix].id,
                        id,
                        "same row at the top"
                    );
                    assert_eq!(top.offset_in_item, offset, "same pixel offset inside it");
                    assert_eq!(top.item_ix, 45);
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        });
    }

    struct ComposerHost(
        TimelineView,
        gpui::Entity<crate::composer_view::ComposerView>,
    );
    impl Render for ComposerHost {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_1().child(self.0.render(
                    matrix_desktop::LIGHT,
                    window,
                    None,
                    None,
                )))
                .child(self.1.clone())
        }
    }

    /// Older rows arriving must not touch the draft, the message selection or where focus sits.
    #[test]
    #[ignore = "requires private display"]
    fn older_rows_keep_draft_selection_and_focus() {
        use matrix_engine::history::{HistoryEvent, HistorySender};
        gpui::Application::new().run(|cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_window, cx| {
                    let scope = RoomScope {
                        account_id: AccountId::new(1),
                        room_id: RoomId::new("!a:fixture.invalid"),
                    };
                    let composer =
                        cx.new(|cx| crate::composer_view::ComposerView::new(scope.clone(), cx));
                    cx.new(|cx| {
                        ComposerHost(
                            TimelineView::with_history(
                                DemoHistory::new(scope, 60),
                                cx.focus_handle(),
                                false,
                            ),
                            composer,
                        )
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                let pause = || executor.timer(std::time::Duration::from_millis(500));
                pause().await;
                let (id, text) = view
                    .update(cx, |host, window, cx| {
                        host.1.update(cx, |c, cx| {
                            gpui::EntityInputHandler::replace_text_in_range(
                                c,
                                None,
                                "bozza caffè 🦀",
                                window,
                                cx,
                            );
                            c.focus_from_accessibility(window, cx);
                        });
                        host.0.state.scroll_to(ListOffset {
                            item_ix: 20,
                            offset_in_item: px(3.),
                        });
                        let history = host.0.history.borrow();
                        let event = &history.events()[22];
                        host.0
                            .selection
                            .borrow_mut()
                            .begin(event.id.clone(), &event.text, 0);
                        host.0
                            .selection
                            .borrow_mut()
                            .extend(&event.id, &event.text, 4);
                        (event.id.clone(), event.text.clone())
                    })
                    .unwrap();
                pause().await;
                let before = view
                    .update(cx, |host, window, cx| {
                        assert!(
                            host.1.read(cx).is_focused(window),
                            "composer focused before"
                        );
                        host.1.read(cx).draft_state()
                    })
                    .unwrap();
                assert_eq!(before.0, "bozza caffè 🦀");
                view.update(cx, |host, _, cx| {
                    let older = (0..25)
                        .map(|n| HistoryEvent {
                            poll: None,
                            redacted: false,
                            event_id: EventId::new(format!("$old{n}:fixture.invalid")),
                            body: format!("older {n}"),
                            outgoing: false,
                            timestamp_ms: n,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                            thread: None,
                            thread_root: None,
                            status: false,
                            attachment_name: None,
                            attachment_size: None,
                        })
                        .collect();
                    host.0.prepend_server(older);
                    cx.notify();
                })
                .unwrap();
                pause().await;
                view.update(cx, |host, window, cx| {
                    assert_eq!(
                        host.1.read(cx).draft_state(),
                        before,
                        "draft text and cursor unchanged"
                    );
                    assert!(host.1.read(cx).is_focused(window), "composer keeps focus");
                    assert_eq!(
                        host.0.selection.borrow().range(&id, &text),
                        Some(0..4),
                        "message selection unchanged"
                    );
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        });
    }

    fn selection_probe(
        check: impl FnOnce(&mut TimelineView, &mut gpui::Window, &mut gpui::App) + 'static,
    ) {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let window = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|_| gpui::Empty)
                })
                .unwrap();
            let view = cx.new(|cx| {
                let mut history = DemoHistory::new(
                    RoomScope {
                        account_id: AccountId::new(1),
                        room_id: RoomId::new("!commands:fixture.invalid"),
                    },
                    0,
                );
                history.submit_text("caffè 🦀\nseconda riga".into());
                history.submit_text("caffè 🦀\nseconda riga".into());
                TimelineView::with_history(history, cx.focus_handle(), false)
            });
            cx.spawn(async move |cx| {
                window
                    .update(cx, |_, window, cx| {
                        view.update(cx, |v, cx| check(v, window, cx))
                    })
                    .unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    fn selection_key(
        v: &TimelineView,
        chord: &str,
        mac: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        (v.selection_key_callback(mac))(
            &gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse(chord).unwrap(),
                is_held: false,
            },
            window,
            cx,
        );
    }

    /// Paints one video row (with or without a ready thumbnail) and checks what reached the screen.
    fn video_bubble_shot(thumbnail: bool) {
        use matrix_engine::history::{HistoryEvent, HistorySender, VideoInfo};
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let id = EventId::new("$video:fixture.invalid");
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi video bubble");
                    cx.new(|cx| {
                        let scope = RoomScope {
                            account_id: AccountId::new(1),
                            room_id: RoomId::new("!video:fixture.invalid"),
                        };
                        let v = TimelineView::with_history(
                            DemoHistory::new(scope, 0),
                            cx.focus_handle(),
                            false,
                        );
                        v.append_server(vec![HistoryEvent {
                            poll: None,
                            redacted: false,
                            event_id: id.clone(),
                            body: "Video · 0:12 · 1.4 MB".into(),
                            outgoing: false,
                            timestamp_ms: 0,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: Some(VideoInfo {
                                duration_ms: Some(12_000),
                                size: Some(1_468_006),
                                #[cfg(feature = "live")]
                                thumbnail: None,
                            }),
                            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                            thread: None,
                            thread_root: None,
                            status: false,
                            attachment_name: None,
                            attachment_size: None,
                        }]);
                        if thumbnail {
                            let buffer =
                                image::RgbaImage::from_raw(160, 90, vec![255; 160 * 90 * 4])
                                    .unwrap();
                            v.images.borrow_mut().insert(
                                id.clone(),
                                crate::media_view::InlineImage::Ready(std::sync::Arc::new(
                                    gpui::RenderImage::new([image::Frame::new(buffer)]),
                                )),
                            );
                        }
                        ShotHost(v)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(500)).await;
                view.update(cx, |host, _, cx| {
                    let parts = host.0.video_painted.borrow();
                    let part = |name| {
                        parts
                            .get(&(
                                TimelineId::Server(EventId::new("$video:fixture.invalid")),
                                name,
                            ))
                            .copied()
                            .unwrap_or_else(|| panic!("{name} was not painted"))
                    };
                    let (picture, play, duration, size) = (
                        part("picture"),
                        part("play"),
                        part("duration"),
                        part("size"),
                    );
                    let (w, h) = if thumbnail { (160., 90.) } else { (200., 150.) };
                    assert_eq!(
                        f32::from(picture.size.width),
                        w,
                        "thumbnail size or neutral box"
                    );
                    assert_eq!(f32::from(picture.size.height), h);
                    for inner in [play, duration, size] {
                        let (tl, br) = (inner.origin, inner.bottom_right());
                        assert!(
                            tl.x >= picture.left() - px(1.)
                                && tl.y >= picture.top() - px(1.)
                                && br.x <= picture.right() + px(1.)
                                && br.y <= picture.bottom() + px(1.),
                            "overlay inside the picture"
                        );
                    }
                    let (cx_, cy_) = (picture.center().x, picture.center().y);
                    assert!(
                        (play.center().x - cx_).abs() < px(1.)
                            && (play.center().y - cy_).abs() < px(1.),
                        "play at the centre"
                    );
                    assert!(
                        duration.origin.x < picture.center().x
                            && size.origin.x > picture.center().x,
                        "duration left, size right"
                    );
                    assert!(
                        duration.origin.y > cy_ && size.origin.y > cy_,
                        "labels along the bottom"
                    );
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_video_bubble_with_thumbnail() {
        video_bubble_shot(true);
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_video_bubble_without_thumbnail_is_a_neutral_box() {
        video_bubble_shot(false);
    }

    /// Paints a row with a pill for me, one for another person, «@room» and a room, then clicks them for real
    /// (synthetic mouse events through the window): only the two user pills open a profile, each its own.
    #[test]
    #[ignore = "requires private display"]
    fn painted_pills_click_opens_that_profile_and_text_around_does_not() {
        use matrix_engine::{history::{HistoryEvent, HistorySender}, rich_text::{MentionKind, MentionSpan}};
        gpui::Application::new().run(move |cx| {
            let id = EventId::new("$pills:fixture.invalid");
            let (me, bea) = ("@anna:fixture.invalid", "@bea:fixture.invalid");
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi pills");
                    cx.new(|cx| {
                        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!pills:fixture.invalid") };
                        let v = TimelineView::with_history(DemoHistory::new(scope, 0), cx.focus_handle(), false);
                        let body = "Anna e Bea, @room in #qa fine";
                        let at = |word: &str| { let i = body.find(word).unwrap(); i..i + word.len() };
                        let user = |who: &str, name: &str, me| MentionKind::User { sender: HistorySender::new(who, Some(name)), me };
                        v.append_server(vec![HistoryEvent {
                            poll: None, redacted: false, event_id: id.clone(), body: body.into(), outgoing: false, timestamp_ms: 0,
                            sender: HistorySender::new("@z:fixture.invalid", Some("Zeno")), video: None, styles: Vec::new(),
                            mentions: vec![
                                MentionSpan { range: at("Anna"), kind: user(me, "Anna", true) },
                                MentionSpan { range: at("Bea"), kind: user(bea, "Bea", false) },
                                MentionSpan { range: at("@room"), kind: MentionKind::AtRoom },
                                MentionSpan { range: at("#qa"), kind: MentionKind::Room },
                            ],
                            highlighted: true, thread: None, thread_root: None, status: false, attachment_name: None, attachment_size: None,
                        }]);
                        ShotHost(v)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(800)).await;
                let (anna, bea_box) = view
                    .update(cx, |host, _, _| {
                        let event = host.0.history.borrow().events()[0].id.clone();
                        let mut painted: Vec<_> = host.0.pill_bounds.borrow().iter().filter(|((e, _), _)| *e == event).map(|((_, s), b)| (*s, *b)).collect();
                        painted.sort_by_key(|(s, _)| *s);
                        assert_eq!(painted.len(), 2, "only the two user pills are painted as targets: {painted:?}");
                        (painted[0].1, painted[1].1)
                    })
                    .unwrap();
                assert!(anna.size.width > px(10.) && anna.right() <= bea_box.left(), "left to right, apart: {anna:?} {bea_box:?}");
                assert!((anna.center().y - bea_box.center().y).abs() < px(1.), "same line");
                // Real X11 clicks (xdotool, window-relative), the same path a hand takes.
                let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--name", "Gnobbi pills"]).output().unwrap().stdout).unwrap();
                let win = win.lines().next().expect("window found").to_owned();
                let mut click = async |at: gpui::Point<Pixels>| {
                    view.update(cx, |host, _, _| { host.0.want_profile.replace(None); }).unwrap();
                    let (x, y) = (f32::from(at.x).round().to_string(), f32::from(at.y).round().to_string());
                    assert!(std::process::Command::new("xdotool").args(["mousemove", "--window", &win, &x, &y, "click", "1"]).status().unwrap().success());
                    executor.timer(std::time::Duration::from_millis(500)).await;
                    view.update(cx, |host, _, _| host.0.want_profile.borrow().clone()).unwrap()
                };
                let on = |b: Bounds<Pixels>| gpui::point(b.right() - px(6.), b.center().y);
                assert_eq!(click(on(anna)).await.as_deref(), Some(me), "my pill opens my profile");
                assert_eq!(click(on(bea_box)).await.as_deref(), Some(bea), "the other's pill opens theirs");
                assert_eq!(click(gpui::point(bea_box.right() + px(300.), bea_box.center().y)).await, None, "empty space after the text opens nothing");
                assert_eq!(click(gpui::point(anna.right() + px(3.), anna.center().y)).await, None, "the plain text between pills opens nothing");
                view.update(cx, |_, _, cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    struct NarrowHost(TimelineView);
    impl Render for NarrowHost {
        fn render(&mut self, window: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().w(px(360.)).h_full().flex().flex_col().child(self.0.render(matrix_desktop::LIGHT, window, None, None))
        }
    }

    /// Paints one poll row in a 360 px wide timeline and checks parts and containment.
    fn poll_bubble_shot(poll: matrix_domain::Poll) {
        use crate::poll_view::PollPart;
        use matrix_engine::history::{HistoryEvent, HistorySender};
        assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
        gpui::Application::new().run(move |cx| {
            let shown = poll.results_visible();
            let options = poll.answers.len();
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi poll bubble");
                    cx.new(|cx| {
                        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!poll:fixture.invalid") };
                        let v = TimelineView::with_history(DemoHistory::new(scope, 0), cx.focus_handle(), false);
                        v.append_server(vec![HistoryEvent {
                            poll: Some(poll.clone()),
                            redacted: false,
                            event_id: EventId::new("$poll:fixture.invalid"),
                            body: poll.question.clone(),
                            outgoing: false,
                            timestamp_ms: 0,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                            thread: None,
                            thread_root: None,
                            status: false,
                            attachment_name: None,
                            attachment_size: None,
                        }]);
                        NarrowHost(v)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(500)).await;
                view.update(cx, |host, _, cx| {
                    let parts = host.0.poll_painted.borrow();
                    let id = TimelineId::Server(EventId::new("$poll:fixture.invalid"));
                    let get = |part| parts.get(&(id.clone(), part)).copied();
                    let bubble = get(PollPart::Bubble).expect("bubble was not painted");
                    assert!(bubble.left() >= px(0.) && bubble.right() <= px(361.), "no overflow at 360: {bubble:?}");
                    for ix in 0..options {
                        let option = get(PollPart::Option(ix)).expect("option was not painted");
                        assert!(option.left() >= bubble.left() - px(1.) && option.right() <= bubble.right() + px(1.), "option inside the bubble");
                        for part in [PollPart::Votes(ix), PollPart::Bar(ix)] {
                            assert_eq!(get(part).is_some(), shown, "{part:?}: counts and bars only when results are visible");
                            if let Some(inner) = get(part) {
                                assert!(inner.right() <= bubble.right() + px(1.), "{part:?} inside the bubble");
                            }
                        }
                    }
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        });
    }

    fn sample_poll(disclosed: bool, ended: bool) -> matrix_domain::Poll {
        let a = |id: &str, text: &str, votes, own| matrix_domain::PollAnswer { id: id.into(), text: text.into(), votes, own };
        matrix_domain::Poll {
            question: "Dove ceniamo venerdì sera con tutti gli amici del corso di fotografia?".into(),
            answers: vec![
                a("y", "Pizzeria da Mario in centro, quella con il forno a legna e il tavolo lungo", 5, true),
                a("n", "No", 1, false),
            ],
            max_selections: 1,
            multiple: false,
            disclosed,
            ended,
            voters: 6,
            can_end: false,
        }
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_poll_bubble_open_disclosed() {
        poll_bubble_shot(sample_poll(true, false));
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_poll_bubble_undisclosed_open_has_no_counts_or_bars() {
        poll_bubble_shot(sample_poll(false, false));
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_poll_bubble_ended_with_own_vote() {
        poll_bubble_shot(sample_poll(false, true));
    }

    #[cfg(feature = "live")]
    struct VotingHost(TimelineView, Rc<RefCell<matrix_desktop::LiveOperations>>);
    #[cfg(feature = "live")]
    impl Render for VotingHost {
        fn render(&mut self, window: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().w(px(360.)).h_full().flex().flex_col().child(self.0.render(matrix_desktop::LIGHT, window, None, Some(self.1.clone())))
        }
    }

    /// Clicks option `ix` with the mouse and checks the answers of the Vote that goes out (None = nothing sent).
    #[cfg(feature = "live")]
    fn poll_click_shot(poll: matrix_domain::Poll, ix: usize, expected: Option<Vec<&'static str>>) {
        use crate::poll_view::PollPart;
        use matrix_engine::history::{HistoryEvent, HistorySender};
        use matrix_engine::operations::{Operation, OperationCommand};
        assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
        gpui::Application::new().run(move |cx| {
            let operations = Rc::new(RefCell::new(matrix_desktop::LiveOperations::default()));
            let ops = operations.clone();
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi poll click");
                    cx.new(|cx| {
                        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!poll:fixture.invalid") };
                        let v = TimelineView::with_history(DemoHistory::new(scope, 0), cx.focus_handle(), false);
                        v.append_server(vec![HistoryEvent {
                            poll: Some(poll.clone()),
                            redacted: false,
                            event_id: EventId::new("$poll:fixture.invalid"),
                            body: poll.question.clone(),
                            outgoing: false,
                            timestamp_ms: 0,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                            thread: None,
                            thread_root: None,
                            status: false,
                            attachment_name: None,
                            attachment_size: None,
                        }]);
                        VotingHost(v, ops)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(500)).await;
                let at = view
                    .update(cx, |host, _, _| {
                        let parts = host.0.poll_painted.borrow();
                        let option = parts[&(TimelineId::Server(EventId::new("$poll:fixture.invalid")), PollPart::Option(ix))];
                        option.center()
                    })
                    .unwrap();
                // Real pointer through the Wayland test session (the window is the only one, at 0,0).
                let helper = std::env::var("WL_INPUT").expect("run through the gui-test-wayland script");
                let status = std::process::Command::new(helper).arg(format!("click:{},{}", f32::from(at.x) as i32, f32::from(at.y) as i32)).status().unwrap();
                assert!(status.success());
                executor.timer(std::time::Duration::from_millis(300)).await;
                let sent = match operations.borrow_mut().take_command() {
                    Some(OperationCommand::Apply(request)) => {
                        assert_eq!(request.target, EventId::new("$poll:fixture.invalid"));
                        let Operation::Vote { answers } = request.operation else { panic!("not a vote") };
                        Some(answers)
                    }
                    other => {
                        assert!(other.is_none(), "unexpected command");
                        None
                    }
                };
                assert_eq!(sent, expected.map(|a| a.into_iter().map(String::from).collect::<Vec<_>>()));
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }

    fn poll_with(multiple: bool, ended: bool, own: &[&str]) -> matrix_domain::Poll {
        let a = |id: &str| matrix_domain::PollAnswer { id: id.into(), text: id.into(), votes: 1, own: own.contains(&id) };
        matrix_domain::Poll {
            question: "q".into(),
            answers: vec![a("a"), a("b"), a("c")],
            max_selections: if multiple { 2 } else { 1 },
            multiple,
            disclosed: true,
            ended,
            voters: 1,
            can_end: false,
        }
    }

    #[test]
    #[ignore = "requires private display"]
    #[cfg(feature = "live")]
    fn painted_poll_click_single_choice_replaces_the_vote() {
        poll_click_shot(poll_with(false, false, &["a"]), 1, Some(vec!["b"]));
    }

    #[test]
    #[ignore = "requires private display"]
    #[cfg(feature = "live")]
    fn painted_poll_click_multiple_choice_adds() {
        poll_click_shot(poll_with(true, false, &["a"]), 2, Some(vec!["a", "c"]));
    }

    #[test]
    #[ignore = "requires private display"]
    #[cfg(feature = "live")]
    fn painted_poll_click_multiple_choice_removes() {
        poll_click_shot(poll_with(true, false, &["a", "b"]), 0, Some(vec!["b"]));
    }

    #[test]
    #[ignore = "requires private display"]
    #[cfg(feature = "live")]
    fn painted_poll_click_over_the_cap_sends_nothing() {
        poll_click_shot(poll_with(true, false, &["a", "b"]), 2, None);
    }

    #[test]
    #[ignore = "requires private display"]
    #[cfg(feature = "live")]
    fn painted_poll_click_on_a_closed_poll_sends_nothing() {
        poll_click_shot(poll_with(false, true, &["a"]), 1, None);
    }

    fn ready_image() -> crate::media_view::InlineImage {
        let buffer = image::RgbaImage::from_raw(2, 2, vec![255; 16]).unwrap();
        crate::media_view::InlineImage::Ready(std::sync::Arc::new(gpui::RenderImage::new([
            image::Frame::new(buffer),
        ])))
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    struct ThemedHost(TimelineView, matrix_desktop::DesignTokens);
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    impl Render for ThemedHost {
        fn render(&mut self, window: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().size_full().flex().flex_col().child(self.0.render(self.1, window, None, None))
        }
    }

    /// «Scarica» ↓ painted for real: on a received and an own-sent image and file (not on text, not on
    /// a row still sending), 32x32 at least, reachable with Tab and Shift-Tab, in the given theme.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn save_icons_painted(dark: bool) {
        use matrix_engine::history::{HistoryEvent, HistorySender};
        assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
        gpui::Application::new().run(move |cx| {
            let tokens = if dark { matrix_desktop::DARK } else { matrix_desktop::LIGHT };
            let ids = ["$img-in", "$file-in", "$img-out", "$file-out"].map(|id| EventId::new(format!("{id}:fixture.invalid")));
            let event = |id: &EventId, outgoing: bool, name: Option<&str>, image: bool, ms: u64| HistoryEvent {
                poll: None,
                redacted: false,
                event_id: id.clone(),
                body: if image { matrix_engine::history::IMAGE_BODY.into() } else { name.unwrap_or("solo testo").into() },
                outgoing,
                timestamp_ms: ms,
                sender: HistorySender::new(if outgoing { "@me:fixture.invalid" } else { "@a:fixture.invalid" }, None),
                video: None,
                styles: Vec::new(), mentions: Vec::new(), highlighted: false,
                thread: None,
                thread_root: None,
                status: false,
                attachment_name: name.map(Into::into),
                attachment_size: name.map(|_| 2048),
            };
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Gnobbi save icons");
                    cx.new(|cx| {
                        let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!save:fixture.invalid") };
                        let mut history = DemoHistory::new(scope, 0);
                        history.submit_text("ancora in invio".into()); // pending: no icon
                        let v = TimelineView::with_history(history, cx.focus_handle(), true);
                        v.append_server(vec![
                            event(&ids[0], false, Some("foto.png"), true, 1),
                            event(&ids[1], false, Some("rel.pdf"), false, 2),
                            event(&ids[2], true, Some("mia.png"), true, 3),
                            event(&ids[3], true, Some("mio.pdf"), false, 4),
                            event(&EventId::new("$txt:fixture.invalid"), true, None, false, 5),
                        ]);
                        v.images.borrow_mut().insert(ids[0].clone(), ready_image());
                        v.images.borrow_mut().insert(ids[2].clone(), ready_image());
                        ThemedHost(v, tokens)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(800)).await;
                view.update(cx, |host, window, cx| {
                    let v = &host.0;
                    let painted = v.save_painted.borrow();
                    assert_eq!(painted.len(), 4, "one ↓ per attachment, none on text or on a row still sending");
                    for id in &ids {
                        let b = painted.get(id).unwrap_or_else(|| panic!("no ↓ painted for {id:?}"));
                        assert!(f32::from(b.size.width) >= 32. && f32::from(b.size.height) >= 32., "{id:?}: {:?}", b.size);
                    }
                    drop(painted);
                    v.focus.focus(window);
                    let mut forward = 0;
                    if v.enter_icons(false, window) {
                        forward = 1;
                        while matches!(v.step_icons(false, window, cx), IconStep::Moved) {
                            forward += 1;
                        }
                    }
                    v.focus.focus(window);
                    let mut backward = 0;
                    if v.enter_icons(true, window) {
                        backward = 1;
                        while matches!(v.step_icons(true, window, cx), IconStep::Moved) {
                            backward += 1;
                        }
                    }
                    println!("SAVEICONS dark={dark} painted=4 tab_stops_forward={forward} tab_stops_backward={backward}");
                    assert_eq!((forward, backward), (4, 4), "Tab and Shift-Tab reach every ↓ once");
                    cx.quit();
                })
                .unwrap();
            })
            .detach();
        });
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    #[test]
    #[ignore = "requires private display"]
    fn painted_save_icons_light() {
        save_icons_painted(false);
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    #[test]
    #[ignore = "requires private display"]
    fn painted_save_icons_dark() {
        save_icons_painted(true);
    }

    #[test]
    #[ignore = "requires private display"]
    fn large_image_view_opens_and_escape_returns_focus_to_timeline() {
        selection_probe(|v, window, _| {
            let id = matrix_domain::EventId::new("$picture:fixture.invalid");
            v.images.borrow_mut().insert(id.clone(), ready_image());
            let other = matrix_domain::EventId::new("$missing:fixture.invalid");
            v.open_image(other, window);
            assert!(
                v.viewing().is_none(),
                "an image that is not loaded has no large view"
            );
            v.open_image(id.clone(), window);
            assert_eq!(v.viewing(), Some(id.clone()));
            assert!(
                v.focus.is_focused(window),
                "the large view owns keyboard focus"
            );
            assert!(!v.viewer_key("a", window), "other keys leave the view open");
            assert_eq!(v.viewing(), Some(id));
            assert!(v.viewer_key("escape", window));
            assert!(v.viewing().is_none());
            assert!(v.focus.is_focused(window), "focus returns to the timeline");
            assert!(
                !v.viewer_key("escape", window),
                "Escape with no view is not consumed"
            );
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn mac_command_copies_only_selected_message_body() {
        selection_probe(|v, window, cx| {
            let history = v.history.borrow();
            let event = &history.events()[1];
            let id = event.id.clone();
            let text = event.text.clone();
            drop(history);
            v.selection.borrow_mut().begin(id.clone(), &text, 0);
            v.selection
                .borrow_mut()
                .extend(&id, &text, "caffè 🦀".len());
            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
            selection_key(v, "cmd-c", true, window, cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some("caffè 🦀"),
                "Cmd+C must copy the exact selected Unicode subrange"
            );
            selection_key(v, "cmd-a", true, window, cx);
            assert_eq!(v.selection.borrow().range(&id, &text), Some(0..text.len()));
            assert!(
                v.selection
                    .borrow()
                    .range(&v.history.borrow().events()[0].id, &text)
                    .is_none(),
                "identical body must not select a different event"
            );
            selection_key(v, "cmd-c", true, window, cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some(text.as_str())
            );
            assert_eq!(v.history.borrow().events().len(), 2);
            cx.write_to_clipboard(ClipboardItem::new_string("".into()));
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn retired_selection_key_cannot_touch_replacement_selection() {
        selection_probe(|v, window, cx| {
            let id = v.history.borrow().events()[1].id.clone();
            let text = v.history.borrow().events()[1].text.clone();
            for (mac, prefix) in [(false, "ctrl"), (true, "cmd")] {
                v.selection.borrow_mut().begin(id.clone(), &text, 0);
                v.selection.borrow_mut().extend(&id, &text, "caffè".len());
                let old = v.selection_key_callback(mac);
                v.retire_selection();
                v.selection.borrow_mut().begin(id.clone(), &text, 0);
                v.selection.borrow_mut().extend(&id, &text, "caffè".len());
                cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                for key in ["c", "a"] {
                    old(
                        &gpui::KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(&format!("{prefix}-{key}")).unwrap(),
                            is_held: false,
                        },
                        window,
                        cx,
                    );
                }
                assert_eq!(
                    cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                    Some("sentinel"),
                    "retired callback must not export replacement selection"
                );
                assert_eq!(
                    v.selection.borrow().range(&id, &text),
                    Some(0.."caffè".len())
                );
                selection_key(v, &format!("{prefix}-a"), mac, window, cx);
                selection_key(v, &format!("{prefix}-c"), mac, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                    Some(text.as_str())
                );
            }
            cx.write_to_clipboard(ClipboardItem::new_string("".into()));
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn selection_commands_preserve_ctrl_and_reject_foreign_bodies() {
        selection_probe(|v, window, cx| {
            let id = v.history.borrow().events()[1].id.clone();
            let text = v.history.borrow().events()[1].text.clone();
            for (mac, prefix) in [(false, "ctrl"), (true, "ctrl"), (true, "cmd")] {
                v.selection.borrow_mut().begin(id.clone(), &text, 0);
                selection_key(v, &format!("{prefix}-a"), mac, window, cx);
                selection_key(v, &format!("{prefix}-c"), mac, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                    Some(text.as_str())
                );
                selection_key(v, "left", mac, window, cx);
                assert!(v.selection.borrow().range(&id, &text).is_none());
                selection_key(v, "shift-right", mac, window, cx);
                assert_eq!(v.selection.borrow().range(&id, &text), Some(0..1));
                for chord in ["shift-cmd-a", "alt-cmd-a", "alt-cmd-c"] {
                    selection_key(v, chord, mac, window, cx);
                    assert_eq!(v.selection.borrow().range(&id, &text), Some(0..1));
                }
                if !mac {
                    for chord in ["cmd-a", "ctrl-cmd-a"] {
                        selection_key(v, chord, mac, window, cx);
                        assert_eq!(
                            v.selection.borrow().range(&id, &text),
                            Some(0..1),
                            "Linux platform combinations keep the previous selection policy"
                        );
                    }
                }
                for change in ["body", "event", "room", "account", "retire"] {
                    let mut wrong = id.clone();
                    match change {
                        "event" => wrong.id = TimelineId::Server(EventId::new("$foreign")),
                        "room" => wrong.scope.room_id = RoomId::new("!foreign:fixture.invalid"),
                        "account" => wrong.scope.account_id = AccountId::new(2),
                        _ => (),
                    }
                    let body = if change == "body" {
                        "wrong body"
                    } else {
                        &text
                    };
                    v.selection.borrow_mut().begin(wrong.clone(), body, 0);
                    v.selection.borrow_mut().extend(&wrong, body, 1);
                    if change == "retire" {
                        v.retire_selection();
                    }
                    let before = v.selection.borrow().range(&wrong, body);
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                    selection_key(v, &format!("{prefix}-a"), mac, window, cx);
                    selection_key(v, &format!("{prefix}-c"), mac, window, cx);
                    assert_eq!(
                        cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                        Some("sentinel")
                    );
                    assert_eq!(v.selection.borrow().range(&wrong, body), before);
                }
            }
            assert_eq!(v.history.borrow().events().len(), 2);
            assert_eq!(v.history.borrow().events()[1].id, id);
            assert_eq!(v.history.borrow().events()[1].text, text);
            cx.write_to_clipboard(ClipboardItem::new_string("".into()));
        });
    }

    #[test]
    fn queued_native_event_stamp_callback_never_copies_a_new_source() {
        for change in ["body", "event", "room", "account", "retire"] {
            let copies = Rc::new(RefCell::new(matrix_desktop::EventIdCopies::default()));
            let id = ScopedTimelineId {
                scope: RoomScope {
                    account_id: AccountId::new(80),
                    room_id: RoomId::new("!stamp:test"),
                },
                id: TimelineId::Server(EventId::new("$exact:test")),
            };
            let target = copies.borrow_mut().target(&id, "body").unwrap();
            let queued = event_id_copy_callback(copies.clone(), target);
            assert_eq!(queued(), Some("$exact:test".into()));
            let mut replacement = id.clone();
            let body = if change == "body" {
                "replacement"
            } else {
                "body"
            };
            match change {
                "event" => replacement.id = TimelineId::Server(EventId::new("$new:test")),
                "room" => replacement.scope.room_id = RoomId::new("!new:test"),
                "account" => replacement.scope.account_id = AccountId::new(81),
                "retire" => copies.borrow_mut().clear(),
                _ => (),
            }
            copies.borrow_mut().retain_current([(&replacement, body)]);
            let new = copies.borrow_mut().target(&replacement, body).unwrap();
            assert!(queued().is_none());
            assert!(event_id_copy_callback(copies.clone(), new)().is_some());
            copies.borrow_mut().retain_current([(&id, "body")]);
            let returned = copies.borrow_mut().target(&id, "body").unwrap();
            assert!(queued().is_none());
            assert_eq!(
                event_id_copy_callback(copies, returned)(),
                Some("$exact:test".into())
            );
        }
    }

    #[test]
    fn captured_offline_callback_rejects_live_retired_and_repeated_sources() {
        let mut h = DemoHistory::new(
            RoomScope {
                account_id: AccountId::new(81),
                room_id: RoomId::new("!offline:test"),
            },
            0,
        );
        h.submit_text("locale 🙂".into());
        let history = Rc::new(RefCell::new(h));
        let sources = Rc::new(RefCell::new(matrix_desktop::OfflineDelivery::default()));
        let target = sources
            .borrow_mut()
            .target(&history.borrow().events()[0])
            .unwrap();
        let blocked =
            offline_delivery_callback(sources.clone(), history.clone(), target.clone(), true);
        assert_eq!(blocked(), None);
        let pending = offline_delivery_callback(sources.clone(), history.clone(), target, false);
        assert_eq!(pending(), Some(0));
        assert_eq!(pending(), None);
        let failed = sources
            .borrow_mut()
            .target(&history.borrow().events()[0])
            .unwrap();
        let retry = offline_delivery_callback(sources.clone(), history.clone(), failed, false);
        assert_eq!(retry(), Some(0));
        assert_eq!(pending(), None);
        let fresh = sources
            .borrow_mut()
            .target(&history.borrow().events()[0])
            .unwrap();
        let fresh = offline_delivery_callback(sources.clone(), history.clone(), fresh, false);
        sources.borrow_mut().clear();
        sources
            .borrow_mut()
            .target(&history.borrow().events()[0])
            .unwrap();
        assert_eq!(fresh(), None);
        assert_eq!(history.borrow().events().len(), 1);
        assert_eq!(
            history.borrow().events()[0].send_state,
            Some(DemoSendState::Pending)
        );
    }

    #[test]
    fn queued_native_confirmation_callbacks_cannot_open_or_cancel_replacements() {
        for (change, return_occurrence) in [
            "replace", "cancel", "consume", "body", "message", "room", "account", "removal",
        ]
        .into_iter()
        .flat_map(|change| [0, 1].map(|occurrence| (change, occurrence)))
        {
            let id = ScopedTimelineId {
                scope: RoomScope {
                    account_id: AccountId::new(71),
                    room_id: RoomId::new("!links:test"),
                },
                id: TimelineId::Server(EventId::new("$links")),
            };
            let text = "caffè https://example.invalid/exact https://example.invalid/exact";
            let sources = Rc::new(RefCell::new(MessageLinkRequests::default()));
            let confirmation = Rc::new(RefCell::new(LinkConfirmation::default()));
            let target = sources.borrow_mut().targets(&id, text)[0].clone();
            assert!(
                sources
                    .borrow_mut()
                    .request(&target, &mut confirmation.borrow_mut())
            );
            let captured = confirmation.borrow().capture().unwrap();
            let (cancel, open) =
                confirmation_callbacks(confirmation.clone(), sources.clone(), captured);
            let mut other = id.clone();
            match change {
                "replace" => {
                    assert!(
                        sources
                            .borrow_mut()
                            .request(&target, &mut confirmation.borrow_mut())
                    );
                }
                "cancel" => confirmation.borrow_mut().cancel(),
                "consume" => {
                    assert_eq!(open().unwrap().as_str(), target.url().as_str());
                }
                "body" => {
                    sources
                        .borrow_mut()
                        .targets(&id, &format!("changed {text}"));
                }
                "message" => {
                    other.id = TimelineId::Server(EventId::new("$other"));
                }
                "room" => {
                    other.scope.room_id = RoomId::new("!other:test");
                }
                "account" => {
                    other.scope.account_id = AccountId::new(72);
                }
                "removal" => sources.borrow_mut().clear(),
                _ => unreachable!(),
            }
            if other != id {
                sources.borrow_mut().targets(&other, text);
                sources.borrow_mut().retain_event(&other);
            }
            assert!(open().is_none(), "queued open accepted: {change}");
            assert!(!cancel(), "queued cancel accepted: {change}");
            // Reopen the same source after away/back, then drain captured callbacks.
            let fresh = sources.borrow_mut().targets(&id, text)[return_occurrence].clone();
            assert!(
                sources
                    .borrow_mut()
                    .request(&fresh, &mut confirmation.borrow_mut())
            );
            let (current_cancel, current_open) = confirmation_callbacks(
                confirmation.clone(),
                sources.clone(),
                confirmation.borrow().capture().unwrap(),
            );
            assert!(!cancel(), "queued cancel cleared new request: {change}");
            assert!(
                open().is_none(),
                "queued open consumed new request: {change}"
            );
            assert_eq!(current_open().unwrap().as_str(), fresh.url().as_str());
            assert!(current_open().is_none());
            assert!(!current_cancel());
            assert!(
                sources
                    .borrow_mut()
                    .request(&fresh, &mut confirmation.borrow_mut())
            );
            let (current_cancel, current_open) = confirmation_callbacks(
                confirmation.clone(),
                sources.clone(),
                confirmation.borrow().capture().unwrap(),
            );
            assert!(current_cancel());
            assert!(!current_cancel());
            assert!(current_open().is_none());
        }
    }
}
