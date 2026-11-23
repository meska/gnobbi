//! Phone facts in one place: safe-area insets, on-screen keyboard height, touch targets. The rest of the UI only reads
//! `content_rect()` and `target()`. On desktop all of it is zero/off unless the test hooks below simulate a phone.
use eframe::egui;
use std::sync::atomic::{AtomicU32, Ordering};

/// Smallest side of anything tappable on a touch screen (iOS HIG 44 pt, Android 48 dp).
const TOUCH: f32 = if cfg!(target_os = "android") { 48. } else { 44. };

/// Home indicator height iOS reports in landscape on notch phones.
const LANDSCAPE_HOME: f32 = 21.;

static KEYBOARD: AtomicU32 = AtomicU32::new(0);

/// Points of the window bottom covered by the on-screen keyboard (0 = closed). The platform observer calls this from any thread.
pub fn set_keyboard(points: f32) {
    KEYBOARD.store(points.max(0.).to_bits(), Ordering::Relaxed);
}

/// Points of the window bottom the keyboard covers now (0 = closed).
pub fn keyboard() -> f32 {
    f32::from_bits(KEYBOARD.load(Ordering::Relaxed))
}

/// Points of the window bottom covered by a keyboard whose end frame (already in window coordinates) starts at `frame.0` and is
/// `frame.1` wide, in a window of `screen` = (width, height). None for the transient frame iOS posts in the rotated space (width =
/// window height, y = 67): a frame that is not as wide as the window is not the keyboard, keep the previous value.
#[cfg(any(target_os = "ios", test))]
fn covered_by_keyboard(screen: (f64, f64), frame: (f64, f64)) -> Option<f32> {
    ((frame.1 - screen.0).abs() <= 1.).then(|| (screen.1 - frame.0).max(0.) as f32)
}

/// The app's own normal-level window (`keyWindow` is a system window of another size during keyboard transitions).
#[cfg(target_os = "ios")]
fn app_window(mtm: objc2::MainThreadMarker) -> Option<objc2::rc::Retained<objc2_ui_kit::UIWindow>> {
    objc2_ui_kit::UIApplication::sharedApplication(mtm).windows().iter().find(|w| w.windowLevel() == 0. && !w.isHidden())
}

/// The view egui draws in (winit's, the root view controller's), else the window: the keyboard is converted into ITS coordinates.
#[cfg(target_os = "ios")]
fn egui_view(w: objc2::rc::Retained<objc2_ui_kit::UIWindow>) -> objc2::rc::Retained<objc2_ui_kit::UIView> {
    w.rootViewController().and_then(|c| c.view()).unwrap_or_else(|| objc2::rc::Retained::into_super(w))
}

/// UIEdgeInsets, without the UIGeometry feature.
#[cfg(target_os = "ios")]
#[repr(C)]
struct Edge { top: f64, left: f64, bottom: f64, right: f64 }
#[cfg(target_os = "ios")]
unsafe impl objc2::Encode for Edge {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct("UIEdgeInsets", &[f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING]);
}

/// What UIKit says now about the window and the egui view, for the overlay: window size, view size, view origin y, window safe top/bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiKitFacts { pub window: (f64, f64), pub view: (f64, f64), pub view_y: f64, pub safe: (f64, f64) }

#[cfg(target_os = "ios")]
fn uikit_facts() -> Option<UiKitFacts> {
    use objc2::msg_send;
    let mtm = objc2::MainThreadMarker::new()?;
    let w = app_window(mtm)?;
    let (window, safe): (_, Edge) = (w.bounds().size, unsafe { msg_send![&*w, safeAreaInsets] });
    let view = egui_view(w);
    Some(UiKitFacts { window: (window.width, window.height), view: (view.bounds().size.width, view.bounds().size.height), view_y: view.frame().origin.y, safe: (safe.top, safe.bottom) })
}
#[cfg(not(target_os = "ios"))]
fn uikit_facts() -> Option<UiKitFacts> { None }

