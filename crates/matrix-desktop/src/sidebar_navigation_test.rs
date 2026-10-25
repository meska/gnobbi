//! Actual Shell capture/bubble dispatch, confined to the private-display runner.
use crate::*;
use gpui::{EntityInputHandler, Keystroke};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static COMPACT: AtomicBool = AtomicBool::new(false);
static SPACES: AtomicBool = AtomicBool::new(false);
static CHAT: AtomicBool = AtomicBool::new(false);
static EMPTY: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicBool = AtomicBool::new(false);
static DYNAMIC: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Key-ups seen by the Shell; GPUI X11 turns an auto-repeat release/press split by scheduling jitter into a real key-up.
pub(crate) static UPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) static KEYDOWNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) static ACTIVATIONS: [std::sync::atomic::AtomicUsize; 3] = [
    std::sync::atomic::AtomicUsize::new(0),
    std::sync::atomic::AtomicUsize::new(0),
    std::sync::atomic::AtomicUsize::new(0), // rail / compact Spaces
];
const DRAFTS: [&str; 2] = [" caffè 🦀\nprima ", " δεύτερη 🚀\nbozza "];

type PointerCallback = std::rc::Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
thread_local! {
    static POINTERS: std::cell::RefCell<[Option<PointerCallback>; 2]> = Default::default();
}
pub(crate) fn capture_pointer(
    unread: bool,
    callback: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> PointerCallback {
    let callback: PointerCallback = std::rc::Rc::new(callback);
    if ACTIVE.load(Ordering::SeqCst) {
        POINTERS.with_borrow_mut(|callbacks| callbacks[unread as usize] = Some(callback.clone()));
    }
    callback
}
thread_local! {
    static RAILS: std::cell::RefCell<std::collections::HashMap<Option<RoomScope>, PointerCallback>> = Default::default();
}
pub(crate) fn capture_rail(
    scope: &Option<RoomScope>,
    callback: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> PointerCallback {
    let callback: PointerCallback = std::rc::Rc::new(callback);
    if ACTIVE.load(Ordering::SeqCst) {
        RAILS.with_borrow_mut(|rails| rails.insert(scope.clone(), callback.clone()));
    }
    callback
}
fn painted_rail(scope: &Option<RoomScope>) -> PointerCallback {
    RAILS.with_borrow(|rails| rails.get(scope).expect("rail entry was never painted").clone())
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
thread_local! {
    static SPACE_NODES: std::cell::RefCell<std::collections::HashMap<accesskit::NodeId, PointerCallback>> = Default::default();
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) fn capture_space(
    id: accesskit::NodeId,
    callback: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> PointerCallback {
    let callback: PointerCallback = std::rc::Rc::new(callback);
    if ACTIVE.load(Ordering::SeqCst) {
        SPACE_NODES.with_borrow_mut(|nodes| nodes.insert(id, callback.clone()));
    }
    callback
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn painted_space(id: accesskit::NodeId) -> PointerCallback {
    SPACE_NODES.with_borrow(|nodes| nodes.get(&id).expect("compact Spaces node was never painted").clone())
}
fn painted_pointer(unread: bool) -> PointerCallback {
    assert!(
        PAINTED
            .lock()
            .unwrap()
            .contains_key(if unread { "unread" } else { "theme" })
    );
    POINTERS.with_borrow(|callbacks| callbacks[unread as usize].as_ref().unwrap().clone())
}

static PAINTED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, gpui::Bounds<gpui::Pixels>>>,
> = std::sync::LazyLock::new(Default::default);

pub(crate) fn observe(label: String) -> Option<gpui::AnyElement> {
    ACTIVE.load(Ordering::SeqCst).then(|| {
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
                PAINTED.lock().unwrap().insert(label.clone(), bounds);
                eprintln!(
                    "painted {label}: {bounds:?}; viewport={:?}",
                    window.viewport_size()
                );
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    })
}


const RING: [&str; 10] = ["list", "search", "theme", "unread", "devices", "notifications", "info", "entry", "back", "none"];

fn ring_label(this: &Shell, window: &Window, cx: &App) -> &'static str {
    use matrix_desktop::panel_openers::Control;
    let entry = {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        { this.history_focus.is_focused(window) || this.composer.read(cx).is_focused(window) }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        { this.composer.read(cx).is_focused(window) }
    };
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    if this.back_ui.focus.is_focused(window) { return "back"; }
    [
        (this.list_focus.is_focused(window), "list"),
        (this.search_focus.is_focused(window), "search"),
        (this.theme_focus.is_focused(window), "theme"),
        (this.unread_focus.is_focused(window), "unread"),
        (this.opener_ui.focused(Control::Devices, window), "devices"),
        (this.opener_ui.focused(Control::Notifications, window), "notifications"),
        (this.opener_ui.focused(Control::Info, window), "info"),
        (entry, "entry"),
    ]
    .into_iter()
    .find_map(|(focused, name)| focused.then_some(name))
    .unwrap_or(RING[9])
}

fn ring_focus(this: &mut Shell, label: &str, window: &mut Window, cx: &mut Context<Shell>) {
    use matrix_desktop::panel_openers::{Control, Intent};
    let opener = |control| Intent { control, source: 0, focus_only: true };
    let control = match label {
        "list" => return this.list_focus.focus(window),
        "search" => return this.search_focus.focus(window),
        "theme" => return this.theme_focus.focus(window),
        "unread" => return this.unread_focus.focus(window),
        "devices" => Control::Devices,
        "notifications" => Control::Notifications,
        "info" => Control::Info,
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        "entry" => return this.history_focus.focus(window),
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        "entry" => return this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx)),
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        "back" => return this.back_ui.focus.focus(window),
        _ => unreachable!(),
    };
    let source = this.opener_ui.source(control).expect("opener must be visible to start");
    this.apply_opener(Intent { source, ..opener(control) }, window, cx);
}

fn ring_visible(label: &str, wide: bool, chat: bool) -> bool {
    match (wide, chat) {
        (true, _) => label != "back",
        (false, false) => ["list", "search", "theme", "unread", "devices", "notifications"].contains(&label),
        (false, true) => ["info", "entry", "back"].contains(&label),
    }
}

fn preserve_drafts_and_capture(
    this: &mut Shell,
    scopes: &[RoomScope],
    window: &mut Window,
    cx: &mut Context<Shell>,
    label: &str,
) {
    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
    assert_eq!(this.active_scope, scopes[0]);
    this.composer.update(cx, |v, cx| {
        for index in [0, 1, 0] {
            v.switch_room(scopes[index].clone(), cx);
            let mut actual = None;
            assert_eq!(
                v.text_for_range(0..65536, &mut actual, window, cx)
                    .as_deref(),
                Some(DRAFTS[index])
            );
        }
    });
    if let Ok(out) = std::env::var("P101_CAPTURE_DIR") {
        let ids = std::process::Command::new("xdotool")
            .args([
                "search",
                "--onlyvisible",
                "--pid",
                &std::process::id().to_string(),
            ])
            .output()
            .unwrap();
        assert!(ids.status.success());
        let ids = String::from_utf8(ids.stdout).unwrap();
        assert!(
            std::process::Command::new("magick")
                .args(["import", "-window", ids.lines().next().unwrap()])
                .arg(std::path::Path::new(&out).join(format!("p109-{label}.png")))
                .status()
                .unwrap()
                .success()
        );
    }
    eprintln!(
        "{label}: exact dual multiline Unicode drafts and selected/active scope preserved"
    );
}

fn pointer_transition(this: &mut Shell, transition: &str, away: bool) {
    match transition {
        "query" => this
            .sidebar
            .set_query(if away { "retire pointer" } else { "" }),
        "unread" => this.sidebar.set_unread_only(away),
        "details" => this.set_room_details_open(away),
        "notifications" => this.set_notifications_open(away),
        "devices" => this.set_verification_open(away),
        "account" => {
            this.active_scope.account_id = if away {
                AccountId::new(900)
            } else {
                this.sidebar.rooms()[0].scope.account_id
            };
        }
        "offline" => this.connected = !away,
        "compact-chat" => this.set_compact_chat(away),
        _ => unreachable!(),
    }
    this.refresh_navigation_visibility();
}

fn upsert_space(this: &mut Shell, account: matrix_domain::AccountId, id: &str, name: &str) {
    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
        scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id) },
        name: name.into(), unread: 0,
        info: matrix_engine::RoomInfo { is_space: true, ..Default::default() },
    }));
}

fn focus_choice(this: &mut Shell, window: &mut Window, cx: &mut Context<Shell>, scope: &Option<RoomScope>) {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    {
        let view = this.semantic_spaces(window).unwrap();
        let id = view.choices.iter().find(|c| c.scope == *scope).unwrap().id;
        this.apply_semantic_spaces(
            matrix_desktop::accessibility::spaces::SpacesAction::Focus { revision: view.revision, account: view.account, id },
            window,
            cx,
        );
    }
    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
    {
        this.spaces_ui.target = Some(scope.clone());
        let _ = cx;
    }
    this.spaces_ui.focus.focus(window);
}

async fn native_tap(key: &str, executor: &gpui::BackgroundExecutor) {
    native_key(key, true);
    native_key(key, false);
    executor.timer(std::time::Duration::from_millis(300)).await;
}

