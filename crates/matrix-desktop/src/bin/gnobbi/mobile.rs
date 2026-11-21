//! Phone facts in one place: safe-area insets, on-screen keyboard height, touch targets. The rest of the UI only reads
//! `content_rect()` and `target()`. On desktop all of it is zero/off unless the test hooks below simulate a phone.
use eframe::egui;
use std::sync::atomic::{AtomicU32, Ordering};

/// Smallest side of anything tappable on a touch screen.
const TOUCH: f32 = 44.;

/// Home indicator height iOS reports in landscape on notch phones.
const LANDSCAPE_HOME: f32 = 21.;

static KEYBOARD: AtomicU32 = AtomicU32::new(0);

/// Points of the window bottom covered by the on-screen keyboard (0 = closed). The platform observer calls this from any thread.
pub fn set_keyboard(points: f32) {
    KEYBOARD.store(points.max(0.).to_bits(), Ordering::Relaxed);
}

/// Points of the screen bottom covered by a keyboard whose end frame starts at `frame.0` and is `frame.1` wide, on a screen of
/// `screen` = (width, height). None for the transient frame iOS posts in the rotated space (width = screen height, y = 67):
/// a frame that is not as wide as the screen is not the keyboard, keep the previous value.
#[cfg(any(target_os = "ios", test))]
fn covered_by_keyboard(screen: (f64, f64), frame: (f64, f64)) -> Option<f32> {
    ((frame.1 - screen.0).abs() <= 1.).then(|| (screen.1 - frame.0).max(0.) as f32)
}

/// iOS: follows the on-screen keyboard (frame changes and hide) into `set_keyboard`. Call once at startup, on the main thread.
#[cfg(target_os = "ios")]
pub fn observe_keyboard(ctx: &egui::Context) {
    use objc2::{MainThreadMarker, msg_send, runtime::AnyObject};
    use objc2_core_foundation::CGRect;
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use objc2_ui_kit::{UIKeyboardFrameEndUserInfoKey, UIScreen, UIKeyboardWillChangeFrameNotification, UIKeyboardWillHideNotification};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let center = NSNotificationCenter::defaultCenter();
    let on = |name: &objc2_foundation::NSNotificationName, hide: bool| {
        let ctx = ctx.clone();
        let block = block2::RcBlock::new(move |n: std::ptr::NonNull<NSNotification>| {
            let n = unsafe { n.as_ref() };
            let covered = if hide {
                0.
            } else {
                // The end frame is in screen coordinates. `keyWindow` is NOT the screen: during keyboard transitions it is a system
                // window of another size (seen: 402 for 874), so measure against the screen.
                let frame = n.userInfo().and_then(|u| u.objectForKey(unsafe { UIKeyboardFrameEndUserInfoKey }));
                let screen = UIScreen::mainScreen(mtm).bounds();
                let Some(v) = frame else { return };
                let v: &AnyObject = &v;
                let f: CGRect = unsafe { msg_send![v, CGRectValue] };
                let Some(covered) = covered_by_keyboard((screen.size.width, screen.max().y), (f.min().y, f.size.width)) else { return };
                covered
            };
            set_keyboard(covered);
            ctx.request_repaint();
        });
        // The centre keeps the observer; it lives as long as the app.
        let _ = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
    };
    unsafe {
        on(UIKeyboardWillChangeFrameNotification, false);
        on(UIKeyboardWillHideNotification, true);
    }
}