/// iOS: follows the on-screen keyboard (every will/did show, frame change and hide) into `set_keyboard`. Call once at startup, on the main thread.
#[cfg(target_os = "ios")]
pub fn observe_keyboard(ctx: &egui::Context) {
    use objc2::{MainThreadMarker, msg_send, runtime::AnyObject};
    use objc2_core_foundation::CGRect;
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use objc2_ui_kit::{UIKeyboardFrameEndUserInfoKey, UIScreen, UIKeyboardDidChangeFrameNotification, UIKeyboardDidHideNotification, UIKeyboardDidShowNotification, UIKeyboardWillChangeFrameNotification, UIKeyboardWillHideNotification, UIKeyboardWillShowNotification};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let center = NSNotificationCenter::defaultCenter();
    let on = |name: &objc2_foundation::NSNotificationName, source: &'static str| {
        let ctx = ctx.clone();
        let block = block2::RcBlock::new(move |n: std::ptr::NonNull<NSNotification>| {
            let n = unsafe { n.as_ref() };
            // The end frame is in SCREEN coordinates, the layout is in egui's view: convert it (rotation, a window that is not
            // the screen, Display Zoom).
            let frame = n.userInfo().and_then(|u| u.objectForKey(unsafe { UIKeyboardFrameEndUserInfoKey }));
            let Some(v) = frame else { return };
            let v: &AnyObject = &v;
            let f: CGRect = unsafe { msg_send![v, CGRectValue] };
            let screen = UIScreen::mainScreen(mtm);
            let (bounds, local) = match app_window(mtm) {
                Some(w) => {
                    let view = egui_view(w);
                    let space: *mut AnyObject = unsafe { msg_send![&*screen, coordinateSpace] };
                    let local: CGRect = unsafe { msg_send![&*view, convertRect: f, fromCoordinateSpace: space] };
                    (view.bounds(), local)
                }
                None => (screen.bounds(), f),
            };
            let covered = if source.ends_with("Hide") { Some(0.) } else { covered_by_keyboard((bounds.size.width, bounds.size.height), (local.min().y, local.size.width)) };
            let Some(covered) = covered else { return };
            set_keyboard(covered);
            ctx.request_repaint();
        });
        // The centre keeps the observer; it lives as long as the app.
        let _ = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
    };
    // Will* come first with the end frame; Did* repeat the settled one (QuickType can arrive after WillShow).
    unsafe {
        on(UIKeyboardWillShowNotification, "WillShow");
        on(UIKeyboardWillChangeFrameNotification, "WillChange");
        on(UIKeyboardDidShowNotification, "DidShow");
        on(UIKeyboardDidChangeFrameNotification, "DidChange");
        on(UIKeyboardWillHideNotification, "WillHide");
        on(UIKeyboardDidHideNotification, "DidHide");
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
    use std::time::{Duration, Instant};

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
        crate::suspend::stop_all();
        released(generation, Instant::now() + Duration::from_secs(8), mtm);
    }
    // Never sleep on the main thread (a late timer or the expiration handler runs here, and a block of 10 s is a watchdog kill):
    // look again every 50 ms until every worker has dropped its stores.
    fn released(generation: usize, deadline: Instant, mtm: MainThreadMarker) {
        if crate::suspend::all_released() {
            return end(mtm);
        }
        if Instant::now() < deadline {
            let again = Box::into_raw(Box::new((generation, deadline))) as *mut c_void;
            unsafe { dispatch_after_f(dispatch_time(0, 50_000_000), &raw const _dispatch_main_q as *const c_void, again, poll); }
            return;
        }
        // Keeping the task open lets iOS terminate rather than suspend with a shared-store lock.
        eprintln!("Gnobbi lifecycle: worker did not release its lock");
        if LIFE.with(|l| l.borrow().2 == Some(unsafe { UIBackgroundTaskInvalid })) {
            std::process::abort();
        }
    }
    extern "C" fn poll(context: *mut c_void) {
        // SAFETY: `context` is the Box leaked by `released` for exactly this call.
        let (generation, deadline) = *unsafe { Box::from_raw(context as *mut (usize, Instant)) };
        if let Some(mtm) = MainThreadMarker::new() { released(generation, deadline, mtm); }
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

/// iOS and Android, or the test hook GNOBBI_EGUI_TOUCH on desktop.
pub fn touch() -> bool {
    static TOUCH_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TOUCH_ON.get_or_init(|| cfg!(any(target_os = "ios", target_os = "android")) || matrix_desktop::hooks::var_os("GNOBBI_EGUI_TOUCH").is_some())
}

/// Side of a tappable target: `desktop` as it is, at least 44 pt on touch.
pub fn target(desktop: f32) -> f32 {
    if touch() { desktop.max(TOUCH) } else { desktop }
}

/// Convert UIKit logical points to egui points, retaining fractional points until rendering.
fn uikit_pt_to_egui(x: f32, ratio: f32) -> f32 { x * ratio }

fn uikit_ratio(screen_width: f32, view_width: f64) -> f32 {
    let ratio = screen_width / view_width as f32;
    if ratio.is_finite() && ratio > 0. { ratio } else { 1. }
}

/// Bottom band the layout leaves free: the keyboard when it is up (it already covers the home indicator), else the home indicator.
/// Never the sum, and never the safe inset above a keyboard: that was a blank band between the content and the keys.
fn bottom_reserved(safe_bottom: f32, keyboard: f32) -> f32 {
    if keyboard > 0. { keyboard } else { safe_bottom }
}

/// Android: pixels covered by the system bars and the display cutout, [top, left, bottom, right], then the keyboard height; None until the window has insets.
/// Reads the decor view's root insets over JNI (NativeActivity has no Java code of its own), the max of bars and cutout per side.
/// shortcut: called from the render thread, not the UI thread (reads only, works in practice); every frame, cache it if profiling shows a cost.
#[cfg(target_os = "android")]
fn android_insets() -> Option<[f32; 5]> {
    use jni::{jni_sig, jni_str, objects::JObject};
    let ctx = std::panic::catch_unwind(ndk_context::android_context).ok()?;
    // ndk_context holds the Application (android-activity 0.6), which has no getWindow: the Activity comes from the AndroidApp.
    let activity_ptr = crate::ANDROID_APP.get()?.activity_as_ptr();
    // SAFETY: the VM pointer comes from android-activity; from_raw is idempotent.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let res = vm.attach_current_thread(|env| -> Result<Option<[f32; 5]>, jni::errors::Error> {
        // SAFETY: the activity is a global ref owned by android-activity; JObject has no Drop and is not kept.
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };
        let window = env.call_method(&activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?.l()?;
        let decor = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?.l()?;
        let insets = env.call_method(&decor, jni_str!("getRootWindowInsets"), jni_sig!("()Landroid/view/WindowInsets;"), &[])?.l()?;
        if insets.is_null() {
            return Ok(None);
        }
        let mut out = [0f32; 5];
        for (i, name) in [jni_str!("getSystemWindowInsetTop"), jni_str!("getSystemWindowInsetLeft"), jni_str!("getSystemWindowInsetBottom"), jni_str!("getSystemWindowInsetRight")].into_iter().enumerate() {
            out[i] = env.call_method(&insets, name, jni_sig!("()I"), &[])?.i()? as f32;
        }
        // getDisplayCutout is API 28+ and null without a cutout: a failure here only means no cutout.
        let cutout = env.call_method(&insets, jni_str!("getDisplayCutout"), jni_sig!("()Landroid/view/DisplayCutout;"), &[]).and_then(|v| v.l());
        if let Some(cutout) = cutout.ok().filter(|c| !c.is_null()) {
            for (i, name) in [jni_str!("getSafeInsetTop"), jni_str!("getSafeInsetLeft"), jni_str!("getSafeInsetBottom"), jni_str!("getSafeInsetRight")].into_iter().enumerate() {
                out[i] = out[i].max(env.call_method(&cutout, name, jni_sig!("()I"), &[])?.i()? as f32);
            }
        }
        // Keyboard height: WindowInsets.getInsets(Type.ime()).bottom, API 30+; older devices keep 0 (the bottom system inset already includes it).
        let ime = env.call_static_method(jni_str!("android/view/WindowInsets$Type"), jni_str!("ime"), jni_sig!("()I"), &[]).and_then(|t| t.i())
            .and_then(|t| env.call_method(&insets, jni_str!("getInsets"), jni_sig!("(I)Landroid/graphics/Insets;"), &[t.into()])).and_then(|v| v.l())
            .and_then(|i| env.get_field(&i, jni_str!("bottom"), jni_sig!("I"))).and_then(|v| v.i());
        out[4] = ime.unwrap_or(0) as f32;
        Ok(Some(out))
    });
    // Log each distinct outcome once: this runs every frame and used to swallow errors silently.
    static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let msg = format!("{res:?}");
    if let Ok(mut last) = LAST.lock() && *last != msg {
        log::info!("gnobbi insets: {msg}");
        *last = msg;
    }
    res.ok().flatten()
}

