//! Phone facts in one place: safe-area insets, on-screen keyboard height, touch targets. The rest of the UI only reads
//! `content_rect()` and `target()`. On desktop all of it is zero/off unless the test hooks below simulate a phone.
use eframe::egui;
use std::sync::atomic::{AtomicU32, Ordering};

/// Smallest side of anything tappable on a touch screen.
const TOUCH: f32 = 44.;

static KEYBOARD: AtomicU32 = AtomicU32::new(0);

/// Points of the window bottom covered by the on-screen keyboard (0 = closed). The platform observer calls this from any thread.
pub fn set_keyboard(points: f32) {
    KEYBOARD.store(points.max(0.).to_bits(), Ordering::Relaxed);
}

/// iOS, or the test hook GNOBBI_EGUI_TOUCH on desktop.
pub fn touch() -> bool {
    static TOUCH_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TOUCH_ON.get_or_init(|| cfg!(target_os = "ios") || std::env::var_os("GNOBBI_EGUI_TOUCH").is_some())
}

/// Side of a tappable target: `desktop` as it is, at least 44 pt on touch.
pub fn target(desktop: f32) -> f32 {
    if touch() { desktop.max(TOUCH) } else { desktop }
}

/// A one-glyph button, 44 pt square on touch.
pub fn icon_button(ui: &mut egui::Ui, glyph: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::new(glyph).min_size(egui::Vec2::splat(target(0.))))
}

/// The insets of the last window event, with the keyboard folded into the bottom one.
#[derive(Default)]
pub struct Insets(egui::SafeAreaInsets);

impl Insets {
    /// Test hooks: GNOBBI_EGUI_INSETS=<top,left,bottom,right> and GNOBBI_EGUI_KEYBOARD=<points> simulate a phone on desktop.
    pub fn from_env() -> Self {
        let mut insets = Self::default();
        if let Ok(v) = std::env::var("GNOBBI_EGUI_INSETS") {
            let mut n = v.split(',').map(|p| p.trim().parse().unwrap_or(0.));
            let m = &mut insets.0.0;
            (m.top, m.left, m.bottom, m.right) = (n.next().unwrap_or(0.), n.next().unwrap_or(0.), n.next().unwrap_or(0.), n.next().unwrap_or(0.));
        }
        if let Some(k) = std::env::var("GNOBBI_EGUI_KEYBOARD").ok().and_then(|v| v.parse().ok()) {
            set_keyboard(k);
        }
        insets
    }

    /// Called from `raw_input_hook`. egui-winit sends the insets only with a window event (resize, rotation, focus): keep the last ones.
    pub fn apply(&mut self, raw: &mut egui::RawInput) {
        if let Some(real) = raw.safe_area_insets {
            self.0 = real;
        }
        let mut all = self.0;
        all.0.bottom = all.0.bottom.max(f32::from_bits(KEYBOARD.load(Ordering::Relaxed)));
        raw.safe_area_insets = Some(all);
    }
}

/// The root ui cut to the safe area; the strips outside it (notch, corners, home bar, keyboard) take the colour of the bars.
pub fn safe_ui(ui: &mut egui::Ui, fill: egui::Color32) -> egui::Ui {
    let (all, safe) = (ui.ctx().viewport_rect(), ui.ctx().content_rect());
    if all != safe {
        ui.painter().rect_filled(all, 0., fill);
    }
    ui.new_child(egui::UiBuilder::new().max_rect(safe))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Header top and composer bottom of one frame on a 393x852 screen.
    fn frame(ctx: &egui::Context, insets: &mut Insets, event: Option<egui::SafeAreaInsets>) -> (f32, f32) {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.))), safe_area_insets: event, ..Default::default() };
        insets.apply(&mut raw);
        let mut edges = (0., 0.);
        let mut out = ctx.run_ui(raw, |ui| {
            let mut ui = safe_ui(ui, egui::Color32::BLACK);
            edges.0 = egui::Panel::top("header").exact_size(64.).show(&mut ui, |_| {}).response.rect.top();
            edges.1 = crate::composer::panel(egui::Frame::NONE).show(&mut ui, |_| {}).response.rect.bottom();
        });
        out.textures_delta.clear();
        edges
    }

    #[test]
    fn content_stays_in_the_safe_area_and_the_composer_rides_the_keyboard() {
        let phone = |top, bottom| egui::SafeAreaInsets(egui::epaint::MarginF32 { top, bottom, ..Default::default() });
        let ctx = egui::Context::default();
        let mut insets = Insets::default();
        assert_eq!(frame(&ctx, &mut insets, None), (0., 852.), "desktop: no insets, nothing moves");
        assert_eq!(frame(&ctx, &mut insets, Some(phone(59., 34.))), (59., 818.));
        assert_eq!(frame(&ctx, &mut insets, None), (59., 818.), "a frame without a window event keeps the insets");
        set_keyboard(336.);
        assert_eq!(frame(&ctx, &mut insets, None), (59., 516.), "keyboard open: the composer sits on it");
        set_keyboard(0.);
        assert_eq!(frame(&ctx, &mut insets, None), (59., 818.), "keyboard closed: back above the home bar");
        assert_eq!(frame(&ctx, &mut insets, Some(phone(0., 21.))), (0., 831.), "rotation: the new insets replace the old");
    }
}
