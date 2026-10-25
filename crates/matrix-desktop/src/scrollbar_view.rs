//! Shared native overlay. Metrics are read after the list's prepaint, never guessed
//! from row counts; GPUI retains variable-height estimates and freezes drag extent.
use gpui::{prelude::*, *};
use matrix_desktop::{DesignTokens, ScrollbarGeometry, VirtualScrollExtent};
use std::{cell::RefCell, rc::Rc, time::Instant};

pub type HistoryExtent = Rc<RefCell<(usize, f32, VirtualScrollExtent)>>;

#[derive(Clone)]
pub enum ScrollTarget {
    Rooms(UniformListScrollHandle),
    History(ListState, HistoryExtent),
}
impl ScrollTarget {
    fn metrics(&self) -> (f32, f32, f32) {
        match self {
            Self::Rooms(handle) => {
                let base = &handle.0.borrow().base_handle;
                let viewport = f32::from(base.bounds().size.height);
                (
                    viewport,
                    viewport + f32::from(base.max_offset().height),
                    -f32::from(base.offset().y),
                )
            }
            Self::History(state, extent) => {
                let viewport = f32::from(state.viewport_bounds().size.height);
                let extent = extent.borrow();
                let total = extent.2.total_height();
                let top = state.logical_scroll_top();
                let offset = extent
                    .2
                    .offset_for_item(top.item_ix, f32::from(top.offset_in_item));
                (viewport, total, offset.min((total - viewport).max(0.)))
            }
        }
    }
    // A jump based on estimated heights can land beyond the subsequently
    // measured height of its logical top item. Retire that invisible anchor
    // before a resize reflows it; preserve the same painted position instead.
    fn normalize_anchor(&self) -> bool {
        let Self::History(state, extent) = self else {
            return false;
        };
        let top = state.logical_scroll_top();
        if top.item_ix >= state.item_count() || top.offset_in_item < px(0.) {
            return false;
        }
        let extent = extent.borrow();
        let offset = extent
            .2
            .offset_for_item(top.item_ix, f32::from(top.offset_in_item));
        let (item_ix, offset_in_item) = extent.2.item_for_offset(offset);
        if item_ix <= top.item_ix {
            return false;
        }
        state.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(offset_in_item),
        });
        true
    }

    fn drag_snapshot(&self) -> Self {
        match self {
            Self::Rooms(_) => self.clone(),
            Self::History(state, extent) => {
                let extent = extent.borrow();
                Self::History(
                    state.clone(),
                    Rc::new(RefCell::new((extent.0, extent.1, extent.2.clone()))),
                )
            }
        }
    }
    fn set_offset(&self, offset: f32) {
        match self {
            Self::Rooms(handle) => handle
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), px(-offset))),
            Self::History(state, extent) => {
                let extent = extent.borrow();
                let max = (extent.2.total_height()
                    - f32::from(state.viewport_bounds().size.height))
                .max(0.);
                let (item_ix, offset_in_item) = if offset >= max {
                    (state.item_count(), 0.)
                } else {
                    extent.2.item_for_offset(offset)
                };
                state.scroll_to(ListOffset {
                    item_ix,
                    offset_in_item: px(offset_in_item),
                });
            }
        }
    }
}

