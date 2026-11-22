//! The size scale in use (buttons, icons, spacing, fonts) and its application to egui. The levels live in the library
//! (`Density::metrics`, `TextSize::zoom`); every control reads `get()` instead of carrying its own numbers.
use eframe::egui;
use matrix_desktop::{Density, Metrics, Preferences};
use std::sync::atomic::{AtomicU8, Ordering};

static LEVEL: AtomicU8 = AtomicU8::new(0);

/// The measures of the current «Dimensione pulsanti e icone».
pub fn get() -> Metrics {
    Density::ALL[LEVEL.load(Ordering::Relaxed) as usize].metrics()
}

/// Side of a button or icon slot: the standard height, 44 on touch.
pub fn button() -> f32 {
    crate::mobile::target(get().button)
}

/// Makes `prefs` the scale in use: zoom for the text, one style for every standard button, the text styles.
pub fn apply(ctx: &egui::Context, prefs: &Preferences, system: f32) {
    LEVEL.store(Density::ALL.iter().position(|d| *d == prefs.density).unwrap_or(0) as u8, Ordering::Relaxed);
    let mut zoom = prefs.text_size.zoom(system);
    if cfg!(target_os = "ios") {
        zoom = matrix_desktop::fit_zoom(zoom, ctx.content_rect().width() * ctx.zoom_factor()); // screen width in points, whatever the zoom now is
    }
    ctx.set_zoom_factor(zoom);
    // Viewport commands are in UI points (zoomed): the same 436 x 420 UI points are a bigger window as the zoom grows, so the layout minimum always fits.
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(crate::MIN_WIDTH, crate::MIN_HEIGHT)));
    let m = get();
    let side = button();
    ctx.all_styles_mut(|s| {
        s.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand); // every egui button, menu entry and tab
        s.spacing.interact_size.y = side;
        s.spacing.button_padding.x = m.pad_x;
        s.spacing.item_spacing.x = m.gap;
        for w in [&mut s.visuals.widgets.inactive, &mut s.visuals.widgets.hovered, &mut s.visuals.widgets.active, &mut s.visuals.widgets.open] {
            w.corner_radius = egui::CornerRadius::same(m.radius as u8);
        }
        for (style, size) in [(egui::TextStyle::Body, m.body), (egui::TextStyle::Button, m.body), (egui::TextStyle::Small, m.secondary), (egui::TextStyle::Heading, m.title)] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
}

/// The OS text-size (accessibility) factor, 1.0 when there is none or it cannot be read. The display's own scale is not in it: egui
/// already multiplies that (native pixels per point). Read at start and when the window regains focus, never polled.
/// macOS has no system-wide text scale besides the display scale, iOS Dynamic Type is not wired (see docs); both give 1.0.
pub fn system_scale() -> f32 {
    #[cfg(debug_assertions)]
    if let Some(v) = std::env::var("GNOBBI_EGUI_SYSTEM_SCALE").ok().and_then(|v| v.parse::<f32>().ok()) {
        return v; // test hook, debug builds only
    }
    #[cfg(target_os = "ios")]
    return ios_scale();
    #[cfg(not(target_os = "ios"))]
    let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    #[cfg(target_os = "linux")]
    let read = run("gsettings", &["get", "org.gnome.desktop.interface", "text-scaling-factor"]).and_then(|o| matrix_desktop::parse_gsettings_scale(&o));
    #[cfg(target_os = "windows")]
    let read = run("reg", &["query", r"HKCU\SOFTWARE\Microsoft\Accessibility", "/v", "TextScaleFactor"]).and_then(|o| matrix_desktop::parse_windows_text_scale(&o));
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "ios")))]
    let read: Option<f32> = { let _ = run; None };
    #[cfg(not(target_os = "ios"))]
    read.unwrap_or(1.)
}

/// iOS Dynamic Type: the app's preferred content size category, mapped on Apple's body-text curve.
#[cfg(target_os = "ios")]
fn ios_scale() -> f32 {
    use objc2::{MainThreadMarker, msg_send, rc::Retained};
    use objc2_foundation::NSString;
    use objc2_ui_kit::UIApplication;
    let Some(mtm) = MainThreadMarker::new() else { return 1. };
    let category: Option<Retained<NSString>> = unsafe { msg_send![&*UIApplication::sharedApplication(mtm), preferredContentSizeCategory] };
    category.map_or(1., |c| matrix_desktop::dynamic_type_scale(&c.to_string()))
}

/// Repaints when the user changes Dynamic Type while the app runs (`refresh` re-reads the category every frame on iOS).
#[cfg(target_os = "ios")]
pub fn observe_text_size(ctx: &egui::Context) {
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
    let ctx = ctx.clone();
    let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| ctx.request_repaint());
    let _ = unsafe { NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(&NSString::from_str("UIContentSizeCategoryDidChangeNotification")), None, None, &block) };
}

/// A single-line field whose text sits on the middle of the box (the fields are as tall as the buttons).
pub fn line(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).align(crate::dir::align(egui::Align2::LEFT_CENTER))
}

/// Pointing-hand cursor over something clickable and enabled; a response that does not sense clicks keeps the normal cursor.
/// egui's own buttons, menu entries and tabs get it from `Visuals::interact_cursor` (set in `apply`); this is for the drawn ones.
pub fn clickable(resp: egui::Response) -> egui::Response {
    if resp.sense.senses_click() && resp.enabled() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cursor the platform is asked for after a few frames with the pointer over a 50x50 widget of `sense`.
    fn cursor_over(sense: egui::Sense) -> egui::CursorIcon {
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 100.))), events: vec![egui::Event::PointerMoved(egui::pos2(10., 10.))], ..Default::default() };
        let mut cursor = egui::CursorIcon::Default;
        for _ in 0..3 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                let (_, resp) = ui.allocate_exact_size(egui::vec2(50., 50.), sense);
                clickable(resp);
            });
            out.textures_delta.clear();
            cursor = out.platform_output.cursor_icon;
        }
        cursor
    }

    #[test]
    fn only_clickable_responses_get_the_hand() {
        assert_eq!(cursor_over(egui::Sense::click()), egui::CursorIcon::PointingHand);
        assert_eq!(cursor_over(egui::Sense::hover()), egui::CursorIcon::Default);
    }
}
