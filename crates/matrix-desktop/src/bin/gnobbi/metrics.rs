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

/// A single-line search field: the OS must not autocorrect it (the results come from its text, which iOS changed when the field
/// lost the focus). Every search field goes through this, directly or via `search`.
pub fn search_line(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    line(text).ime_purpose(egui::IMEPurpose::Search)
}

/// A search field of `size` with a clear button (the X, a button-sized square) inside its end edge, shown while there is text.
/// The margin is always there so the text never runs under the X; a click on it empties `text` and reports the field as changed.
pub fn search(ui: &mut egui::Ui, size: [f32; 2], text: &mut String, hint: &str) -> egui::Response {
    let margin = if crate::dir::rtl() { egui::Margin { left: size[1] as i8, right: 8, top: 0, bottom: 0 } } else { egui::Margin { left: 8, right: size[1] as i8, top: 0, bottom: 0 } };
    let mut r = ui.add_sized(size, search_line(text).hint_text(hint).margin(margin));
    if !text.is_empty() {
        let side = size[1];
        let rect = crate::dir::flip(r.rect, egui::Rect::from_center_size(egui::pos2(r.rect.right() - side / 2., r.rect.center().y), egui::vec2(side, side)));
        if crate::icons::button_at(ui, rect, r.id.with("clear"), crate::icons::Icon::X, false, get().icon, crate::i18n::t("Clear search"), None, true, 1.).clicked() {
            text.clear();
            r.mark_changed();
            r.request_focus();
        }
    }
    r
}