/// Android: dark system-bar icons on a light theme, light ones on a dark theme (`MainActivity.setBarIcons`, which hops to the UI thread).
#[cfg(target_os = "android")]
pub fn set_bar_icons(light_theme: bool) {
    use jni::{jni_sig, jni_str, objects::JObject};
    let (Ok(ctx), Some(app)) = (std::panic::catch_unwind(ndk_context::android_context), crate::ANDROID_APP.get()) else { return };
    // SAFETY: as in android_insets.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let res = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
        // SAFETY: the activity is a global ref owned by android-activity; JObject has no Drop and is not kept.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        env.call_method(&activity, jni_str!("setBarIcons"), jni_sig!("(Z)V"), &[light_theme.into()]).map(|_| ())
    });
    if let Err(e) = res {
        log::warn!("gnobbi bar icons: {e:?}");
    }
}

/// The insets of the last window event, with the keyboard folded into the bottom one.
#[derive(Default)]
pub struct Insets(egui::SafeAreaInsets, bool);

impl Insets {
    /// Test hooks: GNOBBI_EGUI_INSETS=<top,left,bottom,right> and GNOBBI_EGUI_KEYBOARD=<points> simulate a phone on desktop.
    pub fn from_env() -> Self {
        let mut insets = Self::default();
        if let Ok(v) = matrix_desktop::hooks::var("GNOBBI_EGUI_INSETS") {
            let mut n = v.split(',').map(|p| p.trim().parse().unwrap_or(0.));
            let m = &mut insets.0.0;
            (m.top, m.left, m.bottom, m.right) = (n.next().unwrap_or(0.), n.next().unwrap_or(0.), n.next().unwrap_or(0.), n.next().unwrap_or(0.));
        }
        if let Some(k) = matrix_desktop::hooks::var("GNOBBI_EGUI_KEYBOARD").ok().and_then(|v| v.parse().ok()) {
            set_keyboard(k);
        }
        insets
    }

