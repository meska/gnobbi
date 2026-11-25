//! Tapping rows of a scrolling list on touch. egui already opens a row only on release and only within `max_click_dist`
//! (set to `touch_menu::SLOP` in `apply_touch`); what it lacks, and this module adds:
//! - the finger that stops a fling is not a tap (`after` notes it, `tapped` and `pressed` honour it);
//! - the pressed look of a row shows only after `DELAY` with the finger still, so scrolling never flashes the rows (`pressed`).
//!
//! Every tappable row of a scrolling list uses `tapped`/`pressed` instead of `clicked`/`hovered`, and calls `after` on the area's output.
//! Desktop keeps `clicked` and `hovered` as they are.
use crate::mobile::touch;
use eframe::egui::{self, Context, Id, Ui, scroll_area::ScrollAreaOutput};

/// Seconds the finger rests on a row before it looks pressed.
pub const DELAY: f64 = 0.09;
/// Points per second above which a list counts as still flinging.
const FLING: f32 = 40.;
const STOPPED: &str = "touch-list-stopped";

/// egui's own tap distance on touch: the same slop as the long press.
pub fn apply_touch(ctx: &Context) {
    ctx.options_mut(|o| o.input_options.max_click_dist = crate::touch_menu::SLOP);
}

/// Call with the output of every list's `ScrollArea::show`: a press that lands on a list still flinging belongs to the list (it stops it).
pub fn after<R>(ctx: &Context, out: &ScrollAreaOutput<R>) {
    if !touch() {
        return;
    }
    let (frame, press) = (ctx.cumulative_pass_nr(), ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten()));
    let Some(_) = press.filter(|p| out.inner_rect.contains(*p)) else { return };
    let id = Id::new(STOPPED);
    // Nested areas: the innermost one that flings wins, an outer still one must not clear it.
    let was = ctx.data(|d| d.get_temp::<(u64, bool)>(id)).is_some_and(|(f, s)| f == frame && s);
    ctx.data_mut(|d| d.insert_temp(id, (frame, was || out.state.velocity().length() > FLING)));
}

/// Whether the press now going on (or just released) stopped a fling.
pub fn stopped(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<(u64, bool)>(Id::new(STOPPED))).is_some_and(|(_, s)| s)
}

/// A tap on the row: egui's click, unless that finger only stopped the list.
pub fn tapped(resp: &egui::Response) -> bool {
    resp.clicked() && !(touch() && stopped(&resp.ctx))
}

/// Whether the row shows as pressed. Desktop: under the pointer. Touch: finger down on it, still within the slop,
/// for `DELAY`, and not the one that stopped a fling.
pub fn pressed(ui: &Ui, resp: &egui::Response) -> bool {
    if !touch() {
        return resp.hovered();
    }
    let (origin, start, now, tap) = ui.input(|i| (i.pointer.press_origin(), i.pointer.press_start_time(), i.time, i.pointer.primary_down() && i.pointer.could_any_button_be_click()));
    let (Some(p), Some(t0), true) = (origin, start, tap) else { return false };
    if !resp.rect.contains(p) || !ui.clip_rect().contains(p) || stopped(ui.ctx()) {
        return false;
    }
    if now - t0 < DELAY {
        ui.ctx().request_repaint_after_secs((DELAY - (now - t0)) as f32);
        return false;
    }
    true
}

