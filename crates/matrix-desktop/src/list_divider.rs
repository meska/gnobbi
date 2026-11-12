//! Draggable edge between the room list and the chat: limits, clamp, drag and save.
use crate::{Shell, rail_view::RAIL_WIDTH, thread_view::CHAT_MIN_WIDTH};
use gpui::{Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window, div, prelude::FluentBuilder, px, rgb};

/// Provisional: measured with the six-button filter row (304); re-measure once the filter row cards land.
pub(crate) const LIST_MIN_WIDTH: f32 = 304.;
pub(crate) const LIST_MAX_WIDTH: f32 = 480.;
/// Grab area straddling the edge.
const GRAB_WIDTH: f32 = 8.;
/// Arrow keys and assistive-technology steps.
const KEY_STEP: f32 = 16.;

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

    /// Keyboard / assistive-technology move: set the width and save it, as a drag does on release.
    pub(crate) fn move_list_divider(&mut self, width: f32, window: &Window, cx: &mut Context<Self>) {
        self.set_list_width(Some(width), window, cx);
        self.save_list_width();
    }

    /// Invisible Tab stop placed in the tree between list and chat; it takes the arrow keys.
    pub(crate) fn render_list_divider_focus(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div().id("list-divider-focus").absolute().size_0().track_focus(&self.list_divider_focus).on_key_down(cx.listener(
            |this, e: &gpui::KeyDownEvent, window, cx| {
                let step = match e.keystroke.key.as_str() {
                    "left" => -KEY_STEP,
                    "right" => KEY_STEP,
                    _ => return,
                };
                this.move_list_divider(this.list_width(window) + step, window, cx);
                cx.stop_propagation();
            },
        ))
    }

    /// Snapshot for the accessibility tree: a splitter with the width in px.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn divider_snapshot(&self, window: &Window) -> matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot {
        let scale = f64::from(window.scale_factor());
        let x = f64::from(RAIL_WIDTH + self.list_width(window));
        let top = f64::from(f32::from(matrix_desktop::window::titlebar_inset(window.is_fullscreen())));
        let bottom = f64::from(f32::from(window.viewport_size().height));
        matrix_desktop::accessibility::scrollbars::ScrollbarSnapshot {
            pane: matrix_desktop::accessibility::scrollbars::Pane::Divider {
                min: f64::from(LIST_MIN_WIDTH),
                max: f64::from(LIST_MAX_WIDTH),
            },
            fraction: f64::from(self.list_width(window)),
            step: f64::from(KEY_STEP),
            bounds: accesskit::Rect::new((x - 4.) * scale, top * scale, (x + 4.) * scale, bottom * scale),
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
        let ring = crate::input_modality::ring(&self.list_divider_focus, window, cx);
        let accent = self.theme.tokens().accent;
        // Shared by the handle and the drag layer: a quick second press can land on the layer before the first release is handled.
        let press = move |this: &mut Shell, e: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Shell>| {
            window.focus(&this.list_divider_focus);
            if e.click_count >= 2 {
                this.list_drag = None;
                this.set_list_width(None, window, cx);
                this.save_list_width();
            } else {
                this.list_drag = Some((f32::from(e.position.x), width));
                cx.notify();
            }
            cx.stop_propagation();
        };
        // Save only if the drag changed the width (a plain click writes nothing).
        let end = |this: &mut Shell, _: &gpui::MouseUpEvent, window: &mut Window, cx: &mut Context<Shell>| {
            if let Some((_, w0)) = this.list_drag.take() {
                if this.list_width(window) != w0 {
                    this.save_list_width();
                }
                cx.notify();
            }
        };
        let handle = div()
            .id("list-divider")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(RAIL_WIDTH + width - GRAB_WIDTH / 2.))
            .w(px(GRAB_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .when(ring, |d| d.child(div().absolute().top_0().bottom_0().left(px((GRAB_WIDTH - 2.) / 2.)).w(px(2.)).bg(rgb(accent))))
            .on_mouse_down(MouseButton::Left, cx.listener(press))
            .on_mouse_up(MouseButton::Left, cx.listener(end));
        let layer = self.list_drag.map(|(x0, w0)| {
            div()
                .id("list-divider-drag")
                .absolute()
                .inset_0()
                .occlude()
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, cx.listener(press))
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

    /// Window width that leaves exactly `list` px for the list next to the rail and the minimum chat.
    fn viewport_for(list: f32) -> f32 {
        RAIL_WIDTH + list + CHAT_MIN_WIDTH
    }

    #[test]
    fn limits_are_exact_at_both_ends() {
        let wide = 2000.;
        assert_eq!(effective_width(LIST_MIN_WIDTH, wide), LIST_MIN_WIDTH);
        assert_eq!(effective_width(LIST_MIN_WIDTH - 0.5, wide), LIST_MIN_WIDTH);
        assert_eq!(effective_width(LIST_MAX_WIDTH, wide), LIST_MAX_WIDTH);
        assert_eq!(effective_width(LIST_MAX_WIDTH + 0.5, wide), LIST_MAX_WIDTH);
        assert_eq!(effective_width(f32::MAX, wide), LIST_MAX_WIDTH);
        assert_eq!(effective_width(0., wide), LIST_MIN_WIDTH);
        assert_eq!(effective_width(-50., wide), LIST_MIN_WIDTH);
    }

    #[test]
    fn chat_minimum_caps_the_list_and_the_list_minimum_beats_it() {
        // Window with exactly 400 px for the list: 480 is cut to 400, 304 stays.
        assert_eq!(effective_width(480., viewport_for(400.)), 400.);
        assert_eq!(effective_width(304., viewport_for(400.)), 304.);
        // One pixel less than the list minimum plus the chat minimum: the list does not shrink further.
        assert_eq!(effective_width(480., viewport_for(LIST_MIN_WIDTH) - 1.), LIST_MIN_WIDTH);
        assert_eq!(effective_width(480., viewport_for(LIST_MIN_WIDTH)), LIST_MIN_WIDTH);
        // Window wide enough for the maximum: the maximum, not more.
        assert_eq!(effective_width(480., viewport_for(LIST_MAX_WIDTH)), LIST_MAX_WIDTH);
        assert_eq!(effective_width(480., viewport_for(LIST_MAX_WIDTH) + 500.), LIST_MAX_WIDTH);
        // Degenerate windows (zero, negative, NaN-free tiny) never go below the minimum.
        assert_eq!(effective_width(400., 0.), LIST_MIN_WIDTH);
        assert_eq!(effective_width(400., -10.), LIST_MIN_WIDTH);
    }

    #[test]
    fn effective_width_is_idempotent_and_never_leaves_the_range() {
        for viewport in [0., 300., 700., 736., 800., 900., 1280., 1920., 2560., 5000.] {
            for stored in [0., 100., 303., 304., 320., 400., 479., 480., 481., 5000.] {
                let w = effective_width(stored, viewport);
                assert!((LIST_MIN_WIDTH..=LIST_MAX_WIDTH).contains(&w), "{stored} in {viewport}: {w}");
                assert_eq!(effective_width(w, viewport), w, "{stored} in {viewport}");
            }
        }
    }

    #[test]
    fn arrow_step_stops_exactly_on_the_limits() {
        let wide = 2000.;
        // Left/Right apply KEY_STEP to the current width and the same clamp as a drag.
        assert_eq!(effective_width(LIST_MIN_WIDTH - KEY_STEP, wide), LIST_MIN_WIDTH, "Left at the minimum");
        assert_eq!(effective_width(LIST_MAX_WIDTH + KEY_STEP, wide), LIST_MAX_WIDTH, "Right at the maximum");
        assert_eq!(effective_width(LIST_MIN_WIDTH + KEY_STEP, wide), LIST_MIN_WIDTH + KEY_STEP);
        assert_eq!(effective_width(LIST_MAX_WIDTH - KEY_STEP, wide), LIST_MAX_WIDTH - KEY_STEP);
        // A step that overshoots by less than a step lands on the limit, not before it.
        assert_eq!(effective_width(LIST_MAX_WIDTH - 8. + KEY_STEP, wide), LIST_MAX_WIDTH);
        assert_eq!(effective_width(LIST_MIN_WIDTH + 8. - KEY_STEP, wide), LIST_MIN_WIDTH);
        // In a window that only fits 368: Right from 368 stays at 368.
        assert_eq!(effective_width(368. + KEY_STEP, viewport_for(368.)), 368.);
        // Walking from the default to each end takes a whole number of steps and ends exactly on it.
        let mut w = 320.;
        let mut steps = 0;
        while w < LIST_MAX_WIDTH {
            w = effective_width(w + KEY_STEP, wide);
            steps += 1;
            assert!(steps < 100, "never reaches the maximum");
        }
        assert_eq!((w, steps), (LIST_MAX_WIDTH, 10));
        let mut steps = 0;
        while w > LIST_MIN_WIDTH {
            w = effective_width(w - KEY_STEP, wide);
            steps += 1;
            assert!(steps < 100, "never reaches the minimum");
        }
        assert_eq!((w, steps), (LIST_MIN_WIDTH, 11));
    }
}