fn native_key(key: &str, down: bool) {
    let command = if down { "keydown" } else { "keyup" };
    assert!(
        std::process::Command::new("xdotool")
            .args([command, if key == "enter" { "Return" } else { key }])
            .status()
            .unwrap()
            .success()
    );
    eprintln!("isolated XTest {command} {key}");
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn sidebar_session(
    this: &Shell,
    window: &Window,
) -> matrix_desktop::accessibility::semantic_session::SemanticSession {
    use matrix_desktop::accessibility::{NavigationSnapshot, semantic_session::SemanticSession};
    SemanticSession::new(NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: this.sidebar.navigation_revision(),
        theme_focused: this.theme_focus.is_focused(window),
        theme_bounds: this.a11y_theme_bounds,
        unread_filter: this.semantic_unread(window),
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: this.dark,
        visible: true,
        query: this.sidebar.query().into(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    })
}

fn activation_callback(this: &Shell, window: &Window, unread: bool) -> PointerCallback {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    if DYNAMIC.load(Ordering::SeqCst) == 9 {
        let session = sidebar_session(this, window);
        let id = session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| {
                n.label()
                    == Some(if unread {
                        "Solo stanze non lette"
                    } else if this.dark {
                        "Attiva tema chiaro"
                    } else {
                        "Attiva tema scuro"
                    })
            })
            .unwrap()
            .0;
        let intents = [accesskit::Action::Focus, accesskit::Action::Click].map(|action| {
            session
                .action(&accesskit::ActionRequest {
                    action,
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: id,
                    data: None,
                })
                .unwrap()
        });
        let revision = this.sidebar.navigation_revision();
        return std::rc::Rc::new(move |this, window, cx| {
            let current = revision == this.sidebar.navigation_revision();
            for intent in &intents {
                if let matrix_desktop::accessibility::NavigationAction::Filter(action) = intent {
                    this.apply_semantic_filter(*action, window, cx);
                } else {
                    assert!(this.dispatch_sidebar_action(intent, window, cx));
                }
            }
            if current {
                assert!(
                    if unread {
                        this.unread_focus.is_focused(window)
                    } else {
                        this.theme_focus.is_focused(window)
                    },
                    "fresh translated Focus must reach its current native control"
                );
            }
            eprintln!(
                "actual pretranslated SemanticSession Focus/Click unread={unread}; revision={}; native focus theme={} unread={}",
                this.sidebar.navigation_revision(),
                this.theme_focus.is_focused(window),
                this.unread_focus.is_focused(window)
            );
        });
    }
    let _ = (this, window);
    painted_pointer(unread)
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        executor.timer(std::time::Duration::from_millis(400)).await;
        if COMPACT.load(Ordering::SeqCst) {
            navigation_resize_test::resize("360", "480");
            executor.timer(std::time::Duration::from_millis(200)).await;
        }
        let scopes = this.update_in(cx, |this, window, cx| {
            let scopes = this.sidebar.rooms()[..2].iter().map(|r| r.scope.clone()).collect::<Vec<_>>();
            for (scope, draft) in scopes.iter().zip(DRAFTS) {
                assert!(this.sidebar.select(scope));
                this.sync_room(cx);
                this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..65536), draft, window, cx));
            }
            assert!(this.sidebar.select(&scopes[0]));
            this.sync_room(cx);
            if SPACES.load(Ordering::SeqCst) {
                for (id, name) in [("!alpha:fixture", "Alpha"), ("!beta:fixture", "Beta")] {
                    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                        scope: RoomScope { account_id: scopes[0].account_id, room_id: matrix_domain::RoomId::new(id) },
                        name: name.into(), unread: 0,
                        info: matrix_engine::RoomInfo { is_space: true, ..Default::default() },
                    }));
                }
                assert_eq!(this.sidebar.spaces().count(), 2);
            }
            this.connected = CONNECTED.load(Ordering::SeqCst); // DTO fixture only; no network worker.
            if DYNAMIC.load(Ordering::SeqCst) > 0 && std::env::var("P109_DARK").as_deref() == Ok("1") {
                this.toggle_theme(cx);
            }
            this.set_spaces_open(OPEN.load(Ordering::SeqCst));
            this.rail_ui.target = this.sidebar.navigation_entries().last().unwrap().scope.clone();
            if COMPACT.load(Ordering::SeqCst) {
                assert_eq!(window.viewport_size().width, px(360.));
                this.set_compact_chat(false);
                this.theme_focus.focus(window);
            } else {
                this.rail_ui.focus.focus(window);
            }
            cx.notify();
            scopes
        }).unwrap();
        let history = this.update_in(cx, |this, _, _| {
            let active = this.timeline.as_ref().unwrap().sidebar_history_probe();
            let saved = this.saved_timelines[&scopes[1]].sidebar_history_probe();
            let other_scope = scopes[1].clone();
            move |this: &Shell| active(this.timeline.as_ref().unwrap()) && saved(&this.saved_timelines[&other_scope])
        }).unwrap();
        if matches!(DYNAMIC.load(Ordering::SeqCst), 8 | 9) {
            for unread in [false, true] {
                for transition in ["query", "unread", "details", "notifications", "devices", "account", "offline", "compact-chat"] {
                    if transition == "compact-chat" && !COMPACT.load(Ordering::SeqCst) { continue; }
                    executor.timer(std::time::Duration::from_millis(150)).await;
                    let callback = this.update_in(cx, |this, window, cx| {
                        let callback = activation_callback(this, window, unread);
                        let before = ACTIVATIONS[unread as usize].load(Ordering::SeqCst);
                        this.list_focus.focus(window);
                        pointer_transition(this, transition, true);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), before, "retired painted callback activated during {transition}, unread={unread}");
                        assert!(this.list_focus.is_focused(window), "rejected pointer changed focus");
                        pointer_transition(this, transition, false);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), before, "retired painted callback activated after {transition} away/back, unread={unread}");
                        assert!(this.list_focus.is_focused(window));
                        assert!(history(this));
                        eprintln!("painted pointer retired during/after {transition}, unread={unread}, revision={}", this.sidebar.navigation_revision());
                        cx.notify();
                        callback
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(150)).await;
                    this.update_in(cx, |this, window, cx| {
                        let before = ACTIVATIONS[unread as usize].load(Ordering::SeqCst);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), before, "fresh paint resurrected old pointer");
                        activation_callback(this, window, unread)(this, window, cx);
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), before + 1, "reacquired painted callback must activate exactly once");
                        if !unread { this.toggle_theme(cx); }
                        this.sidebar.set_unread_only(false);
                        assert!(history(this));
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("pointer-{unread}-{transition}"));
                        cx.notify();
                    }).unwrap();
                }
            }
            executor.timer(std::time::Duration::from_millis(150)).await;
            let callbacks = this.update_in(cx, |this, window, _| {
                let callbacks = [activation_callback(this, window, false), activation_callback(this, window, true)];
                PAINTED.lock().unwrap().clear();
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                if COMPACT.load(Ordering::SeqCst) { navigation_resize_test::resize("1120", "760"); }
                else { navigation_resize_test::resize("360", "480"); }
                callbacks
            }).unwrap();
            for returned in [false, true] {
                executor.timer(std::time::Duration::from_millis(200)).await;
                this.update_in(cx, |this, window, cx| {
                    let compact = COMPACT.load(Ordering::SeqCst) == returned;
                    assert_eq!(window.viewport_size().width, if compact { px(360.) } else { px(1120.) });
                    assert!(PAINTED.lock().unwrap().is_empty(), "Configure must not repaint while suspended");
                    this.list_focus.focus(window);
                    for (index, callback) in callbacks.iter().enumerate() {
                        let before = ACTIVATIONS[index].load(Ordering::SeqCst);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[index].load(Ordering::SeqCst), before, "old painted source activated on native Configure");
                        assert!(this.list_focus.is_focused(window));
                    }
                    assert!(history(this));
                    eprintln!("actual native Configure returned={returned}, viewport={:?}, no intermediate paint, both callbacks rejected", window.viewport_size());
                    if !returned {
                        if COMPACT.load(Ordering::SeqCst) { navigation_resize_test::resize("360", "480"); }
                        else { navigation_resize_test::resize("1120", "760"); }
                    } else {
                        navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                        cx.notify();
                    }
                }).unwrap();
            }
            executor.timer(std::time::Duration::from_millis(200)).await;
            this.update_in(cx, |this, window, cx| {
                for (index, callback) in callbacks.iter().enumerate() {
                    let before = ACTIVATIONS[index].load(Ordering::SeqCst);
                    callback(this, window, cx);
                    assert_eq!(ACTIVATIONS[index].load(Ordering::SeqCst), before);
                    activation_callback(this, window, index == 1)(this, window, cx);
                    assert_eq!(ACTIVATIONS[index].load(Ordering::SeqCst), before + 1);
                }
                assert!(history(this));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(200)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(history(this));
                assert!(this.unread_focus.is_focused(window));
                let bounds = PAINTED.lock().unwrap()["unread"];
                assert!(bounds.size.width > px(0.) && bounds.bottom() <= window.viewport_size().height);
                preserve_drafts_and_capture(this, &scopes, window, cx, "pointer-native-return");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 16 {
            // List/chat seam: room list -> footer Devices, Notifications -> Info -> chat entry, and back.
            let chat = CHAT.load(Ordering::SeqCst);
            let steps: Vec<(&str, &str)> = if chat {
                vec![("tab", "info"), ("tab", "entry"), ("shift-tab", "info"), ("shift-tab", "back")]
            } else if COMPACT.load(Ordering::SeqCst) {
                vec![("tab", "devices"), ("tab", "notifications"), ("tab", "theme"), ("shift-tab", "notifications"), ("shift-tab", "devices"), ("shift-tab", "list")]
            } else {
                vec![("tab", "devices"), ("tab", "notifications"), ("tab", "info"), ("tab", "entry"), ("shift-tab", "info"), ("shift-tab", "notifications"), ("shift-tab", "devices"), ("shift-tab", "list")]
            };
            this.update_in(cx, |this, window, cx| {
                if chat {
                    this.set_compact_chat(true);
                    this.back_focus_handle().focus(window);
                } else {
                    this.list_focus.focus(window);
                }
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(300)).await;
            for (key, expected) in steps {
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
                this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::Control;
                    let focused = match expected {
                        "list" => this.list_focus.is_focused(window),
                        "theme" => this.theme_focus.is_focused(window),
                        "back" => this.back_focus_handle().is_focused(window),
                        "devices" => this.opener_ui.focused(Control::Devices, window),
                        "notifications" => this.opener_ui.focused(Control::Notifications, window),
                        "info" => this.opener_ui.focused(Control::Info, window),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        "entry" => this.history_focus.is_focused(window),
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        "entry" => this.composer.read(cx).is_focused(window),
                        _ => unreachable!(),
                    };
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "focus must not open a panel");
                    assert_eq!(this.compact_chat, chat);
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.dark && !this.sidebar.unread_only());
                    assert!(history(this));
                    this.composer.update(cx, |v, cx| {
                        let mut actual = None;
                        assert_eq!(v.text_for_range(0..65536, &mut actual, window, cx).as_deref(), Some(DRAFTS[0]));
                    });
                    eprintln!("seam: {key} => {expected}; selected/history/draft unchanged");
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(80)).await;
            }
            this.update_in(cx, |this, window, cx| {
                preserve_drafts_and_capture(this, &scopes, window, cx, if chat { "seam-chat" } else if COMPACT.load(Ordering::SeqCst) { "seam-compact" } else { "seam-wide" });
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 19 {
            // Every ring control, focused before a real native Configure with paint suspended, lands on a visible one.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let cases: &[(bool, bool, &str)] = &[
                (true, false, "devices"), (true, false, "notifications"), (true, false, "info"), (true, false, "entry"),
                (true, false, "theme"), (true, false, "unread"), (true, false, "search"), (true, false, "list"),
                (true, true, "devices"), (true, true, "notifications"), (true, true, "info"), (true, true, "entry"),
                (false, true, "info"), (false, true, "entry"),
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                (false, true, "back"),
                (false, false, "devices"), (false, false, "notifications"), (false, false, "theme"),
                (false, false, "unread"), (false, false, "search"), (false, false, "list"),
            ];
            let size = |wide: bool| if wide { ("1120", "760") } else { ("360", "480") };
            for &(wide, chat, start) in cases {
                // Settle the starting layout with real Configure events and live paint.
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                navigation_resize_test::resize(size(wide).0, size(wide).1);
                wait(300).await;
                let callbacks = this.update_in(cx, |this, window, cx| {
                    assert_eq!(window.viewport_size().width >= px(800.), wide);
                    this.list_focus.focus(window); // no leftover chat-entry focus may pick the page
                    this.navigation_wide = wide;
                    this.set_compact_chat(chat);
                    cx.notify();
                    // Theme and unread are only painted where the sidebar is visible.
                    (wide || !chat).then(|| [false, true].map(|unread| activation_callback(this, window, unread)))
                }).unwrap();
                wait(200).await;
                let retired = this.update_in(cx, |this, window, cx| {
                    ring_focus(this, start, window, cx);
                    assert_eq!(ring_label(this, window, cx), start, "setup must focus {start}");
                    // Normal (non-focus) opener intents for every opener, captured at the start layout.
                    use matrix_desktop::panel_openers::{Control, Intent};
                    let intents = Control::ALL.map(|control| this.opener_ui.source(control).map(|source| Intent { control, source, focus_only: false }));
                    PAINTED.lock().unwrap().clear();
                    navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                    navigation_resize_test::resize(size(!wide).0, size(!wide).1);
                    intents
                }).unwrap();
                wait(300).await;
                this.update_in(cx, |this, window, cx| {
                    assert_eq!(window.viewport_size().width >= px(800.), !wide);
                    assert!(PAINTED.lock().unwrap().is_empty(), "Configure must not repaint while suspended");
                    let landed = ring_label(this, window, cx);
                    // A narrow target shows the page the shell now reports; a focused chat entry brings the chat page.
                    let chat = if wide { this.compact_chat } else { chat };
                    eprintln!("ring wide={wide} chat={chat}: {start} -> {landed} (target visible={})", ring_visible(landed, !wide, chat));
                    assert!(ring_visible(landed, !wide, chat), "{start} must land on a visible control after Configure, got {landed}");
                    if ring_visible(start, !wide, chat) {
                        assert_eq!(landed, start, "{start} stays visible and must keep focus");
                    }
                    // Retired callbacks: nothing opens, nothing is activated, nothing is sent.
                    let before = [0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst));
                    let draft_before = this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) });
                    for callback in callbacks.iter().flatten() { callback(this, window, cx); }
                    for intent in retired.into_iter().flatten() {
                        let hidden = this.opener_ui.source(intent.control) != Some(intent.source);
                        if hidden { this.apply_opener(intent, window, cx); }
                    }
                    assert_eq!([0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst)), before, "retired Theme/unread callback acted");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "retired opener opened a panel");
                    assert_eq!(this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) }), draft_before);
                    assert_eq!(this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) }).as_deref(), Some(DRAFTS[0]), "draft must be intact, nothing sent");
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(history(this));
                    navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                    cx.notify();
                }).unwrap();
                wait(300).await;
                this.update_in(cx, |this, window, cx| {
                    let landed = ring_label(this, window, cx);
                    let chat = if wide { this.compact_chat } else { chat };
                    assert!(ring_visible(landed, !wide, chat), "focus must stay visible after repaint, got {landed}");
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(history(this));
                }).unwrap();
            }
            this.update_in(cx, |this, window, cx| {
                preserve_drafts_and_capture(this, &scopes, window, cx, "ring-resize");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 20 {
            // A light/dark change with focus on each ring control keeps the focus target, room, history and drafts.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let mut cases: Vec<(bool, bool, &str)> = vec![];
            for label in ["list", "search", "theme", "unread", "devices", "notifications", "info", "entry"] {
                cases.push((true, false, label));
            }
            for label in ["list", "search", "theme", "unread", "devices", "notifications"] {
                cases.push((false, false, label));
            }
            for label in ["info", "entry", #[cfg(all(feature = "linux-a11y", target_os = "linux"))] "back"] {
                cases.push((false, true, label));
            }
            for (wide, chat, start) in cases {
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                let (w, h) = if wide { ("1120", "760") } else { ("360", "480") };
                navigation_resize_test::resize(w, h);
                wait(300).await;
                // Capture the sidebar callbacks and opener intents on the list page, then show the target page.
                this.update_in(cx, |this, window, cx| {
                    assert_eq!(window.viewport_size().width >= px(800.), wide);
                    this.navigation_wide = wide;
                    this.set_compact_chat(false);
                    this.list_focus.focus(window);
                    cx.notify();
                }).unwrap();
                wait(200).await;
                let (callbacks, intents) = this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::{Control, Intent};
                    let callbacks = [false, true].map(|unread| activation_callback(this, window, unread));
                    let intents = Control::ALL.map(|control| this.opener_ui.source(control).map(|source| Intent { control, source, focus_only: false }));
                    this.set_compact_chat(chat);
                    cx.notify();
                    (callbacks, intents)
                }).unwrap();
                wait(200).await;
                let dark_before = this.update_in(cx, |this, window, cx| {
                    ring_focus(this, start, window, cx);
                    assert_eq!(ring_label(this, window, cx), start, "setup must focus {start}");
                    let dark = this.dark;
                    let toggles = ACTIVATIONS[0].load(Ordering::SeqCst);
                    this.toggle_theme(cx);
                    assert_eq!(this.dark, !dark, "theme must flip");
                    assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), toggles + 1);
                    assert_eq!(ring_label(this, window, cx), start, "theme change moved focus off {start}");
                    cx.notify();
                    dark
                }).unwrap();
                wait(300).await;
                this.update_in(cx, |this, window, cx| {
                    let landed = ring_label(this, window, cx);
                    eprintln!("theme wide={wide} chat={chat}: {start} -> {landed} after repaint, dark={}", this.dark);
                    assert_eq!(landed, start, "focus must survive the repaint of the new theme");
                    assert_eq!(this.dark, !dark_before);
                    assert_eq!(this.compact_chat, chat);
                    // Hidden sidebar callbacks and openers stay inert after the theme change.
                    let before = [0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst));
                    let draft_before = this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) });
                    if !wide && chat {
                        for callback in &callbacks { callback(this, window, cx); }
                    }
                    for intent in intents.into_iter().flatten() {
                        if this.opener_ui.source(intent.control) != Some(intent.source) { this.apply_opener(intent, window, cx); }
                    }
                    assert_eq!([0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst)), before, "hidden callback acted after theme change");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "hidden opener opened a panel");
                    assert_eq!(this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) }), draft_before);
                    assert_eq!(draft_before.as_deref(), Some(DRAFTS[0]), "draft must be intact, nothing sent");
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.sidebar.unread_only());
                    assert!(history(this));
                    assert_eq!(ring_label(this, window, cx), start);
                    this.toggle_theme(cx); // back to the starting theme for the next case
                    assert_eq!(this.dark, dark_before);
                    cx.notify();
                }).unwrap();
                wait(200).await;
                this.update_in(cx, |this, window, cx| {
                    assert_eq!(ring_label(this, window, cx), start, "focus must survive switching back");
                }).unwrap();
            }
            this.update_in(cx, |this, window, cx| {
                preserve_drafts_and_capture(this, &scopes, window, cx, "ring-theme");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 18 {
            // Filter and room switch, then the empty and selected-hidden list modes keep a deterministic ring.
            let empty = EMPTY.load(Ordering::SeqCst);
            // Painted Theme and unread-filter callbacks captured while still visible; the filter change retires them.
            executor.timer(std::time::Duration::from_millis(300)).await;
            let painted = this.update_in(cx, |this, window, _| [false, true].map(|unread| activation_callback(this, window, unread))).unwrap();
            this.update_in(cx, |this, window, cx| {
                let name = this.sidebar.rooms()[1].name.clone();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                    scope: scopes[1].clone(), name, unread: 3, info: Default::default(),
                }));
                // Room switch and back through the sidebar, then the filter.
                assert!(this.sidebar.select(&scopes[1]));
                this.sync_room(cx);
                assert!(this.sidebar.select(&scopes[0]));
                this.sync_room(cx);
                if empty {
                    this.sidebar.set_query("no matches ");
                } else {
                    this.sidebar.set_unread_only(true);
                }
                assert_eq!(this.sidebar.visible_indices().len(), if empty { 0 } else { 1 });
                this.list_focus.focus(window);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(300)).await;
            // The room switch above replaced the live timelines: probe the post-switch ones.
            let history = this.update_in(cx, |this, _, _| {
                let active = this.timeline.as_ref().unwrap().sidebar_history_probe();
                let saved = this.saved_timelines[&scopes[1]].sidebar_history_probe();
                let other_scope = scopes[1].clone();
                move |this: &Shell| active(this.timeline.as_ref().unwrap()) && saved(&this.saved_timelines[&other_scope])
            }).unwrap();
            this.update_in(cx, |this, window, cx| {
                let before = [0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst));
                let unread = this.sidebar.unread_only();
                for callback in &painted {
                    callback(this, window, cx);
                }
                assert!(this.list_focus.is_focused(window), "retired callbacks must not steal focus");
                assert_eq!([0, 1].map(|i| ACTIVATIONS[i].load(Ordering::SeqCst)), before, "retired callbacks must not act");
                assert!(!this.dark && this.sidebar.unread_only() == unread);
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                eprintln!("hidden-ring empty={empty}: retired Theme/unread callbacks inert");
            }).unwrap();
            let steps = [
                ("tab", "devices"), ("tab", "notifications"), ("tab", "info"), ("tab", "entry"),
                ("shift-tab", "info"), ("shift-tab", "notifications"), ("shift-tab", "devices"), ("shift-tab", "list"),
                ("shift-tab", "search"), ("tab", "list"),
            ];
            for (key, expected) in steps {
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
                this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::Control;
                    let focused = match expected {
                        "list" => this.list_focus.is_focused(window),
                        "search" => this.search_focus.is_focused(window),
                        "devices" => this.opener_ui.focused(Control::Devices, window),
                        "notifications" => this.opener_ui.focused(Control::Notifications, window),
                        "info" => this.opener_ui.focused(Control::Info, window),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        "entry" => this.history_focus.is_focused(window),
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        "entry" => this.composer.read(cx).is_focused(window),
                        _ => unreachable!(),
                    };
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "focus must not open a panel");
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0], "focus must not change the selected room");
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(history(this));
                    this.composer.update(cx, |v, cx| {
                        let mut actual = None;
                        assert_eq!(v.text_for_range(0..65536, &mut actual, window, cx).as_deref(), Some(DRAFTS[0]));
                    });
                    eprintln!("hidden-ring empty={empty}: {key} => {expected}; selected/history/draft unchanged");
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(80)).await;
            }
            this.update_in(cx, |this, window, cx| {
                preserve_drafts_and_capture(this, &scopes, window, cx, if empty { "list-empty" } else { "list-selected-hidden" });
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 17 {
            // Chat end of the ring: history boundary, composer, eligible Send and the wrap to the first navigation control.
            let chat = CHAT.load(Ordering::SeqCst);
            let empty = EMPTY.load(Ordering::SeqCst);
            let last = if empty { "composer" } else { "send" };
            let a11y = cfg!(all(feature = "linux-a11y", target_os = "linux"));
            let first = if chat { "back" } else { "rail0" };
            let mut steps: Vec<(&str, &str)> = vec![("shift-tab", last)];
            if !empty {
                steps.push(("shift-tab", "composer"));
            }
            if a11y {
                steps.push(("shift-tab", "history"));
            }
            steps.push(("shift-tab", "info"));
            steps.push(("tab", if a11y { "history" } else { "composer" }));
            if a11y {
                steps.push(("tab", "composer"));
            }
            if !empty {
                steps.push(("tab", "send"));
            }
            steps.push(("tab", first));
            steps.push(("tab", if chat { "info" } else { "rail1" }));
            this.update_in(cx, |this, window, cx| {
                if chat {
                    this.set_compact_chat(true);
                    this.back_focus_handle().focus(window);
                } else {
                    this.rail_ui.target = this.sidebar.navigation_entries()[0].scope.clone();
                    this.rail_ui.focus.focus(window);
                }
                if empty {
                    this.composer.update(cx, |v, cx| {
                        let mut actual = None;
                        let len = v.text_for_range(0..65536, &mut actual, window, cx).unwrap().len();
                        v.replace_text_in_range(Some(0..len), "  ", window, cx);
                    });
                }
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(300)).await;
            // The first key is the one under test from the first control.
            if !chat { steps.insert(0, ("noop", "rail0")); }
            for (key, expected) in steps {
                if key != "noop" {
                    cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
                }
                this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::Control;
                    let focused = match expected {
                        "back" => this.back_focus_handle().is_focused(window),
                        "info" => this.opener_ui.focused(Control::Info, window),
                        "send" => this.composer.read(cx).send_is_focused(window),
                        "composer" => this.composer.read(cx).is_focused(window),
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        "history" => this.history_focus.is_focused(window),
                        name if name.starts_with("rail") => {
                            let index = name[4..].parse::<usize>().unwrap();
                            this.rail_ui.focus.is_focused(window) && this.rail_ui.target == this.sidebar.navigation_entries()[index].scope
                        }
                        _ => unreachable!(),
                    };
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "focus must not open a panel");
                    assert_eq!(this.compact_chat, chat);
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.dark && !this.sidebar.unread_only());
                    assert!(history(this));
                    if !empty {
                        this.composer.update(cx, |v, cx| {
                            let mut actual = None;
                            assert_eq!(v.text_for_range(0..65536, &mut actual, window, cx).as_deref(), Some(DRAFTS[0]));
                        });
                    }
                    eprintln!("ring: {key} => {expected}; selected/history/draft unchanged");
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(80)).await;
            }
            this.update_in(cx, |this, window, cx| {
                if empty {
                    this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..2), DRAFTS[0], window, cx));
                }
                preserve_drafts_and_capture(this, &scopes, window, cx, if chat { "ring-chat" } else if empty { "ring-empty" } else { "ring-wide" });
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 7 {
            executor.timer(std::time::Duration::from_millis(250)).await;
            for unread in [false, true] {
                this.update_in(cx, |this, window, _| {
                    if unread { this.unread_focus.focus(window); } else { this.theme_focus.focus(window); }
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(120)).await;
                for key in ["space", "enter"] {
                    let before = ACTIVATIONS[unread as usize].load(Ordering::SeqCst);
                    // A release without a matching press must not activate a painted control.
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, _, _| {
                        assert!(!this.dark, "unmatched release activated Theme");
                        assert!(!this.sidebar.unread_only(), "unmatched release activated Unread");
                    }).unwrap();
                    let downs_before = KEYDOWNS.load(Ordering::SeqCst);
                    native_key(key, true);
                    for wait in [100, 900, 150] {
                        executor.timer(std::time::Duration::from_millis(wait)).await;
                        this.update_in(cx, |this, _, _| {
                            assert!(!this.dark, "press/repeat activated Theme before release");
                            assert!(!this.sidebar.unread_only(), "press/repeat activated Unread before release");
                            assert!(history(this));
                        }).unwrap();
                    }
                    assert!(KEYDOWNS.load(Ordering::SeqCst) >= downs_before + 2, "private X server must deliver actual held-repeat keydowns");
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| {
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), before + 1, "exactly one actual callback activation per gesture");
                        eprintln!("current release {unread}/{key}: exact callback count {} -> {}", before, before+1);
                        assert_eq!(this.dark, !unread, "completed gesture must activate Theme once");
                        assert_eq!(this.sidebar.unread_only(), unread, "completed gesture must activate Unread once");
                        if this.dark { this.toggle_theme(cx); }
                        this.sidebar.set_unread_only(false);
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("release-{unread}-{key}"));
                        cx.notify();
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(120)).await;
                    let stale_before = ACTIVATIONS[unread as usize].load(Ordering::SeqCst);
                    native_key(key, true);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    if std::env::var("P110_RESIZE").as_deref() == Ok("1") {
                        this.update_in(cx, |_, _, _| {
                            PAINTED.lock().unwrap().clear();
                            navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                            if COMPACT.load(Ordering::SeqCst) { navigation_resize_test::resize("1120", "760"); } else { navigation_resize_test::resize("360", "480"); }
                        }).unwrap();
                        executor.timer(std::time::Duration::from_millis(200)).await;
                        this.update_in(cx, |_, window, _| {
                            assert_eq!(window.viewport_size().width, if COMPACT.load(Ordering::SeqCst) { px(1120.) } else { px(360.) });
                            if COMPACT.load(Ordering::SeqCst) { navigation_resize_test::resize("360", "480"); } else { navigation_resize_test::resize("1120", "760"); }
                        }).unwrap();
                        executor.timer(std::time::Duration::from_millis(200)).await;
                        this.update_in(cx, |this, window, cx| {
                            assert_eq!(window.viewport_size().width, if COMPACT.load(Ordering::SeqCst) { px(360.) } else { px(1120.) });
                            assert!(!PAINTED.lock().unwrap().contains_key("theme"), "no intermediate sidebar paint allowed");
                            assert!(this.pending_sidebar_key.is_some(), "retain retired press until release");
                            eprintln!("real Configure away/back with paint suspended: {:?}", window.viewport_size());
                            navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                            if unread { this.unread_focus.focus(window); } else { this.theme_focus.focus(window); }
                            cx.notify();
                        }).unwrap();
                        executor.timer(std::time::Duration::from_millis(200)).await;
                        assert!(PAINTED.lock().unwrap().contains_key("theme"), "reacquire actual fresh callback paint before releasing held key");
                    } else {
                        this.update_in(cx, |this, _, cx| {
                            let transition = std::env::var("P112_TRANSITION").unwrap_or_else(|_| "query".into());
                            pointer_transition(this, &transition, true);
                            if std::env::var("P112_RELEASE_HIDDEN").as_deref() != Ok("1") {
                                pointer_transition(this, &transition, false);
                            }
                            cx.notify();
                        }).unwrap();
                    }
                    executor.timer(std::time::Duration::from_millis(900)).await;
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, _, cx| {
                        navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                        if std::env::var("P112_RELEASE_HIDDEN").as_deref() == Ok("1") {
                            let transition = std::env::var("P112_TRANSITION").unwrap();
                            pointer_transition(this, &transition, false);
                        }
                        cx.notify();
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), stale_before, "release after source retirement must not activate a replacement");
                        assert!(!this.dark);
                        assert!(!this.sidebar.unread_only());
                        assert!(history(this));
                    }).unwrap();
                    native_key(key, true);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| { this.search_focus.focus(window); cx.notify(); }).unwrap();
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| {
                        if unread { this.unread_focus.focus(window); } else { this.theme_focus.focus(window); }
                        cx.notify();
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(900)).await;
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| {
                        assert_eq!(ACTIVATIONS[unread as usize].load(Ordering::SeqCst), stale_before, "release after focus away-back must not activate");
                        assert!(!this.dark);
                        assert!(!this.sidebar.unread_only());
                        assert!(history(this));
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("focus-retired-{unread}-{key}"));
                    }).unwrap();
                }
            }
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if DYNAMIC.load(Ordering::SeqCst) == 15 {
            // Translated semantic Spaces intents (rail or compact menu): exact activation counts, stale/replaced sources rejected, Focus changes nothing.
            use matrix_desktop::accessibility::spaces::SpacesAction;
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let compact = COMPACT.load(Ordering::SeqCst);
            let account = scopes[0].account_id;
            let beta = this.update_in(cx, |this, _, _| this.sidebar.navigation_entries()[2].scope.clone()).unwrap();
            fn translate(this: &mut Shell, window: &mut Window, action: accesskit::Action, toggle: bool, scope: &Option<RoomScope>) -> SpacesAction {
                let view = this.semantic_spaces(window).unwrap();
                let id = if toggle { matrix_desktop::accessibility::spaces::TOGGLE } else { view.choices.iter().find(|c| c.scope == *scope).unwrap().id };
                view.action(&accesskit::ActionRequest { action, target_tree: accesskit::TreeId::ROOT, target_node: id, data: None }).unwrap()
            }
            fn reset(this: &mut Shell, open: bool, cx: &mut Context<Shell>) {
                this.sidebar.set_space(None);
                this.set_spaces_open(open);
                cx.notify();
            }
            wait(300).await;
            // 1. Focus alone: no filter/menu change, same selection, drafts and history identities.
            this.update_in(cx, |this, window, cx| {
                reset(this, !compact, cx);
                let revision = this.sidebar.navigation_revision();
                let open = this.spaces_open;
                let focus = translate(this, window, accesskit::Action::Focus, compact, &beta);
                this.apply_semantic_spaces(focus.clone(), window, cx);
                this.apply_semantic_spaces(focus, window, cx);
                assert!(this.sidebar.active_space().is_none(), "Focus activated a Space");
                assert_eq!(this.spaces_open, open, "Focus changed the menu");
                assert_eq!(this.sidebar.navigation_revision(), revision, "Focus changed navigation state");
                assert!(history(this), "Focus must preserve timeline identities");
                preserve_drafts_and_capture(this, &scopes, window, cx, "p111-spaces-focus");
            }).unwrap();
            // 2. Compact toggle: the same translated Click applied twice opens exactly once.
            if compact {
                this.update_in(cx, |this, window, cx| {
                    reset(this, false, cx);
                    let click = translate(this, window, accesskit::Action::Click, true, &None);
                    this.apply_semantic_spaces(click.clone(), window, cx);
                    assert!(this.spaces_open, "translated toggle must open once");
                    let revision = this.sidebar.navigation_revision();
                    this.apply_semantic_spaces(click, window, cx);
                    assert!(this.spaces_open, "second delivery of the same intent toggled again");
                    assert_eq!(this.sidebar.navigation_revision(), revision);
                    reset(this, true, cx);
                }).unwrap();
            }
            // 3. Choice: applied twice, activates once and keeps selection/history; a repeat after it is rejected.
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                reset(this, true, cx);
                let click = translate(this, window, accesskit::Action::Click, false, &beta);
                this.apply_semantic_spaces(click.clone(), window, cx);
                assert_eq!(this.sidebar.active_space(), beta.as_ref(), "translated choice must activate its Space once");
                let revision = this.sidebar.navigation_revision();
                this.apply_semantic_spaces(click, window, cx);
                assert_eq!(this.sidebar.navigation_revision(), revision, "second delivery of the same choice acted again");
                assert!(!compact || !this.spaces_open, "compact menu must close after the choice");
                assert!(history(this), "activation must keep timeline identities");
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                reset(this, true, cx);
            }).unwrap();
            // 4. Stale: revision bumped after translation, and target replaced at the same index.
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                let click = translate(this, window, accesskit::Action::Click, false, &beta);
                this.sidebar.set_unread_only(true);
                this.sidebar.set_unread_only(false);
                this.apply_semantic_spaces(click, window, cx);
                assert!(this.sidebar.active_space().is_none(), "choice translated before a revision change activated");
                let stale = translate(this, window, accesskit::Action::Click, false, &beta);
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(beta.clone().unwrap()));
                upsert_space(this, account, "!gamma:fixture", "Gamma");
                this.apply_semantic_spaces(stale, window, cx);
                assert!(this.sidebar.active_space().is_none(), "choice of a replaced Space activated");
                cx.notify();
            }).unwrap();
            // 5. Fresh intent after reacquisition works exactly once; drafts and history still exact.
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                let gamma = this.sidebar.navigation_entries().into_iter().find(|e| e.name == "Gamma").unwrap().scope;
                let click = translate(this, window, accesskit::Action::Click, false, &gamma);
                this.apply_semantic_spaces(click, window, cx);
                assert_eq!(this.sidebar.active_space(), gamma.as_ref(), "fresh translated choice must activate");
                assert!(history(this));
                reset(this, true, cx);
                preserve_drafts_and_capture(this, &scopes, window, cx, "p111-spaces-intents");
                cx.quit();
            }).unwrap();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if DYNAMIC.load(Ordering::SeqCst) == 14 {
            // Painted compact Spaces toggle/choice callbacks: current ones act once; retired, hidden or replaced ones never; Configure before repaint included.
            use matrix_desktop::accessibility::spaces::TOGGLE;
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let account = scopes[0].account_id;
            let beta = this.update_in(cx, |this, _, _| this.sidebar.navigation_entries()[2].scope.clone()).unwrap();
            fn paint(this: &mut Shell, open: bool, cx: &mut Context<Shell>) {
                this.sidebar.set_space(None);
                this.set_spaces_open(open);
                cx.notify();
            }
            let choice_id = |this: &mut Shell, window: &mut Window, scope: &Option<RoomScope>| {
                this.semantic_spaces(window).unwrap().choices.iter().find(|c| c.scope == *scope).unwrap().id
            };
            // 1. Current toggle opens once; current choice picks its Space once and closes the menu.
            this.update_in(cx, |this, _, cx| paint(this, false, cx)).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                painted_space(TOGGLE)(this, window, cx);
                assert!(this.spaces_open, "current painted toggle must open the menu once");
                cx.notify();
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                let id = choice_id(this, window, &beta);
                painted_space(id)(this, window, cx);
                assert_eq!(this.sidebar.active_space(), beta.as_ref(), "current painted choice must activate its Space");
                assert!(!this.spaces_open);
            }).unwrap();
            // 2. Older navigation revision: toggle and choice callbacks do nothing.
            this.update_in(cx, |this, _, cx| paint(this, true, cx)).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                let id = choice_id(this, window, &beta);
                let (toggle, choice) = (painted_space(TOGGLE), painted_space(id));
                this.sidebar.set_unread_only(true);
                this.sidebar.set_unread_only(false);
                toggle(this, window, cx);
                assert!(this.spaces_open, "retired toggle changed the menu");
                choice(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "retired choice activated");
                cx.notify();
            }).unwrap();
            // 3. Target replaced at the same index: the old callback must not choose the newcomer.
            wait(300).await;
            let old = this.update_in(cx, |this, window, _| {
                let id = choice_id(this, window, &beta);
                painted_space(id)
            }).unwrap();
            this.update_in(cx, |this, window, cx| {
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(beta.clone().unwrap()));
                upsert_space(this, account, "!gamma:fixture", "Gamma");
                old(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "callback of a replaced choice activated");
                cx.notify();
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                let gamma = this.sidebar.navigation_entries().into_iter().find(|e| e.name == "Gamma").unwrap().scope;
                old(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "retired choice survived repaint");
                let id = choice_id(this, window, &gamma);
                painted_space(id)(this, window, cx);
                assert_eq!(this.sidebar.active_space(), gamma.as_ref(), "fresh choice must activate");
                this.sidebar.set_space(None);
                this.set_spaces_open(true);
                cx.notify();
            }).unwrap();
            // 4. Menu hidden by the compact chat page: callbacks painted before are rejected; fresh ones work after return.
            wait(300).await;
            let (toggle, choice) = this.update_in(cx, |this, window, _| {
                let id = this.semantic_spaces(window).unwrap().choices[1].id;
                (painted_space(TOGGLE), painted_space(id))
            }).unwrap();
            this.update_in(cx, |this, window, cx| {
                this.set_compact_chat(true);
                toggle(this, window, cx);
                choice(this, window, cx);
                assert!(this.spaces_open && this.sidebar.active_space().is_none(), "hidden Spaces callback acted");
                this.set_compact_chat(false);
                toggle(this, window, cx);
                choice(this, window, cx);
                assert!(this.spaces_open && this.sidebar.active_space().is_none(), "callback survived hide/return");
                cx.notify();
            }).unwrap();
            wait(300).await;
            // 5. Real Configure with paint suspended rejects the retired callbacks before repaint.
            let fresh = this.update_in(cx, |_, _, _| {
                let fresh = painted_space(TOGGLE);
                PAINTED.lock().unwrap().clear();
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                navigation_resize_test::resize("1120", "760");
                fresh
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                assert!(window.viewport_size().width >= px(800.));
                fresh(this, window, cx);
                assert!(this.spaces_open, "toggle acted on native Configure before repaint");
                navigation_resize_test::resize("360", "480");
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                fresh(this, window, cx);
                assert!(this.spaces_open, "toggle acted on Configure back before repaint");
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                cx.notify();
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                fresh(this, window, cx);
                assert!(this.spaces_open, "retired toggle survived repaint");
                painted_space(TOGGLE)(this, window, cx);
                assert!(!this.spaces_open, "reacquired toggle must act");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 13 {
            // Painted rail pointer callbacks: current ones activate once, retired ones never, even before repaint after a native Configure.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let beta = this.update_in(cx, |this, _, _| this.sidebar.navigation_entries()[2].scope.clone()).unwrap();
            wait(300).await;
            let painted = this.update_in(cx, |this, window, cx| {
                this.sidebar.set_space(None);
                this.list_focus.focus(window);
                cx.notify();
                painted_rail(&beta)
            }).unwrap();
            wait(200).await;
            this.update_in(cx, |this, window, cx| {
                painted(this, window, cx);
                assert_eq!(this.sidebar.active_space(), beta.as_ref(), "current painted rail callback must activate its Space once");
                this.sidebar.set_space(None);
                this.sidebar.set_unread_only(true);
                this.sidebar.set_unread_only(false);
                painted(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "rail callback from an older navigation revision activated");
                cx.notify();
            }).unwrap();
            wait(200).await;
            let fresh = this.update_in(cx, |_, _, _| {
                let fresh = painted_rail(&beta);
                PAINTED.lock().unwrap().clear();
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                navigation_resize_test::resize("360", "480");
                fresh
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                assert_eq!(window.viewport_size().width, px(360.));
                assert!(PAINTED.lock().unwrap().is_empty(), "Configure must not repaint while suspended");
                fresh(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "rail callback activated on native Configure before repaint");
                navigation_resize_test::resize("1120", "760");
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                fresh(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "rail callback activated on native Configure back before repaint");
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                cx.notify();
            }).unwrap();
            wait(300).await;
            this.update_in(cx, |this, window, cx| {
                fresh(this, window, cx);
                assert!(this.sidebar.active_space().is_none(), "retired rail callback survived repaint");
                painted_rail(&beta)(this, window, cx);
                assert_eq!(this.sidebar.active_space(), beta.as_ref(), "reacquired rail callback must activate");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 12 {
            // Rail and compact Spaces activate once per completed key gesture, never on press/repeat or from a retired source.
            let compact = COMPACT.load(Ordering::SeqCst);
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let count = || ACTIVATIONS[2].load(Ordering::SeqCst);
            let entries = this.update_in(cx, |this, _, _| this.sidebar.navigation_entries()).unwrap();
            let (alpha, beta) = (entries[1].scope.clone(), entries[2].scope.clone());
            let account = scopes[0].account_id;
            // Each case: (prepare the focused source, observable state that a single activation changes).
            for key in ["space", "enter"] {
                for retire in [None, Some("revision"), Some("target"), Some("removed")] {
                    'attempt: loop {
                    this.update_in(cx, |this, window, cx| {
                        this.sidebar.set_space(None);
                        this.set_spaces_open(compact);
                        if compact { focus_choice(this, window, cx, &beta); } else { this.rail_ui.target = beta.clone(); this.rail_ui.focus.focus(window); }
                        cx.notify();
                    }).unwrap();
                    wait(300).await;
                    let before = count();
                    native_key(key, false);
                    wait(300).await;
                    this.update_in(cx, |this, _, _| assert!(this.sidebar.active_space().is_none() && count() == before, "unmatched release activated")).unwrap();
                    let downs = KEYDOWNS.load(Ordering::SeqCst);
                    let ups = UPS.load(Ordering::SeqCst);
                    native_key(key, true);
                    for ms in [100, 900, 150] {
                        wait(ms).await;
                        if UPS.load(Ordering::SeqCst) != ups { break; }
                        this.update_in(cx, |this, _, _| assert!(this.sidebar.active_space().is_none() && count() == before, "press/repeat activated: active={:?} count={} before={} open={}", this.sidebar.active_space(), count(), before, this.spaces_open)).unwrap();
                    }
                    if UPS.load(Ordering::SeqCst) != ups {
                        eprintln!("environment split a key repeat into release+press; retrying this gesture");
                        native_key(key, false);
                        wait(300).await;
                        continue 'attempt;
                    }
                    assert!(KEYDOWNS.load(Ordering::SeqCst) >= downs + 2, "private X server must deliver held-repeat keydowns");
                    match retire {
                        Some("revision") => this.update_in(cx, |this, _, _| { this.sidebar.set_unread_only(true); this.sidebar.set_unread_only(false); }).unwrap(),
                        Some("removed") => this.update_in(cx, |this, _, cx| {
                            this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(beta.clone().unwrap()));
                            cx.notify();
                        }).unwrap(),
                        Some(_) => this.update_in(cx, |this, window, cx| { if compact { focus_choice(this, window, cx, &alpha); } else { this.rail_ui.target = alpha.clone(); } }).unwrap(),
                        None => {}
                    }
                    native_key(key, false);
                    wait(300).await;
                    this.update_in(cx, |this, _, _| {
                        if retire == Some("removed") { upsert_space(this, account, "!beta:fixture", "Beta"); }
                        if retire.is_some() {
                            assert!(this.sidebar.active_space().is_none() && count() == before, "retired source activated: {retire:?}");
                        } else {
                            assert_eq!(count(), before + 1, "exactly one activation per gesture");
                            assert_eq!(this.sidebar.active_space(), beta.as_ref(), "activation chose the focused Space");
                        }
                    }).unwrap();
                    break;
                    }
                }
                // Compact toggle: one gesture flips the menu once.
                if compact {
                    this.update_in(cx, |this, window, cx| { this.set_spaces_open(false); this.spaces_shortcut_focus(window, cx); }).unwrap();
                    wait(300).await;
                    let before = count();
                    let ups = UPS.load(Ordering::SeqCst);
                    native_key(key, true);
                    wait(900).await;
                    if UPS.load(Ordering::SeqCst) != ups { eprintln!("environment split a key repeat; toggle gesture skipped for this key"); native_key(key, false); wait(300).await; continue; }
                    this.update_in(cx, |this, _, _| assert!(!this.spaces_open && count() == before, "toggle activated on press/repeat: open={} count={} before={} active={:?}", this.spaces_open, count(), before, this.sidebar.active_space())).unwrap();
                    native_key(key, false);
                    wait(300).await;
                    this.update_in(cx, |this, _, _| assert!(this.spaces_open && count() == before + 1, "toggle must open once on release")).unwrap();
                }
            }
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 21 {
            // Ctrl+PageUp / Ctrl+Home through the Shell key path: the row being read keeps its pixel offset when older rows arrive.
            this.update_in(cx, |this, window, cx| {
                this.list_focus.focus(window);
                this.timeline.as_ref().unwrap().scroll_to_row(20, px(7.));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            for key in ["ctrl-pageup", "ctrl-home"] {
                let (len, row) = this.update_in(cx, |this, _, _| {
                    let t = this.timeline.as_ref().unwrap();
                    (t.len(), t.top_row())
                }).unwrap();
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
                executor.timer(std::time::Duration::from_millis(250)).await;
                let after = this.update_in(cx, |this, _, _| {
                    let t = this.timeline.as_ref().unwrap();
                    (t.len(), t.top_row())
                }).unwrap();
                if key == "ctrl-pageup" {
                    assert_eq!(after.0, len + 25, "Ctrl+PageUp prepends one page");
                    assert_eq!(after.1, row, "same row at the same pixel offset after Ctrl+PageUp");
                } else {
                    assert_eq!((after.1).1, px(0.), "Ctrl+Home reaches the first row");
                }
            }
            // Older rows arriving at the very top: the first row stays put.
            let (len, row) = this.update_in(cx, |this, _, _| {
                let t = this.timeline.as_ref().unwrap();
                (t.len(), t.top_row())
            }).unwrap();
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("ctrl-pageup").unwrap(), cx); }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, _, cx| {
                let t = this.timeline.as_ref().unwrap();
                assert_eq!(t.len(), len + 25);
                assert_eq!(t.top_row(), row, "first row keeps its place after Ctrl+Home then Ctrl+PageUp");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 23 {
            // Connected Shell: the strip above the timeline shows only while an older page loads or after it failed.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let path = format!("/tmp/gui-out/{name}.png");
                let ok = std::process::Command::new("grim").arg(&path).status().map(|s| s.success()).unwrap_or(false);
                assert!(ok, "screenshot {name}");
            };
            wait(700).await;
            shot("p178-idle");
            let loading = this.update_in(cx, |this, _, cx| {
                let scope = this.active_scope.clone();
                this.history.select(Some(scope));
                cx.notify();
                this.history.status() == HistoryStatus::Loading
            }).unwrap();
            assert!(loading, "selecting the room starts loading");
            wait(400).await;
            shot("p178-loading");
            let failed = this.update_in(cx, |this, _, cx| {
                let tag = this.history.pagination_tag().expect("loading request tag");
                this.history.apply(matrix_engine::history::HistoryResult { tag, page: Err(matrix_engine::history::HistoryError::Unavailable) });
                cx.notify();
                this.history.status() == HistoryStatus::Failed
            }).unwrap();
            assert!(failed, "a failed page shows the failed state");
            wait(400).await;
            shot("p178-failed");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 22 {
            // Wheel and scrollbar-thumb drag: older rows arriving keep the row being read at the same pixel offset.
            this.update_in(cx, |this, _, cx| {
                this.timeline.as_ref().unwrap().scroll_to_row(20, px(7.));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            let read = |this: &mut Shell| { let t = this.timeline.as_ref().unwrap(); (t.len(), t.top_row()) };
            let centre = this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().viewport_centre()).unwrap();
            let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout).unwrap().lines().next().expect("window").to_string();
            let xdo = |args: &[&str]| assert!(std::process::Command::new("xdotool").args(args).status().unwrap().success());
            let start = this.update_in(cx, |this, _, _| read(this).1).unwrap();
            xdo(&["mousemove", "--window", &win, &(f32::from(centre.x) as i32).to_string(), &(f32::from(centre.y) as i32).to_string(), "click", "--repeat", "3", "5"]);
            executor.timer(std::time::Duration::from_millis(250)).await;
            let (len, row) = this.update_in(cx, |this, _, _| read(this)).unwrap();
            assert_ne!(row, start, "the wheel must have moved the viewport");
            this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().prepend_older()).unwrap();
            let after = this.update_in(cx, |this, _, _| read(this)).unwrap();
            assert_eq!(after.0, len + 25);
            assert_eq!(after.1, row, "same row at the same pixel offset after wheel scroll then prepend");
            // Thumb drag with the button still held when older rows arrive.
            executor.timer(std::time::Duration::from_millis(250)).await;
            let thumb = this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().thumb_centre().expect("scrollbar thumb painted")).unwrap();
            let row_before_drag = this.update_in(cx, |this, _, _| read(this).1).unwrap();
            let (tx, ty) = (f32::from(thumb.x) as i32, f32::from(thumb.y) as i32);
            xdo(&["mousemove", "--window", &win, &tx.to_string(), &ty.to_string(), "mousedown", "1", "mousemove_relative", "--", "0", "30"]);
            executor.timer(std::time::Duration::from_millis(250)).await;
            let (len, row) = this.update_in(cx, |this, _, _| read(this)).unwrap();
            assert_ne!(row, row_before_drag, "the thumb drag must have moved the viewport");
            this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().prepend_older()).unwrap();
            let after = this.update_in(cx, |this, _, _| read(this)).unwrap();
            assert_eq!(after.0, len + 25);
            assert_eq!(after.1, row, "same row at the same pixel offset after thumb drag then prepend");
            xdo(&["mouseup", "1"]);
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 10 {
            // Hidden Search (compact chat page, repaint suspended) must not be edited by native key dispatch.
            this.update_in(cx, |this, window, cx| {
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                this.set_compact_chat(true);
                this.search_focus.focus(window);
                assert!(this.search_focus.is_focused(window));
                cx.notify();
            }).unwrap();
            for text in ["è", "🦀"] {
                let mut key = Keystroke::parse("x").unwrap();
                key.key_char = Some(text.into());
                cx.update(|window, cx| { window.dispatch_keystroke(key, cx); }).unwrap();
            }
            for name in ["backspace", "escape"] {
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(name).unwrap(), cx); }).unwrap();
            }
            this.update_in(cx, |this, window, cx| {
                assert_eq!(this.sidebar.query(), "", "hidden Search must reject native edits");
                assert!(this.compact_chat);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.composer.read(cx).is_focused(window));
                assert_eq!(this.sidebar.query(), "");
                preserve_drafts_and_capture(this, &scopes, window, cx, "hidden-search-edit");
                cx.quit();
            }).unwrap();
            return;
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        if DYNAMIC.load(Ordering::SeqCst) == 11 {
            // A translated FocusSearch queued while Search is hidden must be dropped by the Shell drain, and honoured once Search is visible again.
            this.update_in(cx, |this, window, cx| {
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                this.set_compact_chat(true);
                this.composer.update(cx, |composer, cx| composer.focus_from_accessibility(window, cx));
                this.accessibility.queue_action(matrix_desktop::accessibility::NavigationAction::FocusSearch);
                Shell::drain_navigation_actions(this, window, cx);
                assert!(!this.search_focus.is_focused(window), "hidden Search must not take queued focus");
                assert_eq!(this.sidebar.query(), "");
                assert!(this.compact_chat);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                this.set_compact_chat(false);
                this.accessibility.queue_action(matrix_desktop::accessibility::NavigationAction::FocusSearch);
                Shell::drain_navigation_actions(this, window, cx);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.search_focus.is_focused(window), "visible Search must take queued focus");
                assert_eq!(this.sidebar.query(), "");
                preserve_drafts_and_capture(this, &scopes, window, cx, "hidden-search-queued-focus");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 6 {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("shift-tab").unwrap(), cx); }).unwrap();
            let beta = this.update_in(cx, |this, window, cx| {
                assert!(this.spaces_ui.focus.is_focused(window));
                assert!(!this.spaces_open);
                let alpha = this.sidebar.navigation_entries()[1].scope.clone().unwrap();
                let beta = this.sidebar.navigation_entries()[2].scope.clone().unwrap();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(alpha));
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary { scope: beta.clone(), name: "Beta no longer a Space".into(), unread: 0, info: Default::default() }));
                cx.notify();
                beta
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.theme_focus.is_focused(window));
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary { scope: beta, name: "Beta replacement".into(), unread: 0, info: matrix_engine::RoomInfo { is_space: true, ..Default::default() } }));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.theme_focus.is_focused(window), "replacement eligibility must not steal focus back");
                assert!(!this.spaces_open);
                assert_eq!(this.sidebar.active_space(), None);
                assert!(history(this));
                preserve_drafts_and_capture(this, &scopes, window, cx, "closed-reclassified-replaced");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 5 {
            let space = this.update_in(cx, |this, window, cx| {
                let space = this.sidebar.navigation_entries()[1].scope.clone().unwrap();
                let name = this.sidebar.rooms()[1].name.clone();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                    scope: scopes[1].clone(), name, unread: 3, info: Default::default(),
                }));
                let name = this.sidebar.rooms().iter().find(|r| r.scope == space).unwrap().name.clone();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                    scope: space.clone(), name, unread: 0,
                    info: matrix_engine::RoomInfo { is_space: true, children: vec![scopes[1].room_id.clone()], ..Default::default() },
                }));
                this.list_focus.focus(window);
                cx.notify();
                space
            }).unwrap();
            for dark in [false, true] {
                for (query, unread, group, count) in [("",false,false,302), ("",true,false,1), ("",true,true,1), ("no matches ",true,true,0), ("",false,true,2), ("",false,false,302)] {
                    this.update_in(cx, |this, _, cx| {
                        if this.dark != dark { this.toggle_theme(cx); }
                        this.sidebar.set_query(query);
                        this.sidebar.set_unread_only(unread);
                        this.sidebar.set_space(group.then(|| space.clone()));
                        PAINTED.lock().unwrap().clear();
                        cx.notify();
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(120)).await;
                    this.update_in(cx, |this, window, cx| {
                        assert_eq!(this.sidebar.visible_indices().len(), count);
                        assert!(this.list_focus.is_focused(window));
                        assert_eq!(this.sidebar.query(), query);
                        assert_eq!(this.sidebar.unread_only(), unread);
                        assert_eq!(this.sidebar.active_space(), group.then_some(&space));
                        assert!(history(this));
                        let bounds = PAINTED.lock().unwrap()["list"];
                        assert!(bounds.left() >= px(0.) && bounds.right() <= window.viewport_size().width && bounds.bottom() <= window.viewport_size().height);
                        eprintln!("painted visible list focus: dark={dark}, query={query:?}, unread={unread}, Space={group}, exact visible={count}, bounds={bounds:?}; hidden selected retained");
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("filters-{dark}-{count}-{unread}-{group}"));
                    }).unwrap();
                }
            }
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 4 {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("shift-tab").unwrap(), cx); }).unwrap();
            this.update_in(cx, |this, window, _| {
                assert!(this.spaces_ui.focus.is_focused(window));
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                assert_eq!(window.viewport_size().width, px(360.));
                navigation_resize_test::resize("1120", "760");
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert_eq!(window.viewport_size().width, px(1120.));
                assert!(this.theme_focus.is_focused(window), "native Configure must move hidden compact focus to visible Theme before repaint");
                this.rail_ui.focus.focus(window);
                navigation_resize_test::resize("360", "480");
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert_eq!(window.viewport_size().width, px(360.));
                assert!(this.search_focus.is_focused(window), "hidden rail must reach Search during native Configure without repaint");
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                this.set_compact_chat(true);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.composer.read(cx).is_focused(window), "hiding sidebar must reach existing composer, never hidden Search");
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                cx.notify();
            }).unwrap();
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("alt-left").unwrap(), cx); }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(!this.compact_chat);
                assert!(this.list_focus.is_focused(window));
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                eprintln!("real Configure FROZEN compact→desktop Theme→compact Search; list/chat/list composer→list unchanged history");
                preserve_drafts_and_capture(this, &scopes, window, cx, "compact-return");
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                navigation_resize_test::resize("1120", "760");
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert_eq!(window.viewport_size().width, px(1120.));
                assert!(this.list_focus.is_focused(window));
                PAINTED.lock().unwrap().clear();
                navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.list_focus.is_focused(window));
                for label in ["theme", "search", "list"] {
                    let bounds = PAINTED.lock().unwrap()[label];
                    assert!(bounds.left() >= px(72.) && bounds.right() <= window.viewport_size().width && bounds.bottom() <= window.viewport_size().height);
                    eprintln!("fresh post-Configure desktop {label} bounds={bounds:?}, native focus=list");
                }
                assert!(history(this));
                preserve_drafts_and_capture(this, &scopes, window, cx, "desktop-return");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 3 {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("shift-tab").unwrap(), cx); }).unwrap();
            let captured = this.update_in(cx, |this, _window, cx| {
                let callback = this.sidebar.navigation_callback();
                assert!(callback(&this.sidebar));
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let intent = {
                    let view = this.semantic_spaces(_window).unwrap();
                    view.action(&accesskit::ActionRequest { action: accesskit::Action::Focus, target_tree: accesskit::TreeId::ROOT, target_node: view.focused.unwrap(), data: None }).unwrap()
                };
                let scope = this.sidebar.navigation_entries()[2].scope.clone().unwrap();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(scope));
                cx.notify();
                (callback, #[cfg(all(feature = "linux-a11y", target_os = "linux"))] intent)
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.spaces_ui.focus.is_focused(window));
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                assert_eq!(this.spaces_ui.target, None, "retired compact choice must reach visible toggle");
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                assert_eq!(this.semantic_spaces(window).unwrap().focused, Some(matrix_desktop::accessibility::spaces::TOGGLE), "retired compact choice must reach visible toggle");
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                assert!(!captured.0(&this.sidebar), "painted source guard must reject the retired visit");
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    this.apply_semantic_spaces(captured.1.clone(), window, cx);
                    assert_eq!(this.semantic_spaces(window).unwrap().focused, Some(matrix_desktop::accessibility::spaces::TOGGLE));
                    eprintln!("already-translated retired Focus rejected at actual Shell boundary: {:?}", captured.1);
                }
                eprintln!("retired compact choice: current native focus=toggle, no activation");
                preserve_drafts_and_capture(this, &scopes, window, cx, &format!("dynamic{}", DYNAMIC.load(Ordering::SeqCst)));
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 2 {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("shift-tab").unwrap(), cx); }).unwrap();
            let beta = this.update_in(cx, |this, window, cx| {
                assert!(this.spaces_ui.focus.is_focused(window));
                let beta = this.sidebar.navigation_entries()[2].scope.clone().unwrap();
                let alpha = this.sidebar.navigation_entries()[1].scope.clone().unwrap();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(alpha));
                cx.notify();
                beta
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.spaces_ui.focus.is_focused(window));
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                assert_eq!(this.spaces_ui.target, Some(Some(beta.clone())), "removing another choice must retain focus on Beta by identity");
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    let view = this.semantic_spaces(window).unwrap();
                    assert_eq!(view.focused, Some(view.choices.iter().find(|c| c.scope == Some(beta.clone())).unwrap().id));
                }
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(beta));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.theme_focus.is_focused(window), "last joined Space removal must reach visible Theme, not hidden menu");
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert_eq!(this.sidebar.active_space(), None);
                assert!(history(this));
                eprintln!("compact: removed preceding choice retains Beta; last Space removal focuses Theme without activation");
                preserve_drafts_and_capture(this, &scopes, window, cx, &format!("dynamic{}", DYNAMIC.load(Ordering::SeqCst)));
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 1 {
            let retired = this.update_in(cx, |this, _, cx| {
                let retired = this.rail_ui.target.clone().unwrap();
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Remove(retired.clone()));
                cx.notify();
                retired
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.rail_ui.focus.is_focused(window));
                assert_eq!(this.rail_ui.target, None, "retired rail choice must focus existing All, not an invisible scope");
                assert_eq!(this.sidebar.active_space(), None);
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
                eprintln!("retired {retired:?}: current native rail focus=All, same conversation/history");
                preserve_drafts_and_capture(this, &scopes, window, cx, &format!("dynamic{}", DYNAMIC.load(Ordering::SeqCst)));
                cx.quit();
            }).unwrap();
            return;
        }
        let steps = if COMPACT.load(Ordering::SeqCst) && OPEN.load(Ordering::SeqCst) {
            vec![("shift-tab", "space3"), ("shift-tab", "space2"), ("shift-tab", "space1"), ("shift-tab", "space0"), ("shift-tab", "notifications"), ("shift-tab", "devices"), ("shift-tab", "list"), ("tab", "devices"), ("tab", "notifications"), ("tab", "space0"), ("tab", "space1"), ("tab", "space2"), ("tab", "space3"), ("tab", "theme"), ("tab", "unread"), ("tab", "search"), ("tab", "list")]
        } else if !COMPACT.load(Ordering::SeqCst) && SPACES.load(Ordering::SeqCst) {
            vec![("tab", "theme"), ("shift-tab", "rail2"), ("shift-tab", "rail1"), ("shift-tab", "rail0"), ("tab", "rail1"), ("tab", "rail2"), ("tab", "theme"), ("tab", "unread"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "theme"), ("shift-tab", "rail2")]
        } else if COMPACT.load(Ordering::SeqCst) {
            if CONNECTED.load(Ordering::SeqCst) {
                vec![("tab", "unread"), ("tab", "search"), ("tab", "list"), ("tab", "devices"), ("tab", "notifications"), ("tab", "space0"), ("shift-tab", "notifications"), ("shift-tab", "devices"), ("shift-tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "theme"), ("shift-tab", "space0"), ("tab", "theme")]
            } else {
                vec![("tab", "search"), ("tab", "list"), ("tab", "theme"), ("shift-tab", "list"), ("shift-tab", "search"), ("shift-tab", "theme")]
            }
        } else if CONNECTED.load(Ordering::SeqCst) {
            vec![("tab", "theme"), ("tab", "unread"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "theme"), ("shift-tab", "rail")]
        } else {
            vec![("tab", "theme"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "theme"), ("shift-tab", "rail")]
        };
        for (key, expected) in steps {
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
            }).unwrap();
            this.update_in(cx, |this, window, cx| {
                let focused = match expected {
                    "theme" => this.theme_focus.is_focused(window),
                    "unread" => this.unread_focus.is_focused(window),
                    "search" => this.search_focus.is_focused(window),
                    "list" => this.list_focus.is_focused(window),
                    "devices" => this.opener_ui.focused(matrix_desktop::panel_openers::Control::Devices, window),
                    "notifications" => this.opener_ui.focused(matrix_desktop::panel_openers::Control::Notifications, window),
                    "rail" => this.rail_ui.focus.is_focused(window),
                    name if name.starts_with("space") => {
                        let index = name[5..].parse::<usize>().unwrap();
                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                        { this.semantic_spaces(window).is_some_and(|view| view.focused == Some(view.nodes()[index].0)) }
                        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                        { this.spaces_ui.focus.is_focused(window) && this.spaces_ui.target == index.checked_sub(1).map(|i| this.sidebar.navigation_entries()[i].scope.clone()) }
                    },
                    name if name.starts_with("rail") => {
                        let index = name[4..].parse::<usize>().unwrap();
                        this.rail_ui.focus.is_focused(window) && this.rail_ui.target == this.sidebar.navigation_entries()[index].scope
                    },
                    _ => unreachable!(),
                };
                assert!(focused, "{key} must reach {expected} through actual Shell dispatch");
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(!this.dark && !this.sidebar.unread_only());
                assert_eq!(this.sidebar.active_space(), None);
                assert_eq!(this.spaces_open, OPEN.load(Ordering::SeqCst));
                assert!(!this.compact_chat);
                assert!(history(this), "focus must preserve actual timeline allocation and every scoped event ID");
                this.composer.update(cx, |v,cx| {
                    let mut actual=None;
                    assert_eq!(v.text_for_range(0..65536,&mut actual,window,cx).as_deref(), Some(DRAFTS[0]));
                });
                eprintln!("actual capture/bubble {key} => {expected}; width={:?}; selected/history/draft unchanged", window.viewport_size().width);
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(80)).await;
            if let Ok(out) = std::env::var("P101_CAPTURE_DIR") {
                let ids = std::process::Command::new("xdotool")
                    .args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()])
                    .output().unwrap();
                assert!(ids.status.success());
                let ids = String::from_utf8(ids.stdout).unwrap();
                let id = ids.lines().next().unwrap();
                assert!(std::process::Command::new("magick")
                    .args(["import", "-window", id])
                    .arg(std::path::Path::new(&out).join(format!("{key}-{expected}.png")))
                    .status().unwrap().success());
            }
        }
        if SPACES.load(Ordering::SeqCst) {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("ctrl-shift-s").unwrap(), cx); }).unwrap();
            this.update_in(cx, |this, window, _| {
                if COMPACT.load(Ordering::SeqCst) {
                    assert!(this.spaces_ui.focus.is_focused(window), "shortcut must enter visible Space toggle, never room list");
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    assert_eq!(this.semantic_spaces(window).unwrap().focused, Some(matrix_desktop::accessibility::spaces::TOGGLE));
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    assert_eq!(this.spaces_ui.target, None);
                } else {
                    assert!(this.rail_ui.focus.is_focused(window));
                    assert_eq!(this.rail_ui.target, None);
                }
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
            }).unwrap();
        }
        if COMPACT.load(Ordering::SeqCst) && SPACES.load(Ordering::SeqCst) {
            // The shortcut already flipped the menu; one completed Enter gesture flips it back.
            this.update_in(cx, |this, _, _| assert_eq!(this.spaces_open, !OPEN.load(Ordering::SeqCst))).unwrap();
            native_tap("enter", &executor).await;
            this.update_in(cx, |this, window, _| {
                assert_eq!(this.spaces_open, OPEN.load(Ordering::SeqCst), "Enter on toggle must only change menu expansion");
                assert!(!this.compact_chat, "Space control Enter must never open the selected conversation");
                assert!(this.spaces_ui.focus.is_focused(window));
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
            }).unwrap();
        }
        if COMPACT.load(Ordering::SeqCst) && OPEN.load(Ordering::SeqCst) {
            for key in ["tab", "tab"] {
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
            }
            native_tap("enter", &executor).await;
            this.update_in(cx, |this, window, _| {
                assert_eq!(this.sidebar.active_space(), this.sidebar.navigation_entries()[1].scope.as_ref(), "Enter must choose only the focused joined Space");
                assert!(!this.spaces_open && !this.compact_chat);
                assert!(this.spaces_ui.focus.is_focused(window));
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(history(this));
            }).unwrap();
        }
        this.update_in(cx, |this, window, _cx| {
            this.connected = false;
            this.sidebar.set_space(None);
            this.list_focus.focus(window);
        }).unwrap();
        for (key, index) in [("down", 1), ("up", 0)] {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
            this.update_in(cx, |this, window, cx| {
                assert!(this.list_focus.is_focused(window));
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[index]);
                assert_eq!(this.active_scope, scopes[index]);
                this.composer.update(cx, |v,cx| {
                    let mut actual=None;
                    assert_eq!(v.text_for_range(0..65536,&mut actual,window,cx).as_deref(), Some(DRAFTS[index]));
                });
                if index == 0 { assert!(history(this)); }
                eprintln!("list arrow {key}: exact room/history/draft{index}");
            }).unwrap();
        }
        this.update_in(cx, |this, window, cx| {
            assert!(this.sidebar.select(&scopes[1]));
            this.sync_room(cx);
            this.composer.update(cx, |v,cx| {
                let mut actual=None;
                assert_eq!(v.text_for_range(0..65536,&mut actual,window,cx).as_deref(), Some(DRAFTS[1]));
            });
            cx.quit();
        }).unwrap();
    }).detach();
}

