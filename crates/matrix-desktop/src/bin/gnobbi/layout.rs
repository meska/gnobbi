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