    /// Called from `raw_input_hook`. egui-winit sends the insets only with a window event (resize, rotation, focus): keep the last ones.
    /// Returns true when the screen just turned between portrait and landscape.
    pub fn apply(&mut self, raw: &mut egui::RawInput, pixels_per_point: f32) -> bool {
        #[cfg(target_os = "android")]
        if let Some(px) = android_insets() {
            raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: px[0] / pixels_per_point, left: px[1] / pixels_per_point, bottom: px[2] / pixels_per_point, right: px[3] / pixels_per_point }));
            set_keyboard(px[4] / pixels_per_point);
        }
        #[cfg(not(target_os = "android"))]
        let _ = pixels_per_point;
        let ratio = raw.screen_rect.zip(uikit_facts()).map_or(1., |(screen, kit)| uikit_ratio(screen.width(), kit.view.0));
        self.apply_with_ratio(raw, ratio)
    }

    fn apply_with_ratio(&mut self, raw: &mut egui::RawInput, ratio: f32) -> bool {
        if let Some(real) = raw.safe_area_insets {
            self.0 = real;
        }
        let mut all = self.0;
        let landscape = raw.screen_rect.is_some_and(|r| r.width() > r.height());
        // iOS keeps reporting the portrait insets after a rotation (top = notch, sides 0, bottom = home bar): in landscape the notch is on a side
        // (both sides, for symmetry), nothing is left on top and the home indicator is the short one.
        // shortcut: assumes insets left+right == 0 means stale and top > 40 means a notch (SE top is 20); drop it if UIKit starts reporting the sides.
        if cfg!(not(target_os = "android")) && landscape && all.0.left + all.0.right == 0. && all.0.top > 40. {
            (all.0.left, all.0.right, all.0.top, all.0.bottom) = (all.0.top, all.0.top, 0., LANDSCAPE_HOME);
        }
        // egui-winit 0.37 forwards UIKit safeAreaInsets unscaled; keep the cached values in UIKit units.
        all.0.top = uikit_pt_to_egui(all.0.top, ratio);
        all.0.bottom = uikit_pt_to_egui(all.0.bottom, ratio);
        all.0.left = uikit_pt_to_egui(all.0.left, ratio);
        all.0.right = uikit_pt_to_egui(all.0.right, ratio);
        let keyboard = uikit_pt_to_egui(f32::from_bits(KEYBOARD.load(Ordering::Relaxed)), ratio);
        let safe = all.0.bottom;
        all.0.bottom = bottom_reserved(safe, keyboard);
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
    {
        // Test hook, debug builds only: GNOBBI_EGUI_REDUCE_MOTION=1.
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| cfg!(debug_assertions) && matrix_desktop::hooks::var_os("GNOBBI_EGUI_REDUCE_MOTION").is_some())
    }
}