#[test]
#[ignore = "requires private display"]
fn closed_compact_menu_reclassification_and_replacement_keep_visible_focus() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(6, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn combined_filters_keep_visible_list_focus_and_hidden_conversation() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(5, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_combined_filters_keep_visible_list_focus_and_hidden_conversation() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(5, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn native_resize_and_hidden_sidebar_focus_reach_visible_existing_controls() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(4, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    OPEN.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn retired_compact_space_focus_falls_back_to_visible_toggle() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(3, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    OPEN.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_space_focus_survives_other_removal_then_falls_back_when_empty() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(2, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    OPEN.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn retired_desktop_space_focus_falls_back_to_all_without_activation() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(1, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[test]
#[ignore = "requires private display"]
fn translated_theme_unread_intents_reject_retired_sources() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    COMPACT.store(
        std::env::var("P110_COMPACT").as_deref() == Ok("1"),
        Ordering::SeqCst,
    );
    DYNAMIC.store(9, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn theme_unread_pointer_callbacks_reject_retired_sources() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    COMPACT.store(
        std::env::var("P110_COMPACT").as_deref() == Ok("1"),
        Ordering::SeqCst,
    );
    DYNAMIC.store(8, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn theme_unread_require_completed_current_keyboard_gestures() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    COMPACT.store(
        std::env::var("P110_COMPACT").as_deref() == Ok("1"),
        Ordering::SeqCst,
    );
    DYNAMIC.store(7, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn desktop_sidebar_tab_segment_is_bidirectional_without_changing_conversation() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn connected_sidebar_includes_unread_without_activating_it() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_list_without_spaces_wraps_through_visible_controls() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    COMPACT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn connected_compact_closed_spaces_join_the_visible_sidebar_ring() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn connected_compact_open_spaces_traverse_all_visible_choices() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    OPEN.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn desktop_joined_spaces_traverse_without_changing_filter_or_conversation() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn hidden_search_rejects_native_key_edits_and_keeps_state() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(10, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[ignore = "requires private display"]
fn queued_focus_search_is_dropped_while_search_hidden() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(11, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn rail_requires_completed_current_key_gestures() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(12, Ordering::SeqCst);
    COMPACT.store(false, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_spaces_require_completed_current_key_gestures() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(12, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn painted_rail_pointer_callbacks_retire_with_their_source() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(13, Ordering::SeqCst);
    COMPACT.store(false, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[ignore = "requires private display"]
fn painted_compact_spaces_callbacks_retire_with_their_source() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(14, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[ignore = "requires private display"]
fn translated_spaces_intents_activate_once_and_reject_stale_sources() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(15, Ordering::SeqCst);
    COMPACT.store(std::env::var("P111_COMPACT").as_deref() == Ok("1"), Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_list_chat_seam_runs_through_footer_and_info_both_ways() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_list_seam_runs_through_footer_both_ways() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_chat_seam_runs_from_back_through_info_both_ways() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CHAT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_chat_end_wraps_to_first_rail_control_both_ways() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(17, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_chat_end_without_eligible_send_wraps_from_composer() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(17, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    EMPTY.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_chat_end_wraps_to_back_both_ways() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(17, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CHAT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn selected_hidden_list_keeps_ring_and_selection() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(18, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn empty_list_keeps_ring_and_selection() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(18, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    EMPTY.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn native_resize_moves_every_ring_control_to_a_visible_one() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(19, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn theme_change_keeps_every_ring_control_focused() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(20, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn page_up_and_home_keep_the_row_being_read_in_place() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(21, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wheel_and_scrollbar_drag_keep_the_row_being_read_in_place() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(22, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn older_history_strip_shows_only_while_loading_or_after_a_failure() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(23, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}