#[derive(Default)]
struct State {
    bounds: Bounds<Pixels>,
    geometry: Option<ScrollbarGeometry>,
    drag: Option<(f32, ScrollbarGeometry, ScrollTarget)>,
    hovered: bool,
    activity: Option<Instant>,
    last_metrics: Option<(f32, f32, f32)>,
}
#[derive(Clone, Default)]
pub struct Scrollbar(Rc<RefCell<State>>);
impl Scrollbar {
    /// Window position of the middle of the painted thumb.
    #[cfg(test)]
    pub(crate) fn thumb_centre(&self) -> Option<Point<Pixels>> {
        let state = self.0.borrow();
        let g = state.geometry?;
        Some(point(
            state.bounds.left() + px(7.),
            state.bounds.top() + px(g.thumb_top + g.thumb_length / 2.),
        ))
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn semantic_snapshot(
        &self,
        target: ScrollTarget,
        pane: matrix_desktop::accessibility::scrollbars::Pane,
        scale: f64,
    ) -> Option<matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot> {
        let (viewport, content, offset) = target.metrics();
        ScrollbarGeometry::new(viewport, content, offset)?;
        let bounds = self.0.borrow().bounds;
        if bounds.size.height <= px(0.) {
            return None;
        }
        Some(
            matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot {
                pane,
                fraction: f64::from((offset / (content - viewport)).clamp(0., 1.)),
                step: f64::from((viewport / (content - viewport)).min(1.)),
                bounds: accesskit::Rect::new(
                    f64::from(f32::from(bounds.left())) * scale,
                    f64::from(f32::from(bounds.top())) * scale,
                    f64::from(f32::from(bounds.right())) * scale,
                    f64::from(f32::from(bounds.bottom())) * scale,
                ),
            },
        )
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn set_fraction(&self, target: ScrollTarget, fraction: f64) {
        let (viewport, content, _) = target.metrics();
        if fraction.is_finite() && content > viewport {
            target.set_offset((content - viewport) * fraction.clamp(0., 1.) as f32);
            self.0.borrow_mut().activity = Some(Instant::now());
        }
    }

    pub fn render(
        &self,
        id: &'static str,
        target: ScrollTarget,
        t: DesignTokens,
    ) -> impl IntoElement {
        let state = self.0.clone();
        let paint_state = state.clone();
        let paint_target = target.clone();
        let prepaint_state = paint_state.clone();
        let hover_state = state.clone();
        div()
            .id(id)
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(14.))
            .on_hover(move |hovered, window, _| {
                let mut state = hover_state.borrow_mut();
                state.hovered = *hovered;
                state.activity = Some(Instant::now());
                window.refresh();
            })
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let mut state = state.borrow_mut();
                if let Some(g) = state.geometry {
                    let y = f32::from(event.position.y - state.bounds.top());
                    let grab = if y >= g.thumb_top && y <= g.thumb_top + g.thumb_length {
                        y - g.thumb_top
                    } else {
                        g.thumb_length / 2.
                    };
                    let target = target.drag_snapshot();
                    target.set_offset(g.offset_for_thumb(y - grab));
                    state.drag = Some((grab, g, target));
                    window.refresh();
                    cx.stop_propagation();
                }
            })
            .child(
                canvas(
                    move |bounds, window, _| {
                        if paint_target.normalize_anchor() {
                            window.on_next_frame(|window, _| window.refresh());
                        }
                        let (viewport, content, offset) = paint_target.metrics();
                        let geometry = ScrollbarGeometry::new(viewport, content, offset);
                        let mut state = prepaint_state.borrow_mut();
                        state.bounds = bounds;
                        state.geometry = geometry;
                        let metrics = (viewport, content, offset);
                        if state.last_metrics != Some(metrics) {
                            state.last_metrics = Some(metrics);
                            state.activity = Some(Instant::now());
                        }
                        let elapsed = state.activity.map_or(2., |at| at.elapsed().as_secs_f32());
                        let opacity = if state.hovered || state.drag.is_some() {
                            1.
                        } else {
                            ((1.1 - elapsed) / 0.3).clamp(0., 1.)
                        };
                        let animate = opacity > 0. && !state.hovered && state.drag.is_none();
                        (bounds, geometry, opacity, animate)
                    },
                    move |_, (bounds, geometry, opacity, animate), window, _| {
                        if animate {
                            window.on_next_frame(|window, _| window.refresh());
                        }
                        let moving = paint_state.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture {
                                let state = moving.borrow();
                                if let Some((grab, g, target)) = &state.drag {
                                    let y = f32::from(event.position.y - state.bounds.top());
                                    target.set_offset(g.offset_for_thumb(y - grab));
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let release = paint_state.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture
                                && event.button == MouseButton::Left
                                && release.borrow_mut().drag.take().is_some()
                            {
                                window.refresh();
                                cx.stop_propagation();
                            }
                        });
                        if let Some(g) = geometry {
                            let thumb = Bounds::new(
                                point(bounds.left() + px(3.), bounds.top() + px(g.thumb_top)),
                                size(px(8.), px(g.thumb_length)),
                            );
                            window.paint_quad(fill(
                                thumb,
                                Rgba {
                                    a: opacity,
                                    ..rgb(t.secondary)
                                },
                            ));
                        }
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
impl crate::Shell {
    pub(crate) fn semantic_scrollbars(
        &self,
        window: &Window,
    ) -> Vec<matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot> {
        use matrix_desktop::accessibility::scrollbars::Pane;
        if self.notifications_open
            || self.verification_open
            || self.room_details_open
            || self.operations.borrow().selection().is_some()
        {
            return vec![];
        }
        let compact = window.viewport_size().width < px(800.);
        let mut bars = vec![];
        if (!compact || !self.compact_chat)
            && let Some(bar) = self.room_scrollbar.semantic_snapshot(
                ScrollTarget::Rooms(self.scroll.clone()),
                Pane::Rooms(
                    self.sidebar
                        .visible_indices()
                        .iter()
                        .map(|&i| self.sidebar.rooms()[i].scope.clone())
                        .collect(),
                ),
                f64::from(window.scale_factor()),
            )
        {
            bars.push(bar);
        }
        if (!compact || self.compact_chat)
            && let Some(bar) = self.timeline.as_ref().and_then(|timeline| {
                timeline
                    .scrollbar_snapshot(self.active_scope.clone(), f64::from(window.scale_factor()))
            })
        {
            bars.push(bar);
        }
        bars
    }

    pub(crate) fn apply_scrollbar(
        &self,
        action: matrix_desktop::accessibility::scrollbars::ScrollbarAction,
        window: &Window,
    ) {
        use matrix_desktop::accessibility::scrollbars::Pane;
        if !self
            .semantic_scrollbars(window)
            .iter()
            .any(|bar| bar.pane == action.pane)
        {
            return;
        }
        match action.pane {
            Pane::Rooms(_) => self
                .room_scrollbar
                .set_fraction(ScrollTarget::Rooms(self.scroll.clone()), action.fraction),
            Pane::History(_) => {
                if let Some(timeline) = &self.timeline {
                    timeline.set_scroll_fraction(action.fraction);
                }
            }
        }
    }
}