/// Pointing-hand cursor over something clickable and enabled; a response that does not sense clicks keeps the normal cursor.
/// egui's own buttons, menu entries and tabs get it from `Visuals::interact_cursor` (set in `apply`); this is for the drawn ones.
pub fn clickable(resp: egui::Response) -> egui::Response {
    if resp.sense.senses_click() && resp.enabled() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The one emoji cell of every grid (composer popup, drawer, message menu): `size` box, `glyph` px emoji, no fill at rest.
/// Desktop: a soft rounded fill under the pointer or focus. Touch: never a fill, not even pressed. `on` = the user's own reaction (accent disc);
/// `grow` enlarges the emoji under pointer or finger.
#[allow(clippy::too_many_arguments)]
pub fn emoji_cell(ui: &mut egui::Ui, size: egui::Vec2, glyph: f32, name: &str, emoji: &str, on: bool, grow: bool, t: &matrix_desktop::DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = clickable(resp);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, matrix_desktop::i18n::t(name)));
    let rgb = crate::list::rgb;
    let disc = rect.width().min(rect.height()) / 2. - 1.;
    if on {
        ui.painter().circle_filled(rect.center(), disc, rgb(t.accent).gamma_multiply(0.3));
    } else if resp.has_focus() || (resp.hovered() && !crate::mobile::touch()) {
        ui.painter().rect_filled(rect.shrink(2.), 8., rgb(t.selected));
    }
    let hot = grow && (resp.is_pointer_button_down_on() || resp.hovered());
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, emoji, egui::FontId::proportional(if hot { (glyph * 1.18).round() } else { glyph }), rgb(t.text));
    resp
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

    /// Filled shapes drawn for one emoji cell after a few frames, pointer at `at` (None = away) and `touch` or desktop.
    fn cell_fills(at: Option<egui::Pos2>, touch: bool) -> usize {
        crate::mobile::FORCE_TOUCH.set(touch);
        let ctx = egui::Context::default();
        let events = at.map(egui::Event::PointerMoved).into_iter().collect();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 100.))), events, ..Default::default() };
        let tokens = matrix_desktop::Theme::Dark.tokens();
        let mut fills = 0;
        for _ in 0..3 {
            let mut out = ctx.run_ui(input.clone(), |ui| { emoji_cell(ui, egui::vec2(40., 40.), 20., "x", "😀", false, false, &tokens); });
            out.textures_delta.clear();
            fills = out.shapes.iter().filter(|s| matches!(s.shape, egui::epaint::Shape::Rect(ref r) if r.fill != egui::Color32::TRANSPARENT)).count();
        }
        crate::mobile::FORCE_TOUCH.set(false);
        fills
    }

    #[test]
    fn emoji_cell_fills_only_on_desktop_hover() {
        assert_eq!(cell_fills(None, false), 0);
        assert_eq!(cell_fills(Some(egui::pos2(60., 60.)), false), 0);
        assert_eq!(cell_fills(Some(egui::pos2(20., 20.)), false), 1);
        assert_eq!(cell_fills(Some(egui::pos2(20., 20.)), true), 0);
    }

    #[test]
    fn only_clickable_responses_get_the_hand() {
        assert_eq!(cursor_over(egui::Sense::click()), egui::CursorIcon::PointingHand);
        assert_eq!(cursor_over(egui::Sense::hover()), egui::CursorIcon::Default);
    }

    /// The cursor after a few frames with the pointer at `at` over the reaction menu (quick row open or expanded) and an egui button.
    fn menu_cursor(at: egui::Pos2, expanded: bool, size: f32) -> (egui::CursorIcon, Option<egui::Rect>) {
        let ctx = crate::icons::test_context();
        crate::metrics::apply(&ctx, &Preferences::default(), size);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(crate::touch_menu::MORE_ID), expanded));
        let t = matrix_desktop::DARK;
        let (mut cursor, mut bottom) = (egui::CursorIcon::Default, None);
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(436., 600.))), events: vec![egui::Event::PointerMoved(at)], ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                crate::touch_menu::reaction_rows(ui, &|_| false, &t);
                bottom = Some(ui.min_rect());
                ui.button("Reply");
            });
            out.textures_delta.clear();
            cursor = out.platform_output.cursor_icon;
        }
        (cursor, bottom)
    }

    #[test]
    fn reaction_menu_shows_the_hand_on_every_clickable_part() {
        let gap = crate::touch_menu::QUICK_GAP;
        let ctx = crate::icons::test_context();
        let mut side = 0.;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(436., 600.))), ..Default::default() }, |ui| side = crate::touch_menu::quick_side(ui));
        out.textures_delta.clear();
        // first quick emoji, the expand button (last cell of the row), a grid cell once expanded, the plain entry under the grid
        let (_, closed) = menu_cursor(egui::pos2(0., 0.), false, 1.);
        let row_y = side / 2. + 8.;
        assert_eq!(menu_cursor(egui::pos2(side / 2. + 8., row_y), false, 1.).0, egui::CursorIcon::PointingHand);
        assert_eq!(menu_cursor(egui::pos2(8. + 6. * (side + gap) + side / 2., row_y), false, 1.).0, egui::CursorIcon::PointingHand);
        assert_eq!(menu_cursor(egui::pos2(side / 2. + 8., row_y + side + gap), true, 1.).0, egui::CursorIcon::PointingHand);
        let below = closed.unwrap().bottom() + 12.;
        assert_eq!(menu_cursor(egui::pos2(20., below), false, 1.).0, egui::CursorIcon::PointingHand);
        assert_eq!(menu_cursor(egui::pos2(420., 590.), false, 1.).0, egui::CursorIcon::Default);
    }

    #[test]
    fn clear_button_empties_the_search_only_when_there_is_text() {
        let ctx = crate::icons::test_context();
        let mut text = "ab".to_string();
        let mut run = |text: &mut String, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 100.))), events, ..Default::default() };
            let (mut rect, mut changed) = (egui::Rect::NOTHING, false);
            let mut out = ctx.run_ui(input, |ui| {
                let r = search(ui, [200., 40.], text, "hint");
                (rect, changed) = (r.rect, r.changed());
            });
            out.textures_delta.clear();
            (rect, changed)
        };
        let (rect, _) = run(&mut text, vec![]);
        let at = egui::pos2(rect.right() - 20., rect.center().y);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut text, vec![egui::Event::PointerMoved(at)]);
        run(&mut text, vec![click(true)]);
        let (_, changed) = run(&mut text, vec![click(false)]);
        assert!(text.is_empty() && changed);
        // no text: no button, a click there leaves the (empty) text alone and changes nothing
        let (_, changed) = run(&mut text, vec![click(true)]);
        assert!(!changed && text.is_empty());
    }
}
