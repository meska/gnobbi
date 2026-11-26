//! Wide (rail, list and chat side by side) or Compact (phone: one column), from the usable window width only.
//! Input kind (`mobile::touch()`) never takes part: an iPad in Split View is Compact, a narrow desktop window too.

use eframe::egui;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LayoutMode { Wide, Compact }

/// The one threshold, in points: rail 76 + list 320 + a chat of at least ~376.
pub const WIDE_MIN: f32 = 772.;
/// Dead band around `WIDE_MIN`, so a width that wobbles at the limit does not flicker between the two layouts.
const HYSTERESIS: f32 = 8.;

/// Pure: `prev` is the mode of the previous frame, if any.
pub fn layout_mode(width: f32, prev: Option<LayoutMode>) -> LayoutMode {
    let limit = match prev {
        Some(LayoutMode::Wide) => WIDE_MIN - HYSTERESIS,
        Some(LayoutMode::Compact) => WIDE_MIN + HYSTERESIS,
        None => WIDE_MIN,
    };
    if width >= limit { LayoutMode::Wide } else { LayoutMode::Compact }
}

/// Mode for the current frame, from the safe-area width. Only the previous mode is remembered, no other state hangs on it.
pub fn mode(ctx: &egui::Context) -> LayoutMode {
    let id = egui::Id::new("layout-mode");
    let m = layout_mode(ctx.content_rect().width(), ctx.data(|d| d.get_temp(id)));
    ctx.data_mut(|d| d.insert_temp(id, m));
    m
}

pub fn compact(ctx: &egui::Context) -> bool { mode(ctx) == LayoutMode::Compact }

#[cfg(test)]
mod tests {
    use super::*;
    use LayoutMode::*;

    #[test]
    fn threshold_without_history() {
        assert_eq!(layout_mode(WIDE_MIN - 0.1, None), Compact);
        assert_eq!(layout_mode(WIDE_MIN, None), Wide);
        assert_eq!(layout_mode(390., None), Compact);
        assert_eq!(layout_mode(1366., None), Wide);
    }

    #[test]
    fn hysteresis_holds_the_previous_mode_near_the_limit() {
        assert_eq!(layout_mode(WIDE_MIN + 1., Some(Compact)), Compact);
        assert_eq!(layout_mode(WIDE_MIN - 1., Some(Wide)), Wide);
        assert_eq!(layout_mode(WIDE_MIN + HYSTERESIS, Some(Compact)), Wide);
        assert_eq!(layout_mode(WIDE_MIN - HYSTERESIS - 0.1, Some(Wide)), Compact);
    }

    #[test]
    fn mode_follows_resize_in_a_context() {
        let ctx = egui::Context::default();
        let at = |w: f32| {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 600.)));
            let mut out = None;
            let mut o = ctx.run_ui(raw, |ui| out = Some(mode(ui.ctx())));
            o.textures_delta.clear(); // egui panics if a pass drops with textures unfreed
            out.unwrap()
        };
        assert_eq!(at(1000.), Wide);
        assert_eq!(at(500.), Compact);
        assert_eq!(at(1000.), Wide);
    }
}

/// The layout does not depend on input kind, on history beyond the previous mode, or on anything but the usable width.
#[cfg(test)]
mod width_only_tests {
    use super::*;
    use LayoutMode::*;

