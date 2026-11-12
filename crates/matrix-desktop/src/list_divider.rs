//! Draggable edge between the room list and the chat: limits, clamp, drag and save.
use crate::{Shell, rail_view::RAIL_WIDTH, thread_view::CHAT_MIN_WIDTH};
use gpui::{Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, Styled, Window, div, px};

/// Provisional: measured with the six-button filter row (304); re-measure once the filter row cards land.
pub(crate) const LIST_MIN_WIDTH: f32 = 304.;
pub(crate) const LIST_MAX_WIDTH: f32 = 480.;
/// Grab area straddling the edge.
const GRAB_WIDTH: f32 = 8.;

/// Width to paint: the saved one inside the limits for this window. The list minimum wins over the chat minimum.
pub(crate) fn effective_width(stored: f32, viewport: f32) -> f32 {
    stored.clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH.min(viewport - RAIL_WIDTH - CHAT_MIN_WIDTH).max(LIST_MIN_WIDTH))
}

impl Shell {
    /// Current list width in this window (not the saved value when the window is narrow).
    pub(crate) fn list_width(&self, window: &Window) -> f32 {
        effective_width(self.prefs.list_width.map_or(self.theme.tokens().sidebar, |w| w as f32), window.viewport_size().width.into())
    }

    /// The one place that changes the width (drag, double click, a future keyboard «move by N px»).
    pub(crate) fn set_list_width(&mut self, width: Option<f32>, window: &Window, cx: &mut Context<Self>) {
        let width = width.map(|w| effective_width(w, window.viewport_size().width.into()).round() as u32);
        if self.prefs.list_width != width {
            self.prefs.list_width = width;
            cx.notify();
        }
    }

    fn save_list_width(&self) {
        if let Some(dirs) = &self.prefs_dirs {
            let width = self.prefs.list_width;
            if let Err(error) = matrix_desktop::Preferences::update(dirs, |p| p.list_width = width) {
                eprintln!("Gnobbi: cannot save list width: {error}");
            }
        }
    }

    /// Grab handle on top of the edge, plus a full-window layer while dragging that keeps the cursor and the events.
    pub(crate) fn render_list_divider(&self, window: &Window, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let width = self.list_width(window);
        let handle = div()
            .id("list-divider")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(RAIL_WIDTH + width - GRAB_WIDTH / 2.))
            .w(px(GRAB_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                if e.click_count >= 2 {
                    this.list_drag = None;
                    this.set_list_width(None, window, cx);
                    this.save_list_width();
                } else {
                    this.list_drag = Some((f32::from(e.position.x), width));
                    cx.notify();
                }
                cx.stop_propagation();
            }));
        let end = |this: &mut Shell, _: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Shell>| {
            if this.list_drag.take().is_some() {
                this.save_list_width();
                cx.notify();
            }
        };
        let layer = self.list_drag.map(|(x0, w0)| {
            div()
                .id("list-divider-drag")
                .absolute()
                .inset_0()
                .occlude()
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, window, cx| {
                    this.set_list_width(Some(w0 + f32::from(e.position.x) - x0), window, cx);
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(end))
                .on_mouse_up_out(MouseButton::Left, cx.listener(end))
        });
        std::iter::once(handle.into_any_element()).chain(layer.map(IntoElement::into_any_element)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_stays_inside_limits_and_list_minimum_wins() {
        assert_eq!(effective_width(320., 1400.), 320.);
        assert_eq!(effective_width(100., 1400.), LIST_MIN_WIDTH);
        assert_eq!(effective_width(900., 1400.), LIST_MAX_WIDTH);
        // 900 wide: room left for the list is 900-72-360 = 468
        assert_eq!(effective_width(480., 900.), 468.);
        assert_eq!(effective_width(400., 800.), 368.);
        // narrower than rail + min list + min chat: the list minimum wins
        assert_eq!(effective_width(400., 700.), LIST_MIN_WIDTH);
        assert_eq!(effective_width(400., 300.), LIST_MIN_WIDTH);
    }
}