/// iOS: give short app switches a grace period, then free every shared-store lock before suspension.
#[cfg(target_os = "ios")]
pub fn observe_lifecycle(ctx: &egui::Context) {
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
    use objc2_ui_kit::{UIApplication, UIApplicationDidEnterBackgroundNotification, UIApplicationWillEnterForegroundNotification, UIBackgroundTaskInvalid};
    use crate::suspend::{Action, Event, State, transition};
    use std::cell::RefCell;
    use std::ffi::c_void;

    thread_local! { static LIFE: RefCell<(State, usize, Option<usize>)> = const { RefCell::new((State::Foreground, 0, None)) }; }
    unsafe extern "C" {
        static _dispatch_main_q: u8;
        fn dispatch_time(when: u64, delta: i64) -> u64;
        fn dispatch_after_f(when: u64, queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    }
    fn end(mtm: MainThreadMarker) {
        let task = LIFE.with(|l| l.borrow_mut().2.take());
        if let Some(task) = task && task != unsafe { UIBackgroundTaskInvalid } {
            UIApplication::sharedApplication(mtm).endBackgroundTask(task);
        }
    }
    fn release(generation: usize, mtm: MainThreadMarker) {
        let do_release = LIFE.with(|l| {
            let mut l = l.borrow_mut();
            if l.1 != generation { return false; }
            let (state, action) = transition(l.0, Event::Deadline);
            l.0 = state;
            action == Action::Release
        });
        if !do_release { return; }
        eprintln!("Gnobbi lifecycle: release");
        if crate::suspend::suspend(std::time::Duration::from_secs(8)) {
            end(mtm);
        } else {
            // Keeping the task open lets iOS terminate rather than suspend with a shared-store lock.
            eprintln!("Gnobbi lifecycle: worker did not release its lock");
            if LIFE.with(|l| l.borrow().2 == Some(unsafe { UIBackgroundTaskInvalid })) {
                std::process::abort();
            }
        }
    }
    extern "C" fn timer(context: *mut c_void) {
        if let Some(mtm) = MainThreadMarker::new() { release(context as usize, mtm); }
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let center = NSNotificationCenter::defaultCenter();
    let back = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
        let generation = LIFE.with(|l| {
            let mut l = l.borrow_mut();
            let (state, action) = transition(l.0, Event::Background);
            l.0 = state;
            if action != Action::Begin { return None; }
            l.1 = l.1.wrapping_add(1).max(1);
            Some(l.1)
        });
        let Some(generation) = generation else { return };
        crate::suspend::background();
        let app = UIApplication::sharedApplication(mtm);
        let expired = block2::RcBlock::new(move || release(generation, mtm));
        let task = app.beginBackgroundTaskWithName_expirationHandler(Some(&NSString::from_str("Gnobbi stores")), Some(&expired));
        LIFE.with(|l| l.borrow_mut().2 = Some(task));
        if task == unsafe { UIBackgroundTaskInvalid } {
            release(generation, mtm);
            return;
        }
        let delay = (app.backgroundTimeRemaining() - 10.0).clamp(0.0, 20.0);
        eprintln!("Gnobbi lifecycle: grace {delay:.1}s");
        unsafe { dispatch_after_f(dispatch_time(0, (delay * 1_000_000_000.0) as i64), &raw const _dispatch_main_q as *const c_void, generation as *mut c_void, timer); }
    });
    let ctx = ctx.clone();
    let front = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
        let action = LIFE.with(|l| {
            let mut l = l.borrow_mut();
            let (state, action) = transition(l.0, Event::Foregrounded);
            l.0 = state;
            action
        });
        match action {
            Action::Cancel => { eprintln!("Gnobbi lifecycle: cancel grace"); crate::suspend::resume(); end(mtm); }
            Action::Restart => { eprintln!("Gnobbi lifecycle: restart"); crate::suspend::resume(); end(mtm); }
            _ => {}
        }
        ctx.request_repaint();
    });
    unsafe {
        let _ = center.addObserverForName_object_queue_usingBlock(Some(UIApplicationDidEnterBackgroundNotification), None, None, &back);
        let _ = center.addObserverForName_object_queue_usingBlock(Some(UIApplicationWillEnterForegroundNotification), None, None, &front);
    }
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

/// Bottom band the layout leaves free: the keyboard when it is up (it already covers the home indicator), else the home indicator.
/// Never the sum, and never the safe inset above a keyboard: that was a blank band between the content and the keys.
fn bottom_reserved(safe_bottom: f32, keyboard: f32) -> f32 {
    if keyboard > 0. { keyboard } else { safe_bottom }
}

