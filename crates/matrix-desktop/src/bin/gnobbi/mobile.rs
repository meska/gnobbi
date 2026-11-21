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

/// Share of the way to the list for a finger `dx` points right of where it went down.
pub fn nav_drag_progress(dx: f32, width: f32) -> f32 {
    if width > 0. { (dx / width).clamp(0., 1.) } else { 0. }
}

/// Ease-out cubic from `from` to `target`; with Reduce Motion the target at once.
pub fn nav_ease(from: f32, target: f32, elapsed: f32, duration: f32, reduce_motion: bool) -> f32 {
    let t = if reduce_motion || duration <= 0. { 1. } else { (elapsed / duration).clamp(0., 1.) };
    from + (target - from) * (1. - (1. - t).powi(3))
}

/// Horizontal shift of (list, chat) at `progress`: the chat slides off by the full width, the list trails at 30 % under it.
pub fn nav_offsets(progress: f32, width: f32, rtl: bool) -> (f32, f32) {
    let sign = if rtl { -1. } else { 1. };
    (sign * -0.3 * (1. - progress) * width, sign * progress * width)
}

/// iOS «Reduce Motion» (Settings > Accessibility); false elsewhere.
pub fn reduce_motion() -> bool {
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn UIAccessibilityIsReduceMotionEnabled() -> bool;
        }
        unsafe { UIAccessibilityIsReduceMotionEnabled() }
    }
    #[cfg(not(target_os = "ios"))]
    false
}

/// Test hook: GNOBBI_EGUI_NAV_PROGRESS=<0..1> holds the one-column navigation (chat over list, or the drawer on top) at that point, the page under it drawn too.
pub fn nav_forced() -> Option<f32> {
    static FORCED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| std::env::var("GNOBBI_EGUI_NAV_PROGRESS").ok().and_then(|v| v.parse::<f32>().ok()).map(|p| p.clamp(0., 1.)))
}

/// A page over the one under it in the one-column layout: progress 0 = the page fully shown, 1 = slid off to the side.
/// The chat over the list, a drawer over the list or over the drawer it came from.
#[derive(Default, Clone, Copy)]
pub struct Nav {
    pub progress: f32,
    from: f32,
    /// None until the first frame, which shows its page without a move.
    target: Option<f32>,
    /// Start and length in seconds of the running move.
    started: Option<(std::time::Instant, f32)>,
    /// The page is under the finger.
    pub dragging: bool,
    forced: Option<f32>,
}

impl Nav {
    const COMPLETE: f32 = 0.25;
    const CANCEL: f32 = 0.18;

    /// With the test hook `nav_forced`.
    pub fn from_env() -> Self {
        Self { forced: nav_forced(), ..Default::default() }
    }

    /// Once a frame. `animate` = one column with a room; `drag` = where the finger holds the page. True when both pages are to be drawn.
    pub fn step(&mut self, animate: bool, list_open: bool, drag: Option<f32>, now: std::time::Instant, reduce_motion: bool) -> bool {
        let target = if list_open { 1. } else { 0. };
        if let (Some(p), true) = (self.forced, animate) {
            self.progress = p;
            return true;
        }
        self.dragging = animate && !reduce_motion && drag.is_some();
        if !animate || self.target.is_none() {
            (self.progress, self.target, self.started) = (target, Some(target), None);
        } else if let Some(p) = drag.filter(|_| self.dragging) {
            (self.progress, self.started) = (p, None);
        } else if self.target != Some(target) || (self.started.is_none() && self.progress != target) {
            // A page let go short of the threshold goes back quicker than a completed move.
            let secs = if self.target == Some(target) { Self::CANCEL } else { Self::COMPLETE };
            (self.from, self.target, self.started) = (self.progress, Some(target), Some((now, secs)));
        }
        if let Some((at, secs)) = self.started {
            let elapsed = now.saturating_duration_since(at).as_secs_f32();
            self.progress = nav_ease(self.from, target, elapsed, secs, reduce_motion);
            if reduce_motion || elapsed >= secs {
                self.started = None;
            }
        }
        // The first frame of a move is still at its start: both pages, or the destination would flash.
        self.dragging || self.started.is_some() || (self.progress > 0. && self.progress < 1.)
    }

