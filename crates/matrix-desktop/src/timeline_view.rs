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

const SENDER_AVATAR_PX: f32 = 32.;

pub struct TimelineView {
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
    link_hover: Rc<Cell<Option<u64>>>,
    viewing: Rc<RefCell<Option<matrix_domain::EventId>>>,
    /// Full-resolution picture of the open large view (loading, ready or failed).
    pub(crate) original:
        Rc<RefCell<Option<(matrix_domain::EventId, crate::media_view::InlineImage)>>>,
    /// Set by a click or «Riprova»; the shell turns it into the original read.
    pub(crate) want_original: Rc<RefCell<Option<matrix_domain::EventId>>>,
    link_menu: Rc<RefCell<matrix_desktop::LinkMenu>>,
    link_menu_at: Rc<Cell<gpui::Point<Pixels>>>,
    /// Last painted bounds of each video bubble part, by event and part name.
    pub(crate) video_painted: Rc<RefCell<VideoParts>>,
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
            link_hover: Rc::default(),
            viewing: Rc::default(),
            original: Rc::default(),
            want_original: Rc::default(),
            link_menu: Rc::default(),
            link_menu_at: Rc::default(),
            images: Rc::default(),
            sender_pictures: Rc::default(),
            video_painted: Rc::default(),
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
        self.requested_link.replace(None);
        self.link.borrow_mut().cancel();
        self.selection_cursor.replace(None);
        self.retry_cursor.replace(None);
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
        let link_hover = self.link_hover.clone();
        let link_menu = self.link_menu.clone();
        let link_menu_at = self.link_menu_at.clone();
        let images = self.images.clone();
        let sender_pictures = self.sender_pictures.clone();
        let video_painted = self.video_painted.clone();
        let viewing = self.viewing.clone();
        let (original, want_original) = (self.original.clone(), self.want_original.clone());
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
                    let sender_label = history.sender_label(index, members);
                    let sender_avatar = history.sender_avatar(index, members).map(|(mxc, initials)| {
                        match mxc.and_then(|m| sender_pictures.borrow().get(m).cloned()) {
                            Some(image) => crate::avatar_view::round(image, SENDER_AVATAR_PX),
                            None => div().flex().items_center().justify_center().size(px(SENDER_AVATAR_PX)).flex_shrink_0().rounded_full().bg(rgb(0x5288b5)).text_color(rgb(t.avatar_text)).text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).child(initials).into_any_element(),
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
                        crate::media_view::video_picture(state, facts.duration.clone(), facts.size.clone(), t, std::rc::Rc::new(move |part, bounds| { parts.borrow_mut().insert((id.id.clone(), part), bounds); }))
                    });
                    let picture = video_picture.or_else(|| matrix_engine::history::is_image_body(&event.text).then(|| {
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
                    let mut body = StyledText::new(event.text.clone());
                    let links = message_links(&event.text);
                    let targets = link_sources.borrow_mut().targets(&event.id, &event.text);
                    let selected = selection.borrow().range(&event.id, &event.text);
                    let mut cuts = vec![0, event.text.len()];
                    // Only spans that fit the text: a stale range must never slice inside a character.
                    let styles: Vec<_> = event.styles.iter().filter(|s| s.range.start < s.range.end && s.range.end <= event.text.len() && event.text.is_char_boundary(s.range.start) && event.text.is_char_boundary(s.range.end)).collect();
                    for style in &styles { cuts.extend([style.range.start, style.range.end]); }
                    for link in &links { cuts.extend([link.range.start, link.range.end]); }
                    if let Some(range) = &selected { cuts.extend([range.start, range.end]); }
                    cuts.sort_unstable();
                    cuts.dedup();
                    body = body.with_highlights(cuts.windows(2).map(|pair| {
                        let range = pair[0]..pair[1];
                        let is_link = links.iter().any(|link| link.range.contains(&range.start));
                        use matrix_engine::rich_text::Style;
                        let has = |kind| styles.iter().any(|s| s.style == kind && s.range.contains(&range.start));
                        let style = HighlightStyle {
                            font_weight: has(Style::Bold).then_some(gpui::FontWeight::BOLD),
                            font_style: has(Style::Italic).then_some(gpui::FontStyle::Italic),
                            strikethrough: has(Style::Strike).then(|| gpui::StrikethroughStyle { thickness: px(1.), color: None }),
                            color: if is_link { Some(rgb(t.accent).into()) } else { has(Style::Quote).then(|| rgb(t.secondary).into()) },
                            underline: is_link.then(|| gpui::UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                            background_color: if selected.as_ref().is_some_and(|r| r.contains(&range.start))
                                || targets.iter().any(|l| link_cursor.get() == Some(l.instance()) && l.range().contains(&range.start)) { Some(rgb(t.selection).into()) } else { has(Style::Code).then(|| rgb(t.input).into()) },
                            ..Default::default()
                        };
                        (range, style)
                    }));
                    let down_layout = body.layout().clone();
                    let move_layout = body.layout().clone();
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
                    let up_layout = body.layout().clone();
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
                    let menu_layout = body.layout().clone();
                    let menu_targets = targets.clone();
                    let menu_state = link_menu.clone();
                    let menu_at = link_menu_at.clone();
                    let paint_targets = targets.clone();
                    let paint_layout = body.layout().clone();
                    let paint_links = row_link_painted.clone();
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
                                .bg(rgb(if event.outgoing { t.outgoing } else { t.surface }))
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
                                .when_some(sender_label, |d, name| d.child(
                                    div().id(("sender-name", index)).mb_1().text_size(px(12.)).line_height(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.accent)).child(name)))
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
                                            && up_layout.index_for_position(e.position) == Ok(start)
                                            && let Some(target) = up_targets.iter().find(|target| target.range().contains(&start))
                                            && up_sources.borrow_mut().request(target, &mut up_link.borrow_mut()) {
                                                up_requested.replace(Some(target.clone()));
                                                window.refresh();
                                        }
                                    })
                                    .relative()
                                    .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
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
                                .child(
                                    div()
                                        .w_full()
                                        .text_right()
                                        .whitespace_normal()
                                        .mt_1()
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
                                )
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
                                .when_some(event.send_state.filter(|s| !live || *s == DemoSendState::Failed), |d, status| d.child(
                                    div().flex().justify_end().h(px(24.)).items_center()
                                        .child(div().id(("demo-delivery", index))
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
                                )),
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
            .children(self.viewing().and_then(|id| match self.images.borrow().get(&id) {
                Some(crate::media_view::InlineImage::Ready(image)) => {
                    let (viewing, focus) = (self.viewing.clone(), self.focus.clone());
                    let original = self.original.borrow().clone().filter(|(o, _)| *o == id).map(|(_, state)| state);
                    let (retry_original, retry_want, retry_id) = (self.original.clone(), self.want_original.clone(), id.clone());
                    Some(crate::media_view::large_view(image.clone(), original, t, move |window| {
                        close_viewer(&viewing, &focus, "escape", window);
                        window.refresh();
                    }, move |window| {
                        *retry_original.borrow_mut() = None;
                        *retry_want.borrow_mut() = Some(retry_id.clone());
                        window.refresh();
                    }))
                }
                _ => None,
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
                            event_id: EventId::new(format!("$old{n}:fixture.invalid")),
                            body: format!("older {n}"),
                            outgoing: false,
                            timestamp_ms: n,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(),
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
                            event_id: EventId::new(format!("$old{n}:fixture.invalid")),
                            body: format!("older {n}"),
                            outgoing: false,
                            timestamp_ms: n,
                            sender: HistorySender::new("@a:fixture.invalid", None),
                            video: None,
                            styles: Vec::new(),
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
                            styles: Vec::new(),
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

    fn ready_image() -> crate::media_view::InlineImage {
        let buffer = image::RgbaImage::from_raw(2, 2, vec![255; 16]).unwrap();
        crate::media_view::InlineImage::Ready(std::sync::Arc::new(gpui::RenderImage::new([
            image::Frame::new(buffer),
        ])))
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