/// The insets of the last window event, with the keyboard folded into the bottom one.
#[derive(Default)]
pub struct Insets(egui::SafeAreaInsets, bool);

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
    /// Returns true when the screen just turned between portrait and landscape.
    pub fn apply(&mut self, raw: &mut egui::RawInput) -> bool {
        if let Some(real) = raw.safe_area_insets {
            self.0 = real;
        }
        let mut all = self.0;
        let landscape = raw.screen_rect.is_some_and(|r| r.width() > r.height());
        // iOS keeps reporting the portrait insets after a rotation (top = notch, sides 0, bottom = home bar): in landscape the notch is on a side
        // (both sides, for symmetry), nothing is left on top and the home indicator is the short one.
        // shortcut: assumes insets left+right == 0 means stale and top > 40 means a notch (SE top is 20); drop it if UIKit starts reporting the sides.
        if landscape && all.0.left + all.0.right == 0. && all.0.top > 40. {
            (all.0.left, all.0.right, all.0.top, all.0.bottom) = (all.0.top, all.0.top, 0., LANDSCAPE_HOME);
        }
        all.0.bottom = bottom_reserved(all.0.bottom, f32::from_bits(KEYBOARD.load(Ordering::Relaxed)));
        raw.safe_area_insets = Some(all);
        raw.screen_rect.is_some() && std::mem::replace(&mut self.1, landscape) != landscape
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

/// A drag that began at the left edge ends as «back»: mostly horizontal and past a third of the width, or a quick flick of at least 40 pt.
fn is_back_swipe(dx: f32, dy: f32, secs: f32, width: f32) -> bool {
    dx > 2. * dy.abs() && (dx > width / 3. || (dx > 40. && dx > 500. * secs))
}

/// True on the frame a left-edge swipe to the right is released (the iOS back gesture).
/// shortcut: acts on release, the page does not follow the finger; add a translate if the feel asks for it.
pub fn back_swipe(ctx: &egui::Context) -> bool {
    const EDGE: f32 = 24.;
    let id = egui::Id::new("back-swipe");
    // RTL: the list is on the right, so the gesture starts at the right edge and goes left.
    let (content, rtl) = (ctx.content_rect(), crate::dir::rtl());
    let side = |x: f32| if rtl { content.right() - x } else { x - content.left() };
    let (pressed, released, time) = ctx.input(|i| (i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten(), i.pointer.any_released().then(|| i.pointer.interact_pos().or(i.pointer.latest_pos())).flatten(), i.time));
    if let Some(p) = pressed {
        ctx.data_mut(|d| d.insert_temp(id, (side(p.x) < EDGE).then_some((p, time))));
    }
    let start = if released.is_some() { ctx.data_mut(|d| d.remove_temp::<Option<(egui::Pos2, f64)>>(id)).flatten() } else { None };
    matches!((start, released), (Some((s, t0)), Some(e)) if is_back_swipe(side(e.x) - side(s.x), e.y - s.y, (time - t0) as f32, content.width()))
}

/// Paint the rail behind the unsafe bands, without moving or unclipping its contents.
pub fn rail_background(ui: &egui::Ui, fill: egui::Color32) {
    let (all, safe) = (ui.ctx().viewport_rect(), ui.ctx().content_rect());
    if all != safe {
        let rect = crate::dir::flip(safe, egui::Rect::from_min_max(egui::pos2(safe.left(), all.top()), egui::pos2((safe.left() + crate::rail::WIDTH).min(safe.right()), all.bottom())));
        egui::Painter::new(ui.ctx().clone(), ui.layer_id(), all).rect_filled(rect, 0., fill);
    }
}

/// A child ui for a phone list that runs on into the home-indicator band (clip included, so no invisible line cuts a row) and the
/// band's height, to add as scrollable padding so the last row can be brought above the indicator. With the keyboard up (it covers
/// the band) or not on a phone: the plain remaining space and 0.
pub fn extend_list(ui: &mut egui::Ui, single: bool) -> (egui::Ui, f32) {
    let (screen, safe) = (ui.ctx().viewport_rect(), ui.ctx().content_rect());
    let band = if single && f32::from_bits(KEYBOARD.load(Ordering::Relaxed)) <= 0. { (screen.bottom() - safe.bottom()).max(0.) } else { 0. };
    let mut rect = ui.available_rect_before_wrap();
    rect.max.y += band;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let mut clip = child.clip_rect();
    clip.max.y = (clip.max.y + band).min(screen.bottom());
    child.set_clip_rect(clip);
    (child, band)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_replaces_the_home_indicator_band_instead_of_adding_to_it() {
        assert_eq!(bottom_reserved(34., 336.), 336.);
        assert_eq!(bottom_reserved(300., 336.), 336., "a stale or oversized safe inset never shows above the keys");
        assert_eq!(bottom_reserved(34., 0.), 34.);
    }

    #[test]
    fn phone_list_extends_clip_and_reserves_the_bottom_inset() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.));
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(screen), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 59., bottom: 34., ..Default::default() })), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let mut ui = safe_ui(ui, egui::Color32::BLACK);
            ui.set_clip_rect(ctx.content_rect());
            let top = ui.cursor().min.y;
            let (child, band) = extend_list(&mut ui, true);
            assert_eq!((band, child.max_rect().bottom(), child.clip_rect().bottom(), child.max_rect().top()), (34., 852., 852., top));
            let (child, band) = extend_list(&mut ui, false);
            assert_eq!((band, child.max_rect().bottom()), (0., 818.), "wide layout: untouched");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn back_swipe_needs_a_horizontal_drag_far_or_fast() {
        assert!(is_back_swipe(150., 10., 0.6, 393.), "past a third");
        assert!(is_back_swipe(60., 5., 0.1, 393.), "short but fast");
        assert!(!is_back_swipe(60., 5., 0.5, 393.), "short and slow: cancelled");
        assert!(!is_back_swipe(200., 150., 0.3, 393.), "diagonal: a scroll");
        assert!(!is_back_swipe(-200., 0., 0.2, 393.), "leftwards");
        assert!(!is_back_swipe(0., 0., 0.1, 393.), "a tap");
    }

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
    fn rail_and_list_paint_safe_bands_in_every_theme() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.));
        for theme in matrix_desktop::Theme::ALL {
            let t = theme.tokens_with(matrix_desktop::Accent::Blue);
            for (top, bottom) in [(59., 34.), (0., 0.)] {
                let ctx = egui::Context::default();
                let raw = egui::RawInput { screen_rect: Some(screen), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top, bottom, ..Default::default() })), ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| {
                    let ui = safe_ui(ui, crate::list_fill(&t));
                    rail_background(&ui, crate::chat_background(&t));
                });
                out.textures_delta.clear();
                let rects: Vec<_> = out.shapes.iter().filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => Some((s.clip_rect, r.rect, r.fill)),
                    _ => None,
                }).collect();
                if top > 0. {
                    assert!(rects.iter().any(|(clip, rect, fill)| *clip == screen && *rect == screen && *fill == crate::list_fill(&t)), "list: {theme:?}");
                    let rail = egui::Rect::from_min_max(screen.min, egui::pos2(crate::rail::WIDTH, screen.bottom()));
                    assert!(rects.iter().any(|(clip, rect, fill)| *clip == screen && *rect == rail && *fill == crate::chat_background(&t)), "rail: {theme:?}");
                } else {
                    assert!(rects.is_empty(), "desktop paints no extra bands: {theme:?}");
                }
            }
        }
    }

    #[test]
    fn landscape_notch_becomes_a_side_inset() {
        let mut insets = Insets::default();
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(852., 393.))), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 62., bottom: 34., ..Default::default() })), ..Default::default() };
        insets.apply(&mut raw);
        let m = raw.safe_area_insets.unwrap().0;
        assert_eq!((m.left, m.right, m.top, m.bottom), (62., 62., 0., 21.));
        let mut se = egui::RawInput { screen_rect: raw.screen_rect, safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 20., ..Default::default() })), ..Default::default() };
        insets.apply(&mut se);
        assert_eq!(se.safe_area_insets.unwrap().0.left, 0., "no notch: nothing to add");
    }

    #[test]
    fn keyboard_frames_of_the_wrong_shape_are_ignored() {
        let screen = (402., 874.);
        assert_eq!(covered_by_keyboard(screen, (539., 402.)), Some(335.), "docked keyboard");
        assert_eq!(covered_by_keyboard(screen, (874., 402.)), Some(0.), "hidden: parked below the screen");
        assert_eq!(covered_by_keyboard(screen, (67., 874.)), None, "transient frame in the rotated space");
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