    /// A move is running, or `list_open` changed after this frame's `step`: the next frame is due now.
    pub fn wants_repaint(&self, list_open: bool) -> bool {
        self.started.is_some() || self.target.is_some_and(|t| (t == 1.) != list_open)
    }
}

/// A page of the one-column navigation shifted sideways by `dx`.
pub fn nav_layer(ui: &mut egui::Ui, dx: f32) -> egui::Ui {
    // The scope id of the page at rest: scroll positions and panel state are keyed by it and must not reset during the move.
    let mut layer = ui.new_child(egui::UiBuilder::new().scope_id(ui.scope_id()).max_rect(ui.available_rect_before_wrap().translate(egui::vec2(dx, 0.))));
    // Clipped to the whole shifted page, not to its visible part: the timeline lays out in its clip width; the screen cuts the rest.
    layer.set_clip_rect(ui.ctx().viewport_rect().translate(egui::vec2(dx, 0.)));
    layer
}

/// Between the two pages: the list dimmed (less as it is uncovered), the shadow of the chat's edge and the chat's opaque base, safe-area bands included.
pub fn nav_cover(ui: &egui::Ui, progress: f32, chat_dx: f32, fill: egui::Color32) {
    const SHADOW: f32 = 10.;
    let screen = ui.ctx().viewport_rect();
    let painter = egui::Painter::new(ui.ctx().clone(), ui.layer_id(), screen);
    painter.rect_filled(screen, 0., egui::Color32::from_black_alpha((0.25 * (1. - progress) * 255.) as u8));
    let page = screen.translate(egui::vec2(chat_dx, 0.));
    let (edge, out) = if crate::dir::rtl() { (page.right(), SHADOW) } else { (page.left(), -SHADOW) };
    let mut mesh = egui::Mesh::default();
    for (x, color) in [(edge, egui::Color32::from_black_alpha(46)), (edge + out, egui::Color32::TRANSPARENT)] {
        mesh.colored_vertex(egui::pos2(x, screen.top()), color);
        mesh.colored_vertex(egui::pos2(x, screen.bottom()), color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_filled(page, 0., fill);
}

/// A drag that began at the left edge ends as «back»: mostly horizontal and past a third of the width, or a quick flick of at least 40 pt.
fn is_back_swipe(dx: f32, dy: f32, secs: f32, width: f32) -> bool {
    dx > 2. * dy.abs() && (dx > width / 3. || (dx > 40. && dx > 500. * secs))
}

#[derive(Debug, PartialEq)]
pub enum BackDrag {
    /// The finger holds the page this far towards the list (0..1).
    Moving(f32),
    /// Let go: true = back to the list.
    Released(bool),
}

/// The iOS back gesture, frame by frame: a touch that went down at the left edge and moves right.
pub fn back_drag(ctx: &egui::Context) -> Option<BackDrag> {
    const EDGE: f32 = 24.;
    let id = egui::Id::new("back-swipe");
    // RTL: the list is on the right, so the gesture starts at the right edge and goes left.
    let (content, rtl) = (ctx.content_rect(), crate::dir::rtl());
    let side = |x: f32| if rtl { content.right() - x } else { x - content.left() };
    let (pressed, released, down, pos, time) = ctx.input(|i| (i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten(), i.pointer.any_released(), i.pointer.any_down(), i.pointer.interact_pos().or(i.pointer.latest_pos()), i.time));
    if let Some(p) = pressed {
        ctx.data_mut(|d| d.insert_temp(id, (side(p.x) < EDGE).then_some((p, time, false))));
    }
    let (start, t0, mut moving) = ctx.data(|d| d.get_temp::<Option<(egui::Pos2, f64, bool)>>(id)).flatten()?;
    let Some(pos) = pos.filter(|_| released || down) else {
        ctx.data_mut(|d| d.remove_temp::<Option<(egui::Pos2, f64, bool)>>(id));
        return None;
    };
    let (dx, dy) = (side(pos.x) - side(start.x), pos.y - start.y);
    if released {
        ctx.data_mut(|d| d.remove_temp::<Option<(egui::Pos2, f64, bool)>>(id));
        let back = is_back_swipe(dx, dy, (time - t0) as f32, content.width());
        return (moving || back).then_some(BackDrag::Released(back));
    }
    if !moving && dx > 8. && dx > 2. * dy.abs() {
        moving = true;
        ctx.data_mut(|d| d.insert_temp(id, Some((start, t0, true))));
        // The page is under the finger now: the keyboard goes away and what was pressed (a scroll, a row) lets go.
        ctx.memory_mut(|m| if let Some(f) = m.focused() { m.surrender_focus(f) });
        ctx.stop_dragging();
    }
    moving.then(|| BackDrag::Moving(nav_drag_progress(dx, content.width())))
}

/// Paint the rail behind the unsafe bands, without moving or unclipping its contents; `dx` = the list page's shift during a navigation move.
pub fn rail_background(ui: &egui::Ui, fill: egui::Color32, dx: f32) {
    let (all, safe) = (ui.ctx().viewport_rect(), ui.ctx().content_rect());
    if all != safe {
        let rect = crate::dir::flip(safe, egui::Rect::from_min_max(egui::pos2(safe.left(), all.top()), egui::pos2((safe.left() + crate::rail::WIDTH).min(safe.right()), all.bottom()))).translate(egui::vec2(dx, 0.));
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
    fn navigation_pages_shift_by_progress() {
        let near = |got: (f32, f32), want: (f32, f32)| assert!((got.0 - want.0).abs() < 1e-3 && (got.1 - want.1).abs() < 1e-3, "{got:?} != {want:?}");
        near(nav_offsets(0., 400., false), (-120., 0.));
        near(nav_offsets(0.5, 400., false), (-60., 200.));
        near(nav_offsets(1., 400., false), (0., 400.));
        near(nav_offsets(0.5, 400., true), (60., -200.));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.));
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(screen), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 59., bottom: 34., ..Default::default() })), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let mut ui = safe_ui(ui, egui::Color32::BLACK);
            for p in [0., 0.25, 0.5, 0.75, 1.] {
                let (list_dx, chat_dx) = nav_offsets(p, screen.width(), false);
                let (list, chat) = (nav_layer(&mut ui, list_dx), nav_layer(&mut ui, chat_dx));
                assert_eq!(chat.max_rect().min.x, p * 393.);
                assert_eq!(list.max_rect().min.x, -0.3 * (1. - p) * 393.);
                assert_eq!((chat.max_rect().width(), chat.max_rect().top(), chat.max_rect().bottom()), (393., 59., 818.), "a page keeps its size");
                assert_eq!((chat.clip_rect(), list.clip_rect()), (screen.translate(egui::vec2(chat_dx, 0.)), screen.translate(egui::vec2(list_dx, 0.))), "full page width, safe-area bands included");
                assert_eq!(chat.scope_id(), ui.scope_id(), "same ids as at rest: no scroll state lost");
            }
        });
        out.textures_delta.clear();
    }

    #[test]
    fn navigation_cover_dims_the_list_and_bases_the_chat_on_the_whole_screen() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.));
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(screen), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 59., bottom: 34., ..Default::default() })), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| nav_cover(ui, 0.5, 196.5, egui::Color32::RED));
        out.textures_delta.clear();
        let rects: Vec<_> = out.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) => Some((s.clip_rect, r.rect, r.fill)),
            _ => None,
        }).collect();
        assert_eq!(rects, [(screen, screen, egui::Color32::from_black_alpha(31)), (screen, screen.translate(egui::vec2(196.5, 0.)), egui::Color32::RED)]);
        assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.vertices[0].pos.x == 196.5 && m.vertices[2].pos.x == 186.5)), "shadow left of the chat edge");
    }

    #[test]
    fn navigation_state_follows_drag_then_animates_to_the_released_side() {
        let t0 = std::time::Instant::now();
        let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
        let mut nav = Nav::default();
        assert!(!nav.step(true, true, None, t0, false), "first frame: the list, no move");
        assert_eq!(nav.progress, 1.);
        // A room is opened: push 1 -> 0 in 250 ms.
        assert!(!nav.wants_repaint(true) && nav.wants_repaint(false));
        assert!(nav.step(true, false, None, at(0), false), "both pages from the first frame of the move");
        assert_eq!(nav.progress, 1.);
        assert!(nav.step(true, false, None, at(125), false));
        assert!((nav.progress - 0.125).abs() < 1e-4);
        assert!(!nav.step(true, false, None, at(260), false));
        assert_eq!((nav.progress, nav.wants_repaint(false)), (0., false));
        // Dragged a quarter of the way and let go: back to the chat in 180 ms.
        assert!(nav.step(true, false, Some(0.25), at(300), false));
        assert_eq!((nav.progress, nav.dragging), (0.25, true));
        assert!(nav.step(true, false, None, at(310), false));
        assert!(!nav.dragging && nav.wants_repaint(false));
        assert!(!nav.step(true, false, None, at(495), false));
        assert_eq!(nav.progress, 0.);
        // Dragged past the threshold: the release turns list_open on, the page completes from where it was.
        nav.step(true, false, Some(0.5), at(600), false);
        assert!(nav.step(true, true, None, at(610), false));
        assert_eq!(nav.progress, 0.5);
        assert!(!nav.step(true, true, None, at(870), false));
        assert_eq!(nav.progress, 1.);
        // Reduce Motion: no page follows the finger, the switch is immediate.
        assert!(!nav.step(true, true, Some(0.5), at(800), true));
        assert!(!nav.step(true, false, None, at(810), true));
        assert_eq!(nav.progress, 0.);
        // Two columns (or no room): never animated.
        assert!(!nav.step(false, true, Some(0.5), at(900), false));
        assert_eq!(nav.progress, 1.);
        let mut forced = Nav { forced: Some(0.25), ..Default::default() };
        assert!(forced.step(true, true, None, t0, false));
        assert_eq!(forced.progress, 0.25);
    }

    #[test]
    fn back_drag_reports_the_finger_then_the_release() {
        let ctx = egui::Context::default();
        let mut frame = |events: Vec<egui::Event>| {
            let mut got = None;
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 800.))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| got = back_drag(ui.ctx()));
            out.textures_delta.clear();
            got
        };
        let button = |x: f32, pressed| egui::Event::PointerButton { pos: egui::pos2(x, 300.), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let moved = |x: f32| egui::Event::PointerMoved(egui::pos2(x, 300.));
        assert_eq!(frame(vec![moved(5.), button(5., true)]), None);
        assert_eq!(frame(vec![moved(9.)]), None, "not yet a drag");
        assert_eq!(frame(vec![moved(105.)]), Some(BackDrag::Moving(0.25)));
        assert_eq!(frame(vec![moved(205.)]), Some(BackDrag::Moving(0.5)));
        assert_eq!(frame(vec![button(205., false)]), Some(BackDrag::Released(true)));
        assert_eq!(frame(vec![moved(300.)]), None);
        // From the middle of the screen: a scroll, not «back».
        assert_eq!(frame(vec![moved(100.), button(100., true)]), None);
        assert_eq!(frame(vec![moved(300.)]), None);
        assert_eq!(frame(vec![button(300., false)]), None);
    }

    #[test]
    fn navigation_drag_progress_is_bounded() {
        assert_eq!(nav_drag_progress(-1., 400.), 0.);
        assert_eq!(nav_drag_progress(100., 400.), 0.25);
        assert_eq!(nav_drag_progress(500., 400.), 1.);
        assert_eq!(nav_drag_progress(10., 0.), 0.);
    }

    #[test]
    fn navigation_eases_to_target_or_skips_for_reduced_motion() {
        assert_eq!(nav_ease(0.25, 1., 0., 0.25, false), 0.25);
        assert_eq!(nav_ease(0.25, 1., 0.25, 0.25, false), 1.);
        assert_eq!(nav_ease(0.25, 1., 0., 0.25, true), 1.);
        assert_eq!(nav_ease(1., 0., 0.18, 0.18, false), 0.);
        assert!((nav_ease(0., 1., 0.125, 0.25, false) - 0.875).abs() < 0.0001);
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
                    rail_background(&ui, crate::chat_background(&t), 0.);
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
