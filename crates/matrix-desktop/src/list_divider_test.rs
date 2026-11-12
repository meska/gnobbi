//! Real-pointer test of the list divider: drag, limits, double click, window resize.
//!   the ct script 'the gui-test-wayland script "live" list_divider'   (screenshots in $SHOT_DIR)
use crate::{list_divider::*, rail_drag_test::{Input, pause}, rail_view::RAIL_WIDTH, *};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
const Y: i32 = 500;

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let ex = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        pause(&ex, 800).await;
        let mut input = Input::new();
        let width = |cx: &mut gpui::AsyncWindowContext| this.update_in(cx, |this, window, _| this.list_width(window)).unwrap();
        let saved = |cx: &mut gpui::AsyncWindowContext| this.update_in(cx, |this, _, _| {
            matrix_desktop::Preferences::load(this.prefs_dirs.as_ref().unwrap()).list_width
        }).unwrap();
        let edge = |w: f32| (RAIL_WIDTH + w) as i32;
        let vw = this.update_in(cx, |_, window, _| f32::from(window.viewport_size().width)).unwrap();
        eprintln!("viewport {vw}");
        assert_eq!(width(cx), 320., "default");

        // 1. Drag +60 in steps: live width, chat takes the rest, saved only on release.
        input.mv(edge(320.), Y);
        input.button(true);
        input.glide(&ex, (edge(320.), Y), (edge(380.), Y), 6).await;
        pause(&ex, 200).await;
        assert_eq!(width(cx), 380., "live width while dragging");
        assert_eq!(saved(cx), None, "nothing saved before the release");
        input.shot("divider", "drag");
        input.button(false);
        pause(&ex, 300).await;
        assert_eq!(saved(cx), Some(380));

        // 2. Limits: far right stops at the maximum, far left at the minimum.
        input.mv(edge(380.), Y);
        input.button(true);
        input.glide(&ex, (edge(380.), Y), (edge(380.) + 700, Y), 6).await;
        pause(&ex, 200).await;
        assert_eq!(width(cx), LIST_MAX_WIDTH);
        input.button(false);
        input.mv(edge(LIST_MAX_WIDTH), Y);
        input.button(true);
        input.glide(&ex, (edge(LIST_MAX_WIDTH), Y), (20, Y), 8).await;
        pause(&ex, 200).await;
        assert_eq!(width(cx), LIST_MIN_WIDTH);
        input.shot("divider", "min");
        input.button(false);
        pause(&ex, 300).await;
        assert_eq!(saved(cx), Some(304));

        // 3. Double click: back to the default, saved value cleared.
        let x = edge(LIST_MIN_WIDTH);
        input.mv(x, Y);
        for _ in 0..2 {
            input.button(true);
            input.button(false);
        }
        pause(&ex, 300).await;
        assert_eq!(width(cx), 320., "double click restores the default");
        assert_eq!(saved(cx), None);

        // 4. A wide saved width is re-clamped (not rewritten) when the window gets narrower.
        this.update_in(cx, |this, window, cx| {
            this.set_list_width(Some(480.), window, cx);
            window.resize(gpui::size(px(900.), window.viewport_size().height));
        }).unwrap();
        pause(&ex, 500).await;
        let (w, vw) = this.update_in(cx, |this, window, _| (this.list_width(window), f32::from(window.viewport_size().width))).unwrap();
        assert_eq!(w, effective_width(480., vw));
        assert!(w + RAIL_WIDTH + 360. <= vw.max(RAIL_WIDTH + LIST_MIN_WIDTH + 360.), "chat keeps its minimum");
        input.shot("divider", "narrow");
        eprintln!("DIVIDER ok");
        this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
    })
    .detach();
}

#[cfg(test)]
#[test]
#[ignore = "requires private display"]
fn list_divider_real_pointer_drag_limits_double_click_resize() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}
