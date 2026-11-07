//! Real Shell on a private display: the rail's focus ring is painted only after a key; the selected entry keeps its side notch
//! either way. Xvfb shows the GPUI window black, so the render path records what it draws (`record`) instead of reading pixels.
//!   the ct script 'the gui-test-x11 script "live,linux-a11y" rail_focus_ring'
use crate::*;
use gpui::Keystroke;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Per rail entry painted by the last render: (ring drawn, notch drawn).
static PAINTED: std::sync::Mutex<Vec<(bool, bool)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn record(ring: bool, notch: bool) {
    if ACTIVE.load(Ordering::SeqCst) {
        PAINTED.lock().unwrap().push((ring, notch));
    }
}

/// Repaint and return what the rail drew.
async fn paint(this: &gpui::WeakEntity<Shell>, cx: &mut gpui::AsyncWindowContext) -> Vec<(bool, bool)> {
    PAINTED.lock().unwrap().clear();
    this.update_in(cx, |_, _, cx| cx.notify()).unwrap();
    cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
    PAINTED.lock().unwrap().clone()
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
        wait(400).await;
        let mut fails: Vec<String> = vec![];
        macro_rules! check { ($c:expr, $($m:tt)+) => { if !$c { fails.push(format!($($m)+)); } }; }
        this.update_in(cx, |this, window, cx| {
            let mut c = matrix_desktop::AccountCatalog::default();
            let id = c.add("@anna:s.org", "https://s.org", "anna");
            this.accounts = c;
            this.connected = true;
            for (room, name) in [("!alpha:fixture", "Alpha"), ("!beta:fixture", "Beta")] {
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(Box::new(matrix_engine::RoomSummary {
                    scope: RoomScope { account_id: id, room_id: matrix_domain::RoomId::new(room) },
                    name: name.into(), unread: 0,
                    info: matrix_engine::RoomInfo { is_space: true, ..Default::default() },
                })));
            }
            this.rail_ui.target = this.sidebar.navigation_entries()[0].scope.clone();
            this.rail_ui.focus.focus(window);
            cx.notify();
        }).unwrap();
        wait(500).await;
        // Focus is on the rail but no key was pressed: no ring, the selected entry still shows its notch.
        let p = paint(&this, cx).await;
        check!(!p.is_empty() && p.iter().all(|e| !e.0) && p.iter().any(|e| e.1), "pointer/startup: {p:?}");
        cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx); }).unwrap();
        let p = paint(&this, cx).await;
        check!(p.iter().filter(|e| e.0).count() == 1 && p.iter().any(|e| e.1), "after a key: {p:?}");
        // Selection survives a ring-free repaint after the pointer takes over again.
        cx.update(|_, cx| input_modality::set_pointer(cx)).unwrap();
        let p = paint(&this, cx).await;
        check!(p.iter().all(|e| !e.0) && p.iter().any(|e| e.1), "back to pointer: {p:?}");
        cx.update(|_, cx| cx.quit()).unwrap();
        FAILED.lock().unwrap().extend(fails);
    })
    .detach();
}

#[test]
#[cfg(not(all(feature = "linux-a11y", target_os = "linux")))] // that build paints the rail through semantic observers
#[ignore = "requires private display"]
fn rail_focus_ring_only_after_a_key_and_selected_entry_keeps_its_notch() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
