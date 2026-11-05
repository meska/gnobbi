//! Rendered-tree test of the non-modal «Informazioni stanza» panel: with it open, the list, the composer and the
//! timeline keep working; Esc closes it only from inside; the other drawers and dialogs stay modal.
//! Real pointer (xdotool) on the private X11 display:
//!   the ct script 'the gui-test-x11 script "" non_modal_room_details_panel'
//! The script's fixed display :77 may be held by a stale Xvfb on the CT (then the screen is tiny and clicks clamp): use another display number.
//! Room change closes the panel (declared by the implementation, sync_room); asserted exactly.
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, Bounds, EntityInputHandler, Keystroke, Pixels, WeakEntity};
use std::collections::HashMap;

thread_local! {
    static BUBBLES: std::cell::RefCell<HashMap<usize, Bounds<Pixels>>> = Default::default();
}

/// Paint hook of the timeline: bounds of each visible message text.
pub(crate) fn bubble_painted(index: usize, bounds: Bounds<Pixels>) {
    if sidebar_navigation_test::ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        BUBBLES.with_borrow_mut(|b| drop(b.insert(index, bounds)));
    }
}

fn painted(label: &str) -> Bounds<Pixels> {
    *sidebar_navigation_test::PAINTED.lock().unwrap().get(label).unwrap_or_else(|| panic!("{label} was never painted"))
}

fn xdotool(args: &[&str]) {
    assert!(std::process::Command::new("xdotool").args(args).status().unwrap().success(), "xdotool {args:?}");
}

