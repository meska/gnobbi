//! Real Shell, private display: the drawer's initial row, the key-only focus ring flag and what an account change does
//! with the drawer open. Ring colour is computed inside render, so the test pins its inputs (`drawer_keys`, `drawer_row`,
//! focus) and the accessible rows, not pixels.
//!   the ct script 'the gui-test-x11 script "live,linux-a11y" drawer_active_row'
use crate::*;
use gpui::Keystroke;
use matrix_domain::AccountId;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn current(this: &Shell) -> Vec<String> {
    this.drawer_accounts().into_iter().filter(|a| a.current).map(|a| a.name).collect()
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
        macro_rules! key { ($k:expr) => { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap() }; }
        let (anna, bob) = this.update_in(cx, |this, _, _| {
            let mut c = matrix_desktop::AccountCatalog::default();
            let a = c.add("@anna:s.org", "https://s.org", "anna");
            let b = c.add("@bob:s.org", "https://s.org", "bob");
            this.accounts = c;
            this.connected = true;
            this.prefs_dirs = None; // opening the drawer would reload the catalog from disk
            this.active_id = Some(b); // not first in the catalog
            (a, b)
        }).unwrap();
        let _: (AccountId, AccountId) = (anna, bob);

        // 1. Mouse-style open (set_menu_open, as the menu button does): row = shown account, no ring, focus in the drawer.
        this.update_in(cx, |this, window, cx| {
            this.menu_focus.focus(window);
            this.set_menu_open(true, window, cx);
            check!(current(this) == ["bob"], "exactly the shown account is current: {:?}", current(this));
            check!(this.drawer_row == 1, "initial row is the shown account (bob, index 1), got {}", this.drawer_row);
            check!(!this.drawer_keys, "no focus ring after a pointer open");
            check!(this.drawer_focus.is_focused(window), "drawer holds focus after open");
        }).unwrap();
        // 2. First key: ring appears and the row moves from the shown account.
        key!("down");
        this.update_in(cx, |this, _, _| {
            check!(this.drawer_keys, "ring flag set by the first key");
            check!(this.drawer_row == 2, "Down moves from the shown row, got {}", this.drawer_row);
        }).unwrap();
        // 3. Close, reopen with the pointer: ring gone again, row back on the shown account.
        this.update_in(cx, |this, window, cx| {
            this.set_menu_open(false, window, cx);
            this.set_menu_open(true, window, cx);
            check!(!this.drawer_keys && this.drawer_row == 1, "reopen resets ring ({}) and row ({})", this.drawer_keys, this.drawer_row);
        }).unwrap();
        // 4. Keyboard open (Ctrl+Shift+M): ring on from the start, still on the shown account.
        this.update_in(cx, |this, window, cx| { this.set_menu_open(false, window, cx); this.menu_focus.focus(window); }).unwrap();
        key!("ctrl-shift-m");
        wait(200).await;
        this.update_in(cx, |this, window, _| {
            check!(this.menu_open && this.drawer_focus.is_focused(window), "shortcut opens the drawer with focus");
            check!(this.drawer_keys && this.drawer_row == 1, "keyboard open: ring {} row {}", this.drawer_keys, this.drawer_row);
        }).unwrap();
        // 5. Account change with the drawer open: refused switch keeps the drawer, error on that row, same account shown.
        this.update_in(cx, |this, window, cx| {
            this.switch_account(anna, window, cx); // anna is not parked: refused
            check!(this.menu_open && this.active_id == Some(bob), "refused switch keeps drawer open and bob shown");
            check!(this.switch_error.is_some_and(|(id, _)| id == anna), "refusal reported on anna's row");
            check!(current(this) == ["bob"], "marker stays on bob: {:?}", current(this));
            // The shown account clicked again just closes the drawer.
            this.switch_account(bob, window, cx);
            check!(!this.menu_open, "clicking the shown account closes the drawer");
            // What a successful switch leaves behind: anna shown. Reopen: marker and row follow, bob unmarked.
            this.active_id = Some(anna);
            this.set_menu_open(true, window, cx);
            check!(current(this) == ["anna"], "marker follows the account: {:?}", current(this));
            check!(this.drawer_row == 0 && !this.drawer_keys, "row {} keys {}", this.drawer_row, this.drawer_keys);
            // Account changes under an open drawer: the marker moves with it (rows are derived, not cached).
            this.active_id = Some(bob);
            check!(current(this) == ["bob"], "live marker move: {:?}", current(this));
        }).unwrap();
        // 6. Single account and no shown account.
        this.update_in(cx, |this, window, cx| {
            this.set_menu_open(false, window, cx);
            let mut c = matrix_desktop::AccountCatalog::default();
            let only = c.add("@anna:s.org", "https://s.org", "anna");
            this.accounts = c;
            this.active_id = Some(only);
            this.set_menu_open(true, window, cx);
            check!(current(this) == ["anna"] && this.drawer_row == 0, "single account: {:?} row {}", current(this), this.drawer_row);
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                let s = this.semantic_drawer().expect("drawer snapshot");
                check!(s.rows.len() == 1 && s.rows[0].current && s.rows[0].label().ends_with("account attivo"), "single row label");
            }
            this.set_menu_open(false, window, cx);
            this.active_id = None;
            this.set_menu_open(true, window, cx);
            check!(current(this).is_empty() && this.drawer_row == 0, "no shown account: nothing marked, row 0 ({:?}, {})", current(this), this.drawer_row);
        }).unwrap();
        // 7. Accessible rows of the real Shell with two accounts.
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        this.update_in(cx, |this, window, cx| {
            this.set_menu_open(false, window, cx);
            let mut c = matrix_desktop::AccountCatalog::default();
            c.add("@anna:s.org", "https://s.org", "anna");
            let b = c.add("@bob:s.org", "https://s.org", "bob");
            this.accounts = c;
            this.active_id = Some(b);
            this.set_menu_open(true, window, cx);
            let rows = this.semantic_drawer().expect("drawer snapshot").rows;
            let l: Vec<_> = rows.iter().map(|r| (r.name.clone(), r.label())).collect();
            check!(l.len() == 2 && !l[0].1.contains("attivo") && l[1].1.ends_with(", account attivo"), "labels {l:?}");
        }).unwrap();
        cx.update(|_, cx| cx.quit()).unwrap();
        FAILED.lock().unwrap().extend(fails);
    })
    .detach();
}

#[test]
#[ignore = "requires private display"]
fn drawer_active_row_initial_focus_ring_and_account_change() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