/// Test hook: GNOBBI_EGUI_NAV_PROGRESS=<0..1> holds the one-column navigation (chat over list, or the drawer on top) at that point, the page under it drawn too.
pub fn nav_forced() -> Option<f32> {
    static FORCED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| matrix_desktop::hooks::var("GNOBBI_EGUI_NAV_PROGRESS").ok().and_then(|v| v.parse::<f32>().ok()).map(|p| p.clamp(0., 1.)))
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

/// Width of the strip at the reading-side edge where the back swipe starts.
pub const BACK_EDGE: f32 = 24.;

/// `p` is in that strip: a press there belongs to the back swipe (the message long press leaves it alone).
pub fn in_back_edge(ctx: &egui::Context, p: egui::Pos2) -> bool {
    let content = ctx.content_rect();
    (if crate::dir::rtl() { content.right() - p.x } else { p.x - content.left() }) < BACK_EDGE
}

/// Forget the back swipe in progress (something else took the gesture); the next one starts with a new touch.
pub fn cancel_back_drag(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<Option<(egui::Pos2, f64, bool)>>(egui::Id::new("back-swipe")));
}

/// A light tick under the finger (iOS; nothing elsewhere). Main thread only, like every caller in the UI.
pub fn haptic_tick() {
    #[cfg(target_os = "ios")]
    {
        use objc2::{class, msg_send, rc::{Allocated, Retained}, runtime::AnyObject};
        // SAFETY: plain UIKit calls on the main thread; UIImpactFeedbackStyleLight = 0.
        unsafe {
            let raw: Allocated<AnyObject> = msg_send![class!(UIImpactFeedbackGenerator), alloc];
            let tick: Retained<AnyObject> = msg_send![raw, initWithStyle: 0isize];
            let _: () = msg_send![&*tick, impactOccurred];
        }
    }
}

