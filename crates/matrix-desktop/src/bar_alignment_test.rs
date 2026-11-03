//! Rendered-tree check: the list's bottom bar and the composer bar share top edge, bottom edge and height (±1 px).
//! Measures what was painted, never a token value, so it fails today and passes once the bars share one height.
//! Private display, harness sets scale and theme:
//!   the ct script 'BAR_THEME=dark the gui-test-x11 script "live" bars_align_at_rest'   (BAR_THEME=light|dark)
//! Scale 100/150/200%: sway output scale (the gui-test script, wayland). macOS titlebar: `titlebar_inset` only exists on macOS,
//! where libtest cannot own the main thread, so that case is not covered here.
use crate::*;
use gpui::{Bounds, EntityInputHandler, Pixels};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIST_BAR: std::sync::Mutex<Option<Bounds<Pixels>>> = std::sync::Mutex::new(None);

/// Records the painted bounds of the parent (the list footer).
pub(crate) fn observe() -> Option<gpui::AnyElement> {
    ACTIVE.load(Ordering::SeqCst).then(|| {
        gpui::canvas(|_, _, _| (), |b, _, _, _| *LIST_BAR.lock().unwrap() = Some(b))
            .absolute()
            .inset_0()
            .into_any_element()
    })
}

/// (top, bottom, height) in logical px.
type Edges = (f32, f32, f32);
fn edges(b: Bounds<Pixels>) -> Edges {
    (f32::from(b.top()), f32::from(b.bottom()), f32::from(b.size.height))
}

fn assert_aligned(list: Edges, composer: Edges, scale: f32) {
    // 1 px is physical: 1/scale logical.
    let tol = 1.0 / scale + 1e-3;
    for (what, a, b) in [("top", list.0, composer.0), ("bottom", list.1, composer.1), ("height", list.2, composer.2)] {
        assert!((a - b).abs() <= tol, "{what}: list bar {a} vs composer bar {b} (scale {scale}, tolerance {tol})");
    }
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let scale = window.scale_factor();
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        executor.timer(std::time::Duration::from_millis(400)).await;
        if std::env::var("BAR_THEME").as_deref() == Ok("dark") {
            this.update_in(cx, |this, _, cx| this.set_theme(Theme::Dark, cx)).unwrap();
            executor.timer(std::time::Duration::from_millis(300)).await;
        }
        let measure = || {
            let list = edges(LIST_BAR.lock().unwrap().expect("list bar was never painted"));
            (list, edges(crate::composer_view::painted_bar()))
        };
        let (list, composer) = measure();
        eprintln!("scale {scale}: list bar {list:?} composer bar {composer:?}");
        let rest = std::panic::catch_unwind(|| assert_aligned(list, composer, scale));
        // The composer grows only upward: the list bar and the composer's bottom edge stay put.
        this.update_in(cx, |this, window, cx| {
            this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..65536), "uno\ndue\ntre\nquattro\ncinque", window, cx));
        })
        .unwrap();
        executor.timer(std::time::Duration::from_millis(400)).await;
        let (list2, composer2) = measure();
        eprintln!("multi-line: list bar {list2:?} composer bar {composer2:?}");
        let grown = std::panic::catch_unwind(|| {
            assert_aligned(list, list2, scale);
            assert!(composer2.2 > composer.2, "composer should have grown: {composer:?} -> {composer2:?}");
            assert!((composer2.1 - composer.1).abs() <= 1.0 / scale + 1e-3, "composer bottom moved: {composer:?} -> {composer2:?}");
        });
        cx.update(|_, cx| cx.quit()).unwrap();
        // Quit first, then fail: a panic inside the app loop would hide the message.
        FAILED.lock().unwrap().extend([rest, grown].into_iter().filter_map(|r| r.err()).map(|e| {
            e.downcast_ref::<String>().cloned().unwrap_or_else(|| "assertion".into())
        }));
    })
    .detach();
}

static FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn run() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
#[ignore = "requires private display"]
fn bars_align_at_rest_and_composer_grows_upward() {
    run();
}

#[test]
fn checker_rejects_the_14px_gap_and_accepts_1px() {
    assert!(std::panic::catch_unwind(|| assert_aligned((600., 672., 72.), (586., 672., 86.), 1.)).is_err());
    assert_aligned((600., 672., 72.), (600.5, 672.5, 72.), 1.);
    assert!(std::panic::catch_unwind(|| assert_aligned((600., 672., 72.), (602., 674., 72.), 2.)).is_err());
}