/// `ui.radio_value` for a list on touch: the choice is made by a tap, not by the finger that stopped a fling.
pub fn radio<T: PartialEq>(ui: &mut Ui, current: &mut T, value: T, text: &str) {
    let r = ui.radio(*current == value, text);
    if tapped(&r) {
        *current = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Sense, Vec2, pos2};

    struct Pad {
        ctx: Context,
        time: f64,
        tapped: bool,
        pressed: bool,
        offset: f32,
    }

    impl Pad {
        fn new() -> Self {
            crate::mobile::FORCE_TOUCH.set(true);
            let ctx = Context::default();
            apply_touch(&ctx);
            let mut pad = Self { ctx, time: 1., tapped: false, pressed: false, offset: 0. };
            pad.wait(0.2); // the list settles over its first frames
            pad
        }

        fn frame(&mut self, dt: f64, events: Vec<Event>) {
            self.time += dt;
            let input = RawInput { time: Some(self.time), screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(300., 400.))), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| { egui::CentralPanel::default().show_inside(ui, |ui| {
                let area = egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 40., 100, |ui, range| {
                    for _ in range {
                        let (_, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), Sense::click());
                        self.tapped |= tapped(&resp);
                        self.pressed |= pressed(ui, &resp);
                    }
                });
                after(ui.ctx(), &area);
                self.offset = area.state.clamped_offset().y;
            }); });
            out.textures_delta.clear();
        }

        fn down(&mut self, p: Pos2) {
            let touch = Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(0), phase: egui::TouchPhase::End, pos: p, force: None };
            self.frame(0.016, vec![touch, Event::PointerMoved(p)]);
            self.frame(0.016, vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        }

        fn up(&mut self, p: Pos2) {
            self.frame(0.016, vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            self.frame(0.016, vec![Event::PointerGone]);
            self.frame(0.016, vec![]);
        }

        fn wait(&mut self, secs: f64) {
            for _ in 0..(secs / 0.016) as usize {
                self.frame(0.016, vec![]);
            }
        }

        fn drag(&mut self, from: Pos2, to: Pos2, steps: usize) {
            for k in 1..=steps {
                let p = from + (to - from) * (k as f32 / steps as f32);
                self.frame(0.016, vec![Event::PointerMoved(p)]);
            }
        }
    }

    #[test]
    fn still_tap_opens_on_release_with_a_delayed_look() {
        let mut pad = Pad::new();
        let p = pos2(100., 100.);
        pad.down(p);
        assert!(!pad.pressed, "no look right at the press");
        pad.wait(0.03);
        assert!(!pad.pressed && !pad.tapped);
        pad.wait(0.1);
        assert!(pad.pressed, "look after the delay");
        assert!(!pad.tapped, "nothing opens before the release");
        pad.up(p);
        assert!(pad.tapped);
    }

    #[test]
    fn small_wobble_is_still_a_tap_and_a_drag_is_not() {
        let mut pad = Pad::new();
        let p = pos2(100., 100.);
        pad.down(p);
        pad.drag(p, p + Vec2::new(0., 8.), 2);
        pad.up(p + Vec2::new(0., 8.));
        assert!(pad.tapped, "8 pt is within the slop");

        let mut pad = Pad::new();
        pad.down(p);
        pad.drag(p, p - Vec2::new(0., 20.), 4);
        pad.wait(0.2);
        pad.up(p - Vec2::new(0., 20.));
        assert!(!pad.tapped, "20 pt is a scroll");
        assert!(!pad.pressed, "rows do not flash while scrolling");
        assert!(pad.offset > 0.);
    }

    #[test]
    fn touch_that_stops_a_fling_neither_taps_nor_looks_pressed() {
        let mut pad = Pad::new();
        let p = pos2(100., 300.);
        pad.down(p);
        pad.drag(p, pos2(100., 60.), 8);
        pad.up(pos2(100., 60.));
        let flying = pad.offset;
        assert!(flying > 0.);
        pad.frame(0.016, vec![]);
        assert!(pad.offset > flying, "the list is still flinging");
        pad.tapped = false;
        pad.pressed = false;
        let q = pos2(100., 100.);
        pad.down(q);
        pad.wait(0.2);
        let held = pad.offset;
        pad.wait(0.1);
        assert_eq!(pad.offset, held, "the finger stops the list");
        pad.up(q);
        assert!(!pad.tapped && !pad.pressed);
        // The next tap, on the resting list, works.
        pad.down(q);
        pad.wait(0.15);
        pad.up(q);
        assert!(pad.tapped && pad.pressed);
    }
}
