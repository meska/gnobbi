//! Tapping rows of a scrolling list on touch. egui already opens a row only on release and only within `max_click_dist`
//! (set to `touch_menu::SLOP` in `apply_touch`); what it lacks, and this module adds:
//! - the finger that stops a fling is not a tap (`after` notes it, `tapped` and `pressed` honour it);
//! - the pressed look of a row shows only after `DELAY` with the finger still, so scrolling never flashes the rows (`pressed`).
//!
//! Every tappable row of a scrolling list uses `tapped`/`pressed` instead of `clicked`/`hovered`, and calls `after` on the area's output.
//! Desktop keeps `clicked` and `hovered` as they are. A touch build driven by a mouse or trackpad (iPad, Android) shows no hover on rows: `pressed` is a finger-down look only.
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
    let (start, now, tap) = ui.input(|i| (i.pointer.press_start_time(), i.time, i.pointer.could_any_button_be_click()));
    let (Some(t0), true) = (start, tap) else { return false };
    // egui's own "the press began on this widget": it knows the layers (a window over the row) and the clip.
    if !resp.is_pointer_button_down_on() || stopped(ui.ctx()) {
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

/// A scrolling list under a simulated finger, on egui's own clock (no real sleeps): shared by the tests of every touch list.
#[cfg(test)]
pub(crate) mod pad {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Sense, Vec2};
    use std::{cell::Cell, ops::Range, rc::Rc};

    /// What the rows reported, across frames.
    #[derive(Default)]
    pub struct Seen {
        pub tapped: Cell<bool>,
        pub pressed: Cell<bool>,
        pub menu: Cell<bool>,
    }

    pub const ROW_H: f32 = 40.;

    pub struct Pad {
        pub ctx: Context,
        pub seen: Rc<Seen>,
        pub offset: f32,
        time: f64,
        rows: Box<dyn FnMut(&mut Ui, Range<usize>, &Seen)>,
    }

    impl Pad {
        /// `rows` draws the visible rows of a 100-row list, 40 pt each, 300x400 screen.
        pub fn new(rows: impl FnMut(&mut Ui, Range<usize>, &Seen) + 'static) -> Self {
            crate::mobile::FORCE_TOUCH.set(true);
            let ctx = crate::icons::test_context();
            apply_touch(&ctx);
            let mut pad = Self { ctx, seen: Rc::default(), offset: 0., time: 1., rows: Box::new(rows) };
            pad.wait(0.2); // the list settles over its first frames
            pad
        }

        /// Plain rows: `tapped` and `pressed` as every list calls them.
        pub fn bare() -> Self {
            Self::new(|ui, range, seen| {
                for _ in range {
                    let (_, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                    seen.tapped.set(seen.tapped.get() | tapped(&resp));
                    seen.pressed.set(seen.pressed.get() | pressed(ui, &resp));
                }
            })
        }

        pub fn clear(&self) {
            self.seen.tapped.set(false);
            self.seen.pressed.set(false);
            self.seen.menu.set(false);
        }

        pub fn frame(&mut self, dt: f64, events: Vec<Event>) {
            self.time += dt;
            let input = RawInput { time: Some(self.time), screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(300., 400.))), events, ..Default::default() };
            let (rows, seen) = (&mut self.rows, &self.seen);
            let mut out = self.ctx.run_ui(input, |ui| { egui::CentralPanel::default().show_inside(ui, |ui| {
                let area = egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, 100, |ui, range| rows(ui, range, seen));
                after(ui.ctx(), &area);
                self.offset = area.state.clamped_offset().y;
            }); });
            out.textures_delta.clear();
        }

        fn touch(&self, phase: egui::TouchPhase, pos: Pos2) -> Event {
            Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(0), phase, pos, force: None }
        }

        pub fn down(&mut self, p: Pos2) {
            // A real finger: the touch is active while it is down, so egui's own long press (`secondary_clicked` after 0.8 s) runs here too.
            self.frame(0.016, vec![self.touch(egui::TouchPhase::Start, p), Event::PointerMoved(p)]);
            self.frame(0.016, vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        }

        pub fn up(&mut self, p: Pos2) {
            self.frame(0.016, vec![self.touch(egui::TouchPhase::End, p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            self.frame(0.016, vec![Event::PointerGone]);
            self.frame(0.016, vec![]);
        }

        pub fn wait(&mut self, secs: f64) {
            for _ in 0..(secs / 0.016) as usize {
                self.frame(0.016, vec![]);
            }
        }

        pub fn drag(&mut self, from: Pos2, to: Pos2, steps: usize) {
            for k in 1..=steps {
                let p = from + (to - from) * (k as f32 / steps as f32);
                self.frame(0.016, vec![self.touch(egui::TouchPhase::Move, p), Event::PointerMoved(p)]);
            }
        }

        /// A quick swipe up that leaves the list flinging; the next frame still scrolls.
        pub fn fling(&mut self) {
            let from = Pos2::new(100., 300.);
            self.down(from);
            self.drag(from, Pos2::new(100., 60.), 8);
            self.up(Pos2::new(100., 60.));
            let flying = self.offset;
            self.frame(0.016, vec![]);
            assert!(self.offset > flying, "the list is still flinging");
            self.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pad::*;
    use super::*;
    use egui::{Vec2, pos2};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn dragging_20_pt_from_a_row_scrolls_and_opens_nothing() {
        let mut pad = Pad::bare();
        let p = pos2(100., 300.);
        pad.down(p);
        pad.drag(p, p - Vec2::new(0., 20.), 4);
        pad.wait(0.2);
        pad.up(p - Vec2::new(0., 20.));
        assert!(!pad.seen.tapped.get(), "20 pt is a scroll, not a tap");
        assert!(pad.offset > 0., "the list scrolled");
    }

    #[test]
    fn still_tap_opens_at_the_release_not_before() {
        let mut pad = Pad::bare();
        let p = pos2(100., 100.);
        pad.down(p);
        pad.wait(0.3);
        assert!(!pad.seen.tapped.get(), "nothing opens while the finger is down");
        pad.up(p);
        assert!(pad.seen.tapped.get());
    }

    #[test]
    fn touch_that_stops_a_fling_stops_it_and_opens_nothing() {
        let mut pad = Pad::bare();
        pad.fling();
        let q = pos2(100., 100.);
        pad.down(q);
        pad.wait(0.2);
        let held = pad.offset;
        pad.wait(0.1);
        assert_eq!(pad.offset, held, "the finger stops the list");
        pad.up(q);
        assert!(!pad.seen.tapped.get() && !pad.seen.pressed.get(), "neither a tap nor a pressed look");
        // The next tap, on the resting list, works.
        pad.down(q);
        pad.wait(0.15);
        pad.up(q);
        assert!(pad.seen.tapped.get() && pad.seen.pressed.get());
    }

    #[test]
    fn movement_under_the_threshold_still_opens() {
        for d in [4., 5., 9.] {
            let mut pad = Pad::bare();
            let p = pos2(100., 100.);
            pad.down(p);
            pad.drag(p, p + Vec2::new(0., d), 2);
            pad.up(p + Vec2::new(0., d));
            assert!(pad.seen.tapped.get(), "{d} pt is within the slop");
        }
        let mut pad = Pad::bare();
        let p = pos2(100., 100.);
        pad.down(p);
        pad.drag(p, p + Vec2::new(0., crate::touch_menu::SLOP + 2.), 2);
        pad.up(p + Vec2::new(0., crate::touch_menu::SLOP + 2.));
        assert!(!pad.seen.tapped.get(), "just past the slop is a drag");
    }

    #[test]
    fn apply_touch_sets_egui_tap_distance_to_the_long_press_slop() {
        let ctx = Context::default();
        apply_touch(&ctx);
        assert_eq!(ctx.options(|o| o.input_options.max_click_dist), crate::touch_menu::SLOP);
    }

    #[test]
    fn pressed_look_waits_for_the_delay() {
        let mut pad = Pad::bare();
        let p = pos2(100., 100.);
        pad.down(p);
        pad.wait(DELAY - 0.04);
        assert!(!pad.seen.pressed.get(), "no look before the delay");
        pad.wait(0.08);
        assert!(pad.seen.pressed.get(), "look once the finger has rested");
    }

    #[test]
    fn pressed_look_never_shows_when_the_finger_moves_and_goes_when_it_starts_to() {
        let mut pad = Pad::bare();
        let p = pos2(100., 300.);
        pad.down(p);
        pad.drag(p, p - Vec2::new(0., 20.), 4); // moving from the start
        pad.wait(0.3);
        assert!(!pad.seen.pressed.get(), "a scroll never flashes the row");

        let mut pad = Pad::bare();
        pad.down(p);
        pad.wait(0.15);
        assert!(pad.seen.pressed.get());
        pad.drag(p, p - Vec2::new(0., 30.), 3); // the first step is still inside the slop
        pad.seen.pressed.set(false);
        pad.wait(0.05);
        assert!(!pad.seen.pressed.get(), "the look goes as soon as the finger leaves the slop");
    }

    #[test]
    fn stopped_is_false_until_a_fling_is_caught_and_clears_on_the_next_press() {
        let mut pad = Pad::bare();
        assert!(!stopped(&pad.ctx));
        pad.fling();
        pad.down(pos2(100., 100.));
        assert!(stopped(&pad.ctx), "the press landed on a flinging list");
        pad.up(pos2(100., 100.));
        pad.wait(0.5);
        pad.down(pos2(100., 100.));
        assert!(!stopped(&pad.ctx), "the list was at rest");
    }

    /// Rows of exactly one list step, so a tap at x=20 hits the radio of the row under it.
    fn radio_pad() -> (Pad, Rc<Cell<u8>>) {
        let chosen = Rc::new(Cell::new(0u8));
        let c = chosen.clone();
        let pad = Pad::new(move |ui, range, _| {
            ui.spacing_mut().item_spacing.y = 0.;
            ui.spacing_mut().interact_size.y = ROW_H;
            for i in range {
                let mut cur = c.get();
                radio(ui, &mut cur, i as u8 + 1, "x");
                c.set(cur);
            }
        });
        (pad, chosen)
    }

    #[test]
    fn radio_is_chosen_by_a_tap_not_by_a_scroll_or_a_catch() {
        let (mut pad, chosen) = radio_pad();
        let p = pos2(20., 300.);
        pad.down(p);
        pad.drag(p, p - Vec2::new(0., 20.), 4);
        pad.up(p - Vec2::new(0., 20.));
        assert_eq!(chosen.get(), 0, "a scroll chooses nothing");

        let (mut pad, chosen) = radio_pad();
        pad.fling();
        pad.down(pos2(20., 100.));
        pad.up(pos2(20., 100.));
        assert_eq!(chosen.get(), 0, "the finger that catches a fling chooses nothing");

        let (mut pad, chosen) = radio_pad();
        pad.down(pos2(20., 100.));
        pad.up(pos2(20., 100.));
        assert_ne!(chosen.get(), 0, "a plain tap chooses");
    }

    #[test]
    fn button_in_a_list_is_pressed_by_a_tap_not_by_the_catch_of_a_fling() {
        let mut pad = Pad::new(|ui, range, seen| {
            for _ in range {
                let b = ui.add_sized([ui.available_width(), ROW_H], egui::Button::new("b"));
                seen.tapped.set(seen.tapped.get() | tapped(&b));
            }
        });
        pad.fling();
        pad.down(pos2(100., 100.));
        pad.up(pos2(100., 100.));
        assert!(!pad.seen.tapped.get(), "the finger that catches a fling presses no button");
        pad.wait(3.);
        pad.down(pos2(100., 100.));
        pad.up(pos2(100., 100.));
        assert!(pad.seen.tapped.get(), "a plain tap does");
    }

    #[test]
    fn member_row_opens_on_tap_and_not_on_scroll_or_catch() {
        let m = matrix_engine::operations::Member { user_id: "@one:example.org".into(), name: "Marco".into(), avatar: None, role: matrix_domain::role::MemberRole::Admin };
        let mut pad = Pad::new(move |ui, range, seen| {
            for _ in range {
                let r = crate::info::member_row(ui, &Default::default(), &m, false, &matrix_desktop::LIGHT);
                seen.tapped.set(seen.tapped.get() | tapped(&r));
            }
        });
        let p = pos2(100., 300.);
        pad.down(p);
        pad.drag(p, p - Vec2::new(0., 20.), 4);
        pad.up(p - Vec2::new(0., 20.));
        assert!(!pad.seen.tapped.get(), "a scroll");
        pad.wait(3.);
        pad.fling();
        pad.down(pos2(100., 100.));
        pad.up(pos2(100., 100.));
        assert!(!pad.seen.tapped.get(), "a catch");
        pad.wait(3.);
        pad.down(pos2(100., 100.));
        pad.up(pos2(100., 100.));
        assert!(pad.seen.tapped.get(), "a tap");
    }
}