    /// One frame in `ctx`: window `width`, safe-area insets `(left, right)`; returns the mode and the usable width.
    fn frame(ctx: &egui::Context, width: f32, insets: (f32, f32)) -> (LayoutMode, f32) {
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.)));
        if insets != (0., 0.) {
            raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { left: insets.0, right: insets.1, top: 0., bottom: 0. }));
        }
        let mut out = None;
        let mut o = ctx.run_ui(raw, |ui| out = Some((mode(ui.ctx()), ui.ctx().content_rect().width())));
        o.textures_delta.clear();
        out.unwrap()
    }

    /// The modes for a width sequence in a fresh context, with the touch hook on or off.
    fn run(widths: &[f32], touch: bool) -> Vec<LayoutMode> {
        crate::mobile::FORCE_TOUCH.set(touch);
        let ctx = egui::Context::default();
        let modes = widths.iter().map(|&w| frame(&ctx, w, (0., 0.)).0).collect();
        crate::mobile::FORCE_TOUCH.set(false);
        modes
    }

    #[test]
    fn same_width_same_layout_with_and_without_touch() {
        for w in [0., 1., 320., 390., 436., 480., 600., 771.9, 772., 780., 1024., 1366., 3000.] {
            let (off, on) = (run(&[w], false), run(&[w], true));
            assert_eq!(off, on, "width {w}: touch changes the layout");
            assert_eq!(off[0], if w >= WIDE_MIN { Wide } else { Compact }, "width {w}");
        }
        // The two ranges the old rule split by touch: a touch device (iPad Split View) between 480 and the threshold is Compact like a desktop window.
        assert_eq!(run(&[600.], true), vec![Compact]);
        assert_eq!(run(&[600.], false), vec![Compact]);
        // ...and a non-touch window below 480 is Compact too.
        assert_eq!(run(&[400.], false), vec![Compact]);
    }

    #[test]
    fn whole_resize_sequence_is_identical_with_and_without_touch() {
        let seq = [1200., 900., 780., 772., 770., 766., 760., 700., 500., 400., 500., 700., 770., 776., 779., 780., 800., 1200.];
        assert_eq!(run(&seq, false), run(&seq, true));
    }

    #[test]
    fn resize_wide_to_narrow_and_back() {
        let ctx = egui::Context::default();
        let seen: Vec<_> = [1280., 800., 700., 390., 700., 770., 780., 1280.].iter().map(|&w| frame(&ctx, w, (0., 0.)).0).collect();
        // 770 is inside the dead band coming from Compact (needs 780); 780 is out of it.
        assert_eq!(seen, [Wide, Wide, Compact, Compact, Compact, Compact, Wide, Wide]);
    }

    #[test]
    fn wobbling_around_the_threshold_flips_at_most_once() {
        let ctx = egui::Context::default();
        // Starts Wide, then wobbles ±4 pt around the limit: always inside the dead band.
        let mut modes = vec![frame(&ctx, 800., (0., 0.)).0];
        modes.extend((0..200).map(|i| frame(&ctx, if i % 2 == 0 { 768. } else { 776. }, (0., 0.)).0));
        assert!(modes.iter().all(|m| *m == Wide), "flickered: {modes:?}");
        // Same from Compact.
        let ctx = egui::Context::default();
        let mut modes = vec![frame(&ctx, 500., (0., 0.)).0];
        modes.extend((0..200).map(|i| frame(&ctx, if i % 2 == 0 { 768. } else { 776. }, (0., 0.)).0));
        assert!(modes.iter().all(|m| *m == Compact), "flickered: {modes:?}");
        // A swing wider than the band does switch, once per crossing.
        let ctx = egui::Context::default();
        let swing: Vec<_> = [800., 750., 800., 750., 800.].iter().map(|&w| frame(&ctx, w, (0., 0.)).0).collect();
        assert_eq!(swing, [Wide, Compact, Wide, Compact, Wide]);
    }

    #[test]
    fn degenerate_widths_are_compact_and_do_not_panic() {
        for w in [0., 0.5, 1., 10.] {
            assert_eq!(run(&[w], false), vec![Compact], "width {w}");
        }
        assert_eq!(layout_mode(-5., None), Compact);
        assert_eq!(layout_mode(f32::NAN, None), Compact);
        assert_eq!(layout_mode(f32::NAN, Some(Wide)), Compact);
        assert_eq!(layout_mode(f32::INFINITY, Some(Compact)), Wide);
    }

    #[test]
    fn safe_area_counts_and_the_raw_window_width_does_not() {
        let ctx = egui::Context::default();
        // 800 wide, 40 + 40 of notch/rounded corners: 720 usable → Compact although the window is ≥ 772.
        assert_eq!(frame(&ctx, 800., (40., 40.)), (Compact, 720.));
        // 780 wide, no insets → Wide (fresh context, no history).
        assert_eq!(frame(&egui::Context::default(), 780., (0., 0.)), (Wide, 780.));
        // Landscape phone: 852 wide with 59 + 59 → 734 usable → Compact.
        assert_eq!(frame(&egui::Context::default(), 852., (59., 59.)), (Compact, 734.));
        // Insets wider than the window: no panic, Compact.
        assert_eq!(frame(&egui::Context::default(), 50., (40., 40.)).0, Compact);
    }

    #[test]
    fn touch_hook_is_not_read_by_mode() {
        // Same context, touch flipped between frames at a width that sat on the old touch-dependent boundary: no change.
        let ctx = egui::Context::default();
        let mut seen = vec![];
        for (w, t) in [(600., false), (600., true), (500., true), (500., false), (900., true), (900., false)] {
            crate::mobile::FORCE_TOUCH.set(t);
            seen.push(frame(&ctx, w, (0., 0.)).0);
        }
        crate::mobile::FORCE_TOUCH.set(false);
        assert_eq!(seen, [Compact, Compact, Compact, Compact, Wide, Wide]);
    }

    /// State that is not the layout's (open chat, draft, cursor) survives Wide → Compact → Wide. The two layouts draw the same
    /// composer under the same id in different containers, as the app does; the chat and the draft live outside, like `App`/`Session`.
    #[test]
    fn open_chat_and_draft_survive_a_resize_round_trip() {
        struct Chat { open: Option<&'static str>, draft: String }
        let mut chat = Chat { open: Some("!room:x"), draft: String::new() };
        let ctx = egui::Context::default();
        let field = egui::Id::new("composer");
        let step = |chat: &mut Chat, width: f32, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.)));
            raw.events = events;
            let mut m = None;
            let mut o = ctx.run_ui(raw, |ui| {
                let compact = compact(ui.ctx());
                m = Some(compact);
                let edit = |ui: &mut egui::Ui| { ui.add(egui::TextEdit::multiline(&mut chat.draft).id(field)); };
                if compact { egui::CentralPanel::default().show_inside(ui, edit); } else { egui::Panel::left("list").show_inside(ui, |_| {}); egui::CentralPanel::default().show_inside(ui, edit); }
            });
            o.textures_delta.clear();
            m.unwrap()
        };
        assert!(!step(&mut chat, 1200., vec![]));
        ctx.memory_mut(|m| m.request_focus(field));
        step(&mut chat, 1200., vec![]);
        step(&mut chat, 1200., vec![egui::Event::Text("bozza in corso".into())]);
        assert_eq!(chat.draft, "bozza in corso");
        assert!(step(&mut chat, 390., vec![]), "narrow must be compact");
        assert_eq!((chat.open, chat.draft.as_str()), (Some("!room:x"), "bozza in corso"));
        assert!(ctx.memory(|m| m.has_focus(field)), "focus lost on the way to compact");
        step(&mut chat, 390., vec![egui::Event::Text("!".into())]);
        assert!(!step(&mut chat, 1200., vec![]));
        assert_eq!((chat.open, chat.draft.as_str()), (Some("!room:x"), "bozza in corso!"));
        assert!(ctx.memory(|m| m.has_focus(field)), "focus lost on the way back");
    }
}