fn composer_text(this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) -> String {
    this.composer.update(cx, |v, cx| {
        let mut actual = None;
        v.text_for_range(0..65536, &mut actual, window, cx).unwrap_or_default()
    })
}

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    let win = String::from_utf8(
        std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout,
    )
    .unwrap()
    .lines()
    .next()
    .expect("window")
    .to_string();
    macro_rules! click {
        ($p:expr) => {{
            let p: gpui::Point<Pixels> = $p;
            // Separate move and click: a click fired together with the move can reach the app at the old pointer position.
            xdotool(&["mousemove", "--window", &win, &(f32::from(p.x) as i32).to_string(), &(f32::from(p.y) as i32).to_string()]);
            wait(200).await;
            xdotool(&["click", "1"]);
            wait(400).await;
        }};
    }
    macro_rules! key {
        ($k:expr) => {{
            cx.update(|window, cx| drop(window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx))).unwrap();
            wait(100).await;
        }};
    }
    macro_rules! shell {
        ($body:expr) => { this.update_in(cx, $body).unwrap() };
    }
    let open_panel = |this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>| {
        this.set_room_details_open(true);
        this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx));
        cx.notify();
    };
    let area = |window: &Window| window.viewport_size();

    // Fixture: connected shell, wide window, room 0 selected, empty composer.
    wait(500).await;
    shell!(|this, window, cx| {
        assert!(this.connected && window.viewport_size().width >= px(800.), "wide connected fixture");
        this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..65536), "", window, cx));
    });

    // 1. A click on a list row changes room; the panel closes with it (sync_room, declared choice).
    shell!(|this, window, cx| open_panel(this, window, cx));
    wait(400).await;
    let (before, expected) = shell!(|this, _, _| {
        assert!(this.room_details_open);
        let visible = this.sidebar.visible_indices();
        (this.active_scope.clone(), this.sidebar.rooms()[visible[1]].scope.clone())
    });
    assert_ne!(before, expected);
    let list = painted("list");
    click!(gpui::point(list.left() + px(100.), list.top() + px(matrix_desktop::row_text::ROW_HEIGHT_PX * 1.5)));
    shell!(|this, _, _| {
        assert_eq!(this.active_scope, expected, "click on a list row with the panel open changes room");
        assert!(!this.room_details_open, "changing room closes the panel");
    });

    // 2. Keyboard in the composer types text with the panel open; the panel keeps neither focus nor the keys.
    shell!(|this, window, cx| {
        this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx));
        assert!(this.composer.read(cx).is_focused(window));
    });
    key!("ctrl-i");
    shell!(|this, window, cx| {
        assert!(this.room_details_open);
        assert!(this.composer.read(cx).is_focused(window), "opening the panel keeps composer focus");
    });
    let base = shell!(|this, window, cx| composer_text(this, window, cx));
    key!("h");
    key!("i");
    shell!(|this, window, cx| {
        assert!(this.room_details_open, "typing does not close the panel");
        assert!(this.composer.read(cx).is_focused(window) && !this.details_zone.contains_focused(window, cx), "composer keeps the focus");
        assert_eq!(composer_text(this, window, cx), format!("{base}hi"), "keys reach the composer");
    });

    // Ctrl+I closes without moving focus outside the panel, but restores list focus from inside it.
    key!("ctrl-i");
    shell!(|this, window, cx| assert!(!this.room_details_open && this.composer.read(cx).is_focused(window)));
    key!("ctrl-i");
    key!("f6");
    shell!(|this, window, cx| assert!(this.details_zone.contains_focused(window, cx)));
    key!("ctrl-i");
    shell!(|this, window, _| assert!(!this.room_details_open && this.list_focus.is_focused(window)));
    shell!(|this, window, cx| this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx)));
    key!("ctrl-i");

    // 3. Esc in the composer does not close the panel.
    key!("escape");
    shell!(|this, window, cx| {
        assert!(this.room_details_open, "Esc with focus in the composer keeps the panel open");
        assert!(this.composer.read(cx).is_focused(window));
    });

    // 4. A click on a bubble works with the panel open (focus moves to the timeline, the panel stays).
    shell!(|this, window, cx| {
        let scope = this.active_scope.clone();
        this.timeline = Some(TimelineView::new(scope, cx.focus_handle()));
        open_panel(this, window, cx);
    });
    BUBBLES.with_borrow_mut(HashMap::clear);
    wait(600).await;
    let width = shell!(|_, window, _| area(window).width);
    let panel = painted("details-panel");
    let bar = composer_view::painted_bar();
    let bubble = BUBBLES
        .with_borrow(|b| {
            let mut v: Vec<_> = b.values().copied().filter(|r| r.left() + px(10.) < panel.left() && r.size.height > px(8.) && r.top() > px(0.) && r.bottom() < bar.top()).collect();
            v.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
            v.pop()
        })
        .unwrap_or_else(|| panic!("no bubble clickable left of the panel {panel:?} (window {width:?})"));
    click!(gpui::point(bubble.left() + px(10.), bubble.top() + px(8.)));
    shell!(|this, window, cx| {
        assert!(this.room_details_open, "clicking a bubble does not dismiss the panel");
        assert!(this.timeline.as_ref().unwrap().is_focused(window), "the bubble click reached the timeline");
        assert!(!this.composer.read(cx).is_focused(window));
    });

    // 5. Esc with focus in the panel closes it (F6 moves the focus in, and back to the composer).
    key!("f6");
    shell!(|this, window, cx| assert!(this.details_zone.contains_focused(window, cx), "F6 focuses the panel"));
    key!("f6");
    shell!(|this, window, cx| assert!(this.composer.read(cx).is_focused(window) && !this.details_zone.contains_focused(window, cx), "F6 again returns to the composer"));
    key!("f6");
    key!("escape");
    shell!(|this, _, _| assert!(!this.room_details_open, "Esc with focus in the panel closes it"));

    // 6. «Chiudi» closes it.
    shell!(|this, window, cx| open_panel(this, window, cx));
    wait(400).await;
    click!(painted("details-close").center());
    shell!(|this, _, _| assert!(!this.room_details_open, "«Chiudi» closes the panel"));

    // 7. Regression: drawers and dialogs stay modal (a click on the scrim closes them and goes nowhere else).
    for what in ["menu", "settings", "groups", "room-edit"] {
        let scope = shell!(|this, window, cx| {
            this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..65536), "bozza", window, cx));
            match what {
                "menu" => this.set_menu_open(true, window, cx),
                "settings" => {
                    this.set_menu_open(true, window, cx);
                    this.set_settings_open(true, window, cx);
                }
                "groups" => this.open_groups_edit(window, cx),
                _ => this.open_room_edit(window, cx),
            }
            this.active_scope.clone()
        });
        wait(500).await;
        // Bottom-right corner: outside every dialog, and the composer's send button sits near it.
        let size = shell!(|_, window, _| area(window));
        click!(gpui::point(size.width - px(6.), size.height - px(6.)));
        shell!(|this, window, cx| {
            let open = this.menu_open || this.settings_open || this.groups_edit.is_some() || this.room_edit.is_some();
            assert!(!open, "{what}: a click outside closes it");
            assert_eq!(this.active_scope, scope, "{what}: room unchanged");
            assert_eq!(composer_text(this, window, cx), "bozza", "{what}: the click did not reach the composer");
        });
    }
    // «Modifica stanza»: text goes to its field, Esc cancels and focus returns to the list.
    shell!(|this, window, cx| this.open_room_edit(window, cx));
    key!("z");
    shell!(|this, window, cx| {
        assert!(this.room_edit.is_some() && !this.composer.read(cx).is_focused(window), "the dialog holds the focus");
        assert_eq!(composer_text(this, window, cx), "bozza", "typing does not reach the composer behind the dialog");
    });
    key!("escape");
    shell!(|this, window, _| assert!(this.room_edit.is_none() && this.list_focus.is_focused(window), "Esc closes «Modifica stanza» and focus returns to the list"));

    shell!(|_, _, cx| cx.quit());
}
