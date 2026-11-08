//! Real Shell, private Wayland display: the verification popup with seven SAS emoji as painted (grim screenshot),
//! light + dark. Per emoji: the painted glyph is whole (its pixel box has a margin inside its cell, never touches the
//! cell edge) and the cells sit inside the dialog, the dialog inside the window. Also the keys: first button focused,
//! Tab trapped in the popup, Esc asks before leaving a comparison, Enter presses, the outcome closes with OK.
//! The runner sets the scale (GUI_SCALE=1|1.5|2 → 100/150/200%); PNGs land in $SW_OUT (default /tmp/gui-out):
//!   CT_SRC=$PWD the ct script 'GUI_SCALE=1.5 SW_OUT=/tmp/gui-out the gui-test-switch script "live" verification_popup'
use crate::*;
use gpui::{Bounds, Keystroke, Pixels};
use matrix_engine::verification::{VerificationAction, VerificationCommand, VerificationEvent, VerificationSnapshot, VerificationState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CELLS: Mutex<[Option<Bounds<Pixels>>; 7]> = Mutex::new([None; 7]);
static DIALOG: Mutex<Option<Bounds<Pixels>>> = Mutex::new(None);

/// Records the laid-out bounds of emoji cell `i` (its parent).
pub(crate) fn observe(i: usize) -> Option<gpui::AnyElement> {
    ACTIVE.load(Ordering::SeqCst).then(|| {
        gpui::canvas(move |b, _, _| CELLS.lock().unwrap()[i] = Some(b), |_, _, _, _| {}).absolute().inset_0().into_any_element()
    })
}

/// Records the laid-out bounds of the dialog box (its parent).
pub(crate) fn observe_dialog() -> Option<gpui::AnyElement> {
    ACTIVE.load(Ordering::SeqCst).then(|| {
        gpui::canvas(|b, _, _| *DIALOG.lock().unwrap() = Some(b), |_, _, _, _| {}).absolute().inset_0().into_any_element()
    })
}

/// The seven emoji the SDK delivered on the disposable homeserver (names are the SDK's).
fn sample() -> Vec<(String, String)> {
    [("💡", "Light Bulb"), ("🐱", "Cat"), ("🦋", "Butterfly"), ("🎅", "Santa"), ("🎧", "Headphone"), ("✏️", "Pencil"), ("✂️", "Scissors")]
        .into_iter()
        .map(|(s, n)| (s.to_owned(), n.to_owned()))
        .collect()
}

fn snapshot(account: matrix_domain::AccountId, state: VerificationState) -> VerificationSnapshot {
    VerificationSnapshot {
        account_id: account,
        user_id: "@amico:example.org".into(),
        device_id: "PEER".into(),
        flow_id: "popup-test".into(),
        state,
        device_verified: false,
        sas_emoji: Some(sample()),
    }
}

/// Smallest and largest painted coordinate (logical px) of pixels that differ from `bg` inside the box; None if empty.
fn ink(img: &image::RgbImage, scale: f32, bg: u32, x: (f32, f32), y: (f32, f32)) -> Option<(f32, f32, f32, f32)> {
    let want = [(bg >> 16) as u8, (bg >> 8) as u8, bg as u8];
    let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
    let to_px = |v: f32| (v * scale).round().max(0.) as u32;
    for py in to_px(y.0)..to_px(y.1).min(img.height()) {
        for px in to_px(x.0)..to_px(x.1).min(img.width()) {
            let p = img.get_pixel(px, py).0;
            if p.iter().zip(want).any(|(a, b)| a.abs_diff(b) > 14) {
                (x0, x1, y0, y1) = (x0.min(px), x1.max(px), y0.min(py), y1.max(py));
            }
        }
    }
    (x1 >= x0 && y1 >= y0).then(|| (x0 as f32 / scale, (x1 + 1) as f32 / scale, y0 as f32 / scale, (y1 + 1) as f32 / scale))
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
        wait(500).await;
        let scale = cx.update(|window, _| window.scale_factor()).unwrap();
        let mut fails: Vec<String> = vec![];
        macro_rules! check { ($c:expr, $($m:tt)+) => { if !$c { fails.push(format!("[x{scale}] {}", format!($($m)+))); } }; }
        macro_rules! key { ($k:expr) => { { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap(); wait(150).await; } }; }
        let account = this.update_in(cx, |this, window, cx| {
            this.connected = true;
            this.list_focus.focus(window);
            cx.notify();
            this.active_scope.account_id
        }).unwrap();
        let (w, h) = this.update_in(cx, |_, window, _| {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        }).unwrap();

        for (theme, name) in [(Theme::Classic, "light"), (Theme::Dark, "dark")] {
            let t = theme.tokens();
            this.update_in(cx, |this, _, cx| {
                this.set_theme(theme, cx);
                this.verification = matrix_desktop::DeviceVerification::new(account);
                // A request from the other side opens the popup; then the comparison starts.
                this.verification.apply(VerificationEvent::Snapshot(snapshot(account, VerificationState::Requested)));
                this.verification.apply(VerificationEvent::Snapshot(snapshot(account, VerificationState::Compare { decimals: (1, 2, 3) })));
                cx.notify();
            }).unwrap();
            wait(600).await;
            let dir = std::env::var("SW_OUT").unwrap_or_else(|_| "/tmp/gui-out".into());
            let path = format!("{dir}/popup-{name}-{scale}.png");
            let shot = std::process::Command::new("grim").arg(&path).status().map_err(|e| e.to_string()).and_then(|s| s.success().then_some(()).ok_or("grim failed".to_string()))
                .and_then(|()| image::open(&path).map_err(|e| e.to_string())).map(|i| i.to_rgb8());
            let img = match shot {
                Ok(img) => img,
                Err(e) => { check!(false, "{name}: {e}"); continue }
            };
            // Screenshot px per logical px: gpui rounds a fractional output scale up (1.5 → 2), so derive it.
            let k = img.width() as f32 / w;
            let dialog = DIALOG.lock().unwrap().expect("dialog never painted");
            let (dl, dr, dt, db) = (f32::from(dialog.left()), f32::from(dialog.right()), f32::from(dialog.top()), f32::from(dialog.bottom()));
            check!(dl >= 0. && dt >= 0. && dr <= w + 0.5 && db <= h + 0.5, "{name}: dialog {dl},{dt}..{dr},{db} outside the {w}x{h} window");
            let cells = *CELLS.lock().unwrap();
            for (i, cell) in cells.iter().enumerate() {
                let Some(c) = cell else { check!(false, "{name}: emoji cell {i} never painted"); continue };
                let (cl, cr, ct) = (f32::from(c.left()), f32::from(c.right()), f32::from(c.top()));
                check!(cl >= dl - 0.5 && cr <= dr + 0.5 && f32::from(c.bottom()) <= db + 0.5, "{name}: emoji cell {i} leaves the dialog");
                // The glyph line of the cell: its ink must stay clear of the box edges, or the emoji is cut.
                let (gt, gb) = (ct, ct + crate::verification_view::EMOJI_LINE);
                match ink(&img, k, t.surface, (cl, cr), (gt, gb)) {
                    None => check!(false, "{name}: emoji {i} not drawn (no pixel differs from the dialog background)"),
                    Some((x0, x1, y0, y1)) => {
                        let m = 1.0 / k;
                        check!(x1 - x0 >= 20. && y1 - y0 >= 20., "{name}: emoji {i} too small: {:.1}x{:.1} logical px", x1 - x0, y1 - y0);
                        check!(x0 > cl + m && x1 < cr - m && y0 > gt + m && y1 < gb - m, "{name}: emoji {i} touches its box ({x0:.1},{y0:.1}..{x1:.1},{y1:.1} in {cl:.1},{gt:.1}..{cr:.1},{gb:.1})");
                    }
                }
            }
            eprintln!("{name} x{scale}: dialog {dl:.0},{dt:.0}..{dr:.0},{db:.0} in {w}x{h}");
        }

        // Keys on the real window. First button focused; Tab stays inside the popup.
        let focus = |this: &Shell| this.verification.popup_focused();
        use matrix_desktop::PopupButton as B;
        this.update_in(cx, |this, _, _| check!(focus(this) == Some(B::Match), "initial focus is not «Corrispondono»: {:?}", focus(this))).unwrap();
        key!("tab");
        this.update_in(cx, |this, _, _| check!(focus(this) == Some(B::NoMatch), "Tab 1: {:?}", focus(this))).unwrap();
        key!("tab");
        this.update_in(cx, |this, _, _| check!(focus(this) == Some(B::Close), "Tab 2: {:?}", focus(this))).unwrap();
        key!("tab");
        this.update_in(cx, |this, _, _| check!(focus(this) == Some(B::Match), "Tab 3 must wrap inside the popup: {:?}", focus(this))).unwrap();
        key!("shift-tab");
        this.update_in(cx, |this, _, _| check!(focus(this) == Some(B::Close), "Shift+Tab: {:?}", focus(this))).unwrap();
        // Esc in the comparison asks first; Esc again goes back.
        key!("escape");
        this.update_in(cx, |this, _, _| {
            let v = this.verification.popup_view();
            check!(v.as_ref().is_some_and(|v| v.title == "Annullare la verifica?"), "Esc did not ask for confirmation: {v:?}");
        }).unwrap();
        key!("escape");
        this.update_in(cx, |this, _, _| {
            let v = this.verification.popup_view();
            check!(v.as_ref().is_some_and(|v| v.title == "Verifica" && v.buttons == vec![B::Match, B::NoMatch]), "second Esc did not return to the comparison: {v:?}");
        }).unwrap();
        // Enter presses the focused button (the first one again): «Corrispondono» → Confirm for the exact flow.
        key!("enter");
        this.update_in(cx, |this, _, _| {
            check!(
                matches!(this.verification.pending(), Some(VerificationCommand::Act { action: VerificationAction::Confirm, target }) if target.flow_id == "popup-test"),
                "Enter on «Corrispondono» queued no Confirm"
            )
        }).unwrap();
        // Outcome: OK closes, and the popup is gone from the window.
        this.update_in(cx, |this, _, cx| {
            this.verification.apply(VerificationEvent::Snapshot(snapshot(account, VerificationState::Verified)));
            cx.notify();
        }).unwrap();
        wait(200).await;
        this.update_in(cx, |this, _, _| check!(this.verification.popup_view().is_some_and(|v| v.buttons == vec![B::Ok]), "no OK on the outcome")).unwrap();
        key!("enter");
        this.update_in(cx, |this, _, _| check!(!this.verification.popup_open(), "OK did not close the popup")).unwrap();
        // Esc while waiting leaves at once and cancels the live flow.
        this.update_in(cx, |this, _, cx| {
            this.verification = matrix_desktop::DeviceVerification::new(account);
            let mut second = snapshot(account, VerificationState::Requested);
            second.flow_id = "popup-test-2".into();
            this.verification.apply(VerificationEvent::Snapshot(second));
            cx.notify();
        }).unwrap();
        wait(200).await;
        this.update_in(cx, |this, _, _| check!(this.verification.popup_open(), "an incoming request did not open the popup")).unwrap();
        key!("escape");
        this.update_in(cx, |this, _, _| {
            check!(!this.verification.popup_open(), "Esc on a request did not close the popup");
            check!(matches!(this.verification.pending(), Some(VerificationCommand::Act { action: VerificationAction::Cancel, .. })), "closing a live request queued no Cancel");
        }).unwrap();
        cx.update(|_, cx| cx.quit()).unwrap();
        FAILED.lock().unwrap().extend(fails);
    })
    .detach();
}

#[test]
#[ignore = "requires private display"]
fn verification_popup_painted_emoji_whole_and_keys() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn ink_box_finds_a_cut_glyph() {
    let mut img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
    for y in 10..30 {
        for x in 10..30 {
            img.put_pixel(x, y, image::Rgb([200, 30, 30]));
        }
    }
    assert_eq!(ink(&img, 1., 0xffffff, (0., 40.), (0., 40.)), Some((10., 30., 10., 30.)));
    // A box that ends inside the glyph sees ink on its last row: that is how a cut shows up.
    assert_eq!(ink(&img, 1., 0xffffff, (0., 40.), (0., 20.)).map(|b| b.3), Some(20.));
    assert_eq!(ink(&img, 1., 0xffffff, (0., 5.), (0., 5.)), None);
}