/// The iOS back gesture, frame by frame: a touch that went down at the left edge and moves right.
pub fn back_drag(ctx: &egui::Context) -> Option<BackDrag> {
    const EDGE: f32 = BACK_EDGE;
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

    /// The keyboard height is a process-wide static: tests that set or read it run one at a time.
    static KEYBOARD_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn uikit_units_follow_egui_zoom_without_rounding() {
        assert_eq!(uikit_pt_to_egui(347., 1.), 347.);
        assert!((uikit_pt_to_egui(347., 383. / 440.) - 302.04773).abs() < 0.001);
        assert!((uikit_pt_to_egui(1., 1. / 1.15) - 0.8695652).abs() < 0.00001);
        assert_eq!(uikit_ratio(0., 0.), 1.);
        assert_eq!(uikit_ratio(383., 0.), 1.);
    }

    #[test]
    fn uikit_scaled_safe_area_and_keyboard_align_composer() {
        let _one = KEYBOARD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let ratio = 383. / 440.;
        let mut insets = Insets::default();
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(383., 956. * ratio))), safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 62., bottom: 34., left: 3., right: 4. })), ..Default::default() };
        set_keyboard(0.);
        insets.apply_with_ratio(&mut raw, ratio);
        let safe = raw.safe_area_insets.unwrap().0;
        assert!((safe.top - 62. * ratio).abs() < 0.001);
        assert!((safe.bottom - 34. * ratio).abs() < 0.001);
        assert!((safe.left - 3. * ratio).abs() < 0.001);
        set_keyboard(347.);
        raw.safe_area_insets = None;
        insets.apply_with_ratio(&mut raw, ratio);
        let keyboard_top = uikit_pt_to_egui(609., ratio);
        let mut composer_bottom = 0.;
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(raw, |ui| {
            let mut ui = safe_ui(ui, egui::Color32::BLACK);
            composer_bottom = crate::composer::panel(egui::Frame::NONE).show(&mut ui, |_| {}).response.rect.bottom();
        });
        out.textures_delta.clear();
        assert!((keyboard_top - composer_bottom).abs() <= 1., "{keyboard_top} vs {composer_bottom}");
        set_keyboard(0.);
    }

    #[test]
    fn keyboard_replaces_the_home_indicator_band_instead_of_adding_to_it() {
        assert_eq!(bottom_reserved(34., 336.), 336.);
        assert_eq!(bottom_reserved(300., 336.), 336., "a stale or oversized safe inset never shows above the keys");
        assert_eq!(bottom_reserved(34., 0.), 34.);
    }


    #[test]
    fn phone_list_extends_clip_and_reserves_the_bottom_inset() {
        let _one = KEYBOARD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
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
        insets.apply(&mut raw, 1.);
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
        insets.apply(&mut raw, 1.);
        let m = raw.safe_area_insets.unwrap().0;
        assert_eq!((m.left, m.right, m.top, m.bottom), (62., 62., 0., 21.));
        let mut se = egui::RawInput { screen_rect: raw.screen_rect, safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top: 20., ..Default::default() })), ..Default::default() };
        insets.apply(&mut se, 1.);
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
    fn keyboard_inset_follows_the_window_it_is_converted_into() {
        // Landscape: the keyboard is as wide as the window (874) and 200 high.
        assert_eq!(covered_by_keyboard((874., 402.), (202., 874.)), Some(200.));
        // A window shorter than the screen (frame already converted into it): only the part inside the window counts.
        assert_eq!(covered_by_keyboard((402., 800.), (539., 402.)), Some(261.));
        // The composer sits on the keyboard, never above the safe inset of the closed state, with the keyboard of any height.
        for k in [200., 291., 335., 336.] {
            assert_eq!(bottom_reserved(34., k), k);
        }
        assert_eq!(bottom_reserved(34., 0.), 34.);
    }

    #[test]
    fn content_stays_in_the_safe_area_and_the_composer_rides_the_keyboard() {
        let _one = KEYBOARD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let phone = |top, bottom| egui::SafeAreaInsets(egui::epaint::MarginF32 { top, bottom, ..Default::default() });
        let ctx = egui::Context::default();
        let mut insets = Insets::default();
        assert_eq!(frame(&ctx, &mut insets, None), (0., 852.), "desktop: no insets, nothing moves");
        assert_eq!(frame(&ctx, &mut insets, Some(phone(59., 34.))), (59., 818.));
        assert_eq!(frame(&ctx, &mut insets, None), (59., 818.), "a frame without a window event keeps the insets");
        set_keyboard(336.);
        assert_eq!(frame(&ctx, &mut insets, None), (59., 516.), "keyboard open: the composer sits on it");
        // No consumer adds the home-bar band above the keys: the list helper reserves nothing while the keyboard is up.
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(393., 852.))), ..Default::default() };
        insets.apply(&mut raw, 1.);
        let mut out = ctx.run_ui(raw, |ui| {
            let mut ui = safe_ui(ui, egui::Color32::BLACK);
            assert_eq!(extend_list(&mut ui, true).1, 0.);
        });
        out.textures_delta.clear();
        set_keyboard(0.);
        assert_eq!(frame(&ctx, &mut insets, None), (59., 818.), "keyboard closed: back above the home bar");
        assert_eq!(frame(&ctx, &mut insets, Some(phone(0., 21.))), (0., 831.), "rotation: the new insets replace the old");
    }

}
