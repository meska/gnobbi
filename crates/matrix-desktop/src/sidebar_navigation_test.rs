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
pub(crate) static ACTIVATIONS: [std::sync::atomic::AtomicUsize; 2] = [
    std::sync::atomic::AtomicUsize::new(0), // unread filter
    std::sync::atomic::AtomicUsize::new(0), // rail / compact Spaces
];
const DRAFTS: [&str; 2] = [" caffè 🦀\nprima ", " δεύτερη 🚀\nbozza "];

type PointerCallback = std::rc::Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
thread_local! {
    static POINTER: std::cell::RefCell<Option<PointerCallback>> = Default::default();
}
pub(crate) fn capture_pointer(
    callback: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> PointerCallback {
    let callback: PointerCallback = std::rc::Rc::new(callback);
    if ACTIVE.load(Ordering::SeqCst) {
        POINTER.with_borrow_mut(|painted| *painted = Some(callback.clone()));
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
    RAILS.with_borrow(|rails| {
        rails
            .get(scope)
            .expect("rail entry was never painted")
            .clone()
    })
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
    SPACE_NODES.with_borrow(|nodes| {
        nodes
            .get(&id)
            .expect("compact Spaces node was never painted")
            .clone()
    })
}
fn painted_pointer() -> PointerCallback {
    assert!(PAINTED.lock().unwrap().contains_key("unread"));
    POINTER.with_borrow(|painted| painted.as_ref().unwrap().clone())
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

const RING: [&str; 9] = [
    "list",
    "search",
    "unread",
    "devices",
    "notifications",
    "info",
    "entry",
    "back",
    "none",
];

fn ring_label(this: &Shell, window: &Window, cx: &App) -> &'static str {
    use matrix_desktop::panel_openers::Control;
    let entry = {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            this.history_focus.is_focused(window) || this.composer.read(cx).is_focused(window)
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            this.composer.read(cx).is_focused(window)
        }
    };
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    if this.back_ui.focus.is_focused(window) {
        return "back";
    }
    [
        (this.list_focus.is_focused(window), "list"),
        (this.search_focus.is_focused(window), "search"),
        (this.unread_focus.is_focused(window), "unread"),
        (this.opener_ui.focused(Control::Devices, window), "devices"),
        (
            this.opener_ui.focused(Control::Notifications, window),
            "notifications",
        ),
        (this.opener_ui.focused(Control::Info, window), "info"),
        (entry, "entry"),
    ]
    .into_iter()
    .find_map(|(focused, name)| focused.then_some(name))
    .unwrap_or(RING[8])
}

fn ring_focus(this: &mut Shell, label: &str, window: &mut Window, cx: &mut Context<Shell>) {
    use matrix_desktop::panel_openers::{Control, Intent};
    let opener = |control| Intent {
        control,
        source: 0,
        focus_only: true,
    };
    let control = match label {
        "list" => return this.list_focus.focus(window),
        "search" => return this.search_focus.focus(window),
        "unread" => return this.unread_focus.focus(window),
        "devices" => Control::Devices,
        "notifications" => Control::Notifications,
        "info" => Control::Info,
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        "entry" => return this.history_focus.focus(window),
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        "entry" => {
            return this
                .composer
                .update(cx, |v, cx| v.focus_from_accessibility(window, cx));
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        "back" => return this.back_ui.focus.focus(window),
        _ => unreachable!(),
    };
    let source = this
        .opener_ui
        .source(control)
        .expect("opener must be visible to start");
    this.apply_opener(
        Intent {
            source,
            ..opener(control)
        },
        window,
        cx,
    );
}

fn ring_visible(label: &str, wide: bool, chat: bool) -> bool {
    match (wide, chat) {
        (true, _) => label != "back",
        (false, false) => ["list", "search", "unread"].contains(&label),
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
    eprintln!("{label}: exact dual multiline Unicode drafts and selected/active scope preserved");
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
    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(
        matrix_engine::RoomSummary {
            scope: RoomScope {
                account_id: account,
                room_id: matrix_domain::RoomId::new(id),
            },
            name: name.into(),
            unread: 0,
            info: matrix_engine::RoomInfo {
                is_space: true,
                ..Default::default()
            },
        },
    ));
}

fn focus_choice(
    this: &mut Shell,
    window: &mut Window,
    cx: &mut Context<Shell>,
    scope: &Option<RoomScope>,
) {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    {
        let view = this.semantic_spaces(window).unwrap();
        let id = view.choices.iter().find(|c| c.scope == *scope).unwrap().id;
        this.apply_semantic_spaces(
            matrix_desktop::accessibility::spaces::SpacesAction::Focus {
                revision: view.revision,
                account: view.account,
                id,
            },
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
        title: matrix_desktop::window::TITLE.into(),
        new_chat: Default::default(),
        #[cfg(feature = "live")]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: this.sidebar.navigation_revision(),
        icon_choice: None,
        unread_filter: this.semantic_unread(window),
        new_room: None,
        devices: Default::default(),
        settings: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: this.theme.is_dark(),
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

fn activation_callback(this: &Shell, window: &Window) -> PointerCallback {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    if DYNAMIC.load(Ordering::SeqCst) == 9 {
        let session = sidebar_session(this, window);
        let id = session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Solo stanze non lette"))
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
                let matrix_desktop::accessibility::NavigationAction::Filter(action) = intent else {
                    unreachable!("the unread control only translates to filter actions")
                };
                this.apply_semantic_filter(*action, window, cx);
            }
            if current {
                assert!(
                    this.unread_focus.is_focused(window),
                    "fresh translated Focus must reach its current native control"
                );
            }
            eprintln!(
                "actual pretranslated SemanticSession Focus/Click; revision={}; native focus unread={}",
                this.sidebar.navigation_revision(),
                this.unread_focus.is_focused(window)
            );
        });
    }
    let _ = (this, window);
    painted_pointer()
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
                this.menu_focus.focus(window);
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
            {
                for transition in ["query", "unread", "details", "notifications", "devices", "account", "offline", "compact-chat"] {
                    if transition == "compact-chat" && !COMPACT.load(Ordering::SeqCst) { continue; }
                    executor.timer(std::time::Duration::from_millis(150)).await;
                    let callback = this.update_in(cx, |this, window, cx| {
                        let callback = activation_callback(this, window);
                        let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                        this.list_focus.focus(window);
                        pointer_transition(this, transition, true);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "retired painted callback activated during {transition}");
                        assert!(this.list_focus.is_focused(window), "rejected pointer changed focus");
                        pointer_transition(this, transition, false);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "retired painted callback activated after {transition} away/back");
                        assert!(this.list_focus.is_focused(window));
                        assert!(history(this));
                        eprintln!("painted pointer retired during/after {transition}, revision={}", this.sidebar.navigation_revision());
                        cx.notify();
                        callback
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(150)).await;
                    this.update_in(cx, |this, window, cx| {
                        let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                        callback(this, window, cx);
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "fresh paint resurrected old pointer");
                        activation_callback(this, window)(this, window, cx);
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before + 1, "reacquired painted callback must activate exactly once");
                        this.sidebar.set_unread_only(false);
                        assert!(history(this));
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("pointer-{transition}"));
                        cx.notify();
                    }).unwrap();
                }
            }
            executor.timer(std::time::Duration::from_millis(150)).await;
            let callback = this.update_in(cx, |this, window, _| {
                let callback = activation_callback(this, window);
                PAINTED.lock().unwrap().clear();
                navigation_resize_test::FROZEN.store(true, Ordering::SeqCst);
                if COMPACT.load(Ordering::SeqCst) { navigation_resize_test::resize("1120", "760"); }
                else { navigation_resize_test::resize("360", "480"); }
                callback
            }).unwrap();
            for returned in [false, true] {
                executor.timer(std::time::Duration::from_millis(200)).await;
                this.update_in(cx, |this, window, cx| {
                    let compact = COMPACT.load(Ordering::SeqCst) == returned;
                    assert_eq!(window.viewport_size().width, if compact { px(360.) } else { px(1120.) });
                    assert!(PAINTED.lock().unwrap().is_empty(), "Configure must not repaint while suspended");
                    this.list_focus.focus(window);
                    let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                    callback(this, window, cx);
                    assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "old painted source activated on native Configure");
                    assert!(this.list_focus.is_focused(window));
                    assert!(history(this));
                    eprintln!("actual native Configure returned={returned}, viewport={:?}, no intermediate paint, callback rejected", window.viewport_size());
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
                let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                callback(this, window, cx);
                assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before);
                activation_callback(this, window)(this, window, cx);
                assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before + 1);
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
                vec![("tab", "menu"), ("shift-tab", "list")]
            } else {
                vec![("tab", "info"), ("tab", "entry"), ("shift-tab", "info"), ("shift-tab", "list")]
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
                        "menu" => this.menu_focus.is_focused(window),
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
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch (focus is on {})", ring_label(this, window, cx));
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "focus must not open a panel");
                    assert_eq!(this.compact_chat, chat);
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.theme.is_dark() && !this.sidebar.unread_only());
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
                (true, false, "info"), (true, false, "entry"),
                (true, false, "unread"), (true, false, "search"), (true, false, "list"),
                (true, true, "info"), (true, true, "entry"),
                (false, true, "info"), (false, true, "entry"),
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                (false, true, "back"),
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
                    // The unread filter is only painted where the sidebar is visible.
                    (wide || !chat).then(|| activation_callback(this, window))
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
                    let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                    let draft_before = this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) });
                    for callback in callbacks.iter() { callback(this, window, cx); }
                    for intent in retired.into_iter().flatten() {
                        let hidden = this.opener_ui.source(intent.control) != Some(intent.source);
                        if hidden { this.apply_opener(intent, window, cx); }
                    }
                    assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "retired unread callback acted");
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
            for label in ["list", "search", "unread", "info", "entry"] {
                cases.push((true, false, label));
            }
            for label in ["list", "search", "unread"] {
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
                let (callback, intents) = this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::{Control, Intent};
                    let callback = activation_callback(this, window);
                    let intents = Control::ALL.map(|control| this.opener_ui.source(control).map(|source| Intent { control, source, focus_only: false }));
                    this.set_compact_chat(chat);
                    cx.notify();
                    (callback, intents)
                }).unwrap();
                wait(200).await;
                let dark_before = this.update_in(cx, |this, window, cx| {
                    ring_focus(this, start, window, cx);
                    assert_eq!(ring_label(this, window, cx), start, "setup must focus {start}");
                    let dark = this.theme.is_dark();
                    this.toggle_theme(cx);
                    assert_eq!(this.theme.is_dark(), !dark, "theme must flip");
                    assert_eq!(ring_label(this, window, cx), start, "theme change moved focus off {start}");
                    cx.notify();
                    dark
                }).unwrap();
                wait(300).await;
                this.update_in(cx, |this, window, cx| {
                    let landed = ring_label(this, window, cx);
                    eprintln!("theme wide={wide} chat={chat}: {start} -> {landed} after repaint, dark={}", this.theme.is_dark());
                    assert_eq!(landed, start, "focus must survive the repaint of the new theme");
                    assert_eq!(this.theme.is_dark(), !dark_before);
                    assert_eq!(this.compact_chat, chat);
                    // Hidden sidebar callbacks and openers stay inert after the theme change.
                    let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                    let draft_before = this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) });
                    if !wide && chat {
                        callback(this, window, cx);
                    }
                    for intent in intents.into_iter().flatten() {
                        if this.opener_ui.source(intent.control) != Some(intent.source) { this.apply_opener(intent, window, cx); }
                    }
                    assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "hidden callback acted after theme change");
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "hidden opener opened a panel");
                    assert_eq!(this.composer.update(cx, |v, cx| { let mut a = None; v.text_for_range(0..65536, &mut a, window, cx) }), draft_before);
                    assert_eq!(draft_before.as_deref(), Some(DRAFTS[0]), "draft must be intact, nothing sent");
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.sidebar.unread_only());
                    assert!(history(this));
                    assert_eq!(ring_label(this, window, cx), start);
                    this.toggle_theme(cx); // back to the starting theme for the next case
                    assert_eq!(this.theme.is_dark(), dark_before);
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
            // The painted unread-filter callback is captured while still visible; the filter change retires them.
            executor.timer(std::time::Duration::from_millis(300)).await;
            let painted = this.update_in(cx, |this, window, _| activation_callback(this, window)).unwrap();
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
                let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                let unread = this.sidebar.unread_only();
                painted(this, window, cx);
                assert!(this.list_focus.is_focused(window), "retired callbacks must not steal focus");
                assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before, "retired callback must not act");
                assert!(!this.theme.is_dark() && this.sidebar.unread_only() == unread);
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                eprintln!("hidden-ring empty={empty}: retired unread callback inert");
            }).unwrap();
            let steps = [
                ("tab", "info"), ("tab", "entry"),
                ("shift-tab", "info"), ("shift-tab", "list"),
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
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch (focus is on {})", ring_label(this, window, cx));
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
            let first = if chat { "back" } else { "menu" };
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
            steps.push(("tab", if chat { "info" } else { "rail0" }));
            this.update_in(cx, |this, window, cx| {
                if chat {
                    this.set_compact_chat(true);
                    this.back_focus_handle().focus(window);
                } else {
                    this.menu_focus.focus(window);
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
            if !chat { steps.insert(0, ("noop", "menu")); }
            for (key, expected) in steps {
                if key != "noop" {
                    cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
                }
                this.update_in(cx, |this, window, cx| {
                    use matrix_desktop::panel_openers::Control;
                    let focused = match expected {
                        "back" => this.back_focus_handle().is_focused(window),
                        "menu" => this.menu_focus.is_focused(window),
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
                    assert!(focused, "{key} must reach {expected} through actual Shell dispatch (focus is on {})", ring_label(this, window, cx));
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open, "focus must not open a panel");
                    assert_eq!(this.compact_chat, chat);
                    assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                    assert_eq!(this.active_scope, scopes[0]);
                    assert!(!this.theme.is_dark() && !this.sidebar.unread_only());
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
            {
                this.update_in(cx, |this, window, _| this.unread_focus.focus(window)).unwrap();
                executor.timer(std::time::Duration::from_millis(120)).await;
                for key in ["space", "enter"] {
                    let before = ACTIVATIONS[0].load(Ordering::SeqCst);
                    // A release without a matching press must not activate a painted control.
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, _, _| {
                        assert!(!this.sidebar.unread_only(), "unmatched release activated Unread");
                    }).unwrap();
                    let downs_before = KEYDOWNS.load(Ordering::SeqCst);
                    native_key(key, true);
                    for wait in [100, 900, 150] {
                        executor.timer(std::time::Duration::from_millis(wait)).await;
                        this.update_in(cx, |this, _, _| {
                            assert!(!this.sidebar.unread_only(), "press/repeat activated Unread before release");
                            assert!(history(this));
                        }).unwrap();
                    }
                    assert!(KEYDOWNS.load(Ordering::SeqCst) >= downs_before + 2, "private X server must deliver actual held-repeat keydowns");
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| {
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), before + 1, "exactly one actual callback activation per gesture");
                        eprintln!("current release {key}: exact callback count {} -> {}", before, before+1);
                        assert!(this.sidebar.unread_only(), "completed gesture must activate Unread once");
                        this.sidebar.set_unread_only(false);
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("release-{key}"));
                        cx.notify();
                    }).unwrap();
                    executor.timer(std::time::Duration::from_millis(120)).await;
                    let stale_before = ACTIVATIONS[0].load(Ordering::SeqCst);
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
                            assert!(!PAINTED.lock().unwrap().contains_key("unread"), "no intermediate sidebar paint allowed");
                            assert!(this.pending_sidebar_key.is_some(), "retain retired press until release");
                            eprintln!("real Configure away/back with paint suspended: {:?}", window.viewport_size());
                            navigation_resize_test::FROZEN.store(false, Ordering::SeqCst);
                            this.unread_focus.focus(window);
                            cx.notify();
                        }).unwrap();
                        executor.timer(std::time::Duration::from_millis(200)).await;
                        assert!(PAINTED.lock().unwrap().contains_key("unread"), "reacquire actual fresh callback paint before releasing held key");
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
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), stale_before, "release after source retirement must not activate a replacement");
                        assert!(!this.sidebar.unread_only());
                        assert!(history(this));
                    }).unwrap();
                    native_key(key, true);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| { this.search_focus.focus(window); cx.notify(); }).unwrap();
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| { this.unread_focus.focus(window); cx.notify(); }).unwrap();
                    executor.timer(std::time::Duration::from_millis(900)).await;
                    native_key(key, false);
                    executor.timer(std::time::Duration::from_millis(100)).await;
                    this.update_in(cx, |this, window, cx| {
                        assert_eq!(ACTIVATIONS[0].load(Ordering::SeqCst), stale_before, "release after focus away-back must not activate");
                        assert!(!this.sidebar.unread_only());
                        assert!(history(this));
                        preserve_drafts_and_capture(this, &scopes, window, cx, &format!("focus-retired-{key}"));
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
            let count = || ACTIVATIONS[1].load(Ordering::SeqCst);
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
        if DYNAMIC.load(Ordering::SeqCst) == 28 {
            // Debug mode: event ids under messages and a room/sync line; off shows none.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                assert!(ok, "screenshot {name}");
            };
            wait(700).await;
            this.update_in(cx, |this, _, cx| {
                let events = (1..=3).map(|n| matrix_engine::history::HistoryEvent {
                    redacted: false,
                    event_id: matrix_domain::EventId::new(format!("$debugevent{n}:gnobbi.test")),
                    body: format!("Messaggio {n}"),
                    outgoing: n == 2,
                    timestamp_ms: 1_700_000_000_000 + n * 60_000,
                    sender: matrix_engine::history::HistorySender::new("@anna:gnobbi.test", None),
                    video: None,
                    styles: Vec::new(),
                    thread: None,
                    thread_root: None,
                    status: false,
                    attachment_name: None,
                    attachment_size: None,
                }).collect();
                // The harness timeline is an offline fixture; debug ids only show on live rows.
                let timeline = crate::timeline_view::TimelineView::connected(this.active_scope.clone(), cx.focus_handle());
                timeline.append_server(events);
                this.timeline = Some(timeline);
                this.prefs.debug = false;
                cx.notify();
            }).unwrap();
            wait(500).await;
            shot("p185-off");
            this.update_in(cx, |this, _, cx| { this.prefs.debug = true; cx.notify(); }).unwrap();
            wait(500).await;
            shot("p185-on");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 30 {
            // List header: space icon and name on the first row, search full width below; «Tutte» and a space.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                let _ = ok; // grim exists only on Wayland; the X11 run has no screenshots
            };
            let layout = |label: &str| {
                let p = PAINTED.lock().unwrap();
                let (icon, name, search, list) = (p["list-avatar"], p["list-name"], p["search"], p["list"]);
                assert!(icon.right() <= name.left(), "{label}: icon {icon:?} left of name {name:?}");
                assert!(search.top() >= icon.bottom() && search.top() >= name.bottom(), "{label}: search {search:?} below the row");
                assert!(search.left() <= icon.left() + px(16.) && search.right() + px(1.) >= name.right(), "{label}: search {search:?} spans the row");
                assert!(search.bottom() <= list.top(), "{label}: list below search");
            };
            wait(500).await;
            assert_eq!(this.update_in(cx, |this, _, _| this.sidebar.list_header().0).unwrap(), "Tutte");
            layout("Tutte");
            shot("p289-tutte");
            this.update_in(cx, |this, _, cx| {
                let scope = this.sidebar.spaces().next().unwrap().scope.clone();
                assert!(this.sidebar.set_space(Some(scope)));
                cx.notify();
            }).unwrap();
            wait(500).await;
            let (name, initials, _) = this.update_in(cx, |this, _, _| this.sidebar.list_header()).unwrap();
            assert_eq!((name.as_str(), initials.as_str()), ("Alpha", "Al"));
            layout("Alpha");
            shot("p289-alpha");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 29 {
            // Devices and Notifications live in the drawer: Down/Enter open them, the list footer no longer has them.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                assert!(ok, "screenshot {name}");
            };
            macro_rules! key { ($k:expr) => { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap() }; }
            wait(400).await;
            this.update_in(cx, |this, window, _| {
                use matrix_desktop::panel_openers::Control;
                assert!(this.opener_ui.source(Control::Devices).is_none() && this.opener_ui.source(Control::Notifications).is_none(), "footer no longer offers Devices/Notifications");
                this.menu_focus.focus(window);
            }).unwrap();
            key!("enter");
            wait(400).await;
            shot("p186-drawer");
            key!("down");
            key!("enter");
            wait(300).await;
            this.update_in(cx, |this, window, _| {
                assert!(this.verification_open && !this.menu_open && !this.settings_open, "Down+Enter opens Devices and closes the drawer");
                assert!(this.list_focus.is_focused(window), "focus lands on the panel root");
                this.set_verification_open(false);
                this.menu_focus.focus(window);
            }).unwrap();
            key!("enter");
            wait(400).await;
            key!("down");
            key!("down");
            wait(300).await;
            shot("p186-notifications-row");
            key!("enter");
            wait(300).await;
            this.update_in(cx, |this, _, _| {
                assert!(this.notifications_open && !this.menu_open, "two Downs + Enter open Notifications");
                this.set_notifications_open(false);
            }).unwrap();
            shot("p186-notifications");
            // Up wraps back; Esc still closes the drawer.
            this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
            wait(200).await;
            key!("ctrl-shift-m");
            wait(300).await;
            this.update_in(cx, |this, window, _| assert!(this.menu_open && this.drawer_focus.is_focused(window), "shortcut reopens the drawer")).unwrap();
            key!("up");
            this.update_in(cx, |this, _, _| assert_eq!(this.drawer_row, 6, "Up from the first row wraps to the last")).unwrap();
            for _ in 0..4 {
                key!("up");
            }
            key!("enter");
            this.update_in(cx, |this, window, cx| {
                assert!(this.notifications_open, "Up five times from the first row reaches Notifications");
                this.set_notifications_open(false);
                this.set_menu_open(true, window, cx);
            }).unwrap();
            key!("escape");
            this.update_in(cx, |this, window, cx| {
                assert!(!this.menu_open && this.menu_focus.is_focused(window), "Esc closes the drawer");
                this.switch_error = Some((matrix_domain::AccountId::new(1), "Riprova tra poco"));
                this.set_menu_open(true, window, cx);
                assert!(this.switch_error.is_none(), "opening the drawer clears a stale row error");
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 27 {
            // Drawer entry opens Settings; Enter toggles the debug switch and persists it; Esc steps back out.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                assert!(ok, "screenshot {name}");
            };
            macro_rules! key { ($k:expr) => { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap() }; }
            let root = std::env::temp_dir().join(format!("p184-{}", std::process::id()));
            let dirs = matrix_desktop::AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
            let saved = matrix_desktop::Preferences::load;
            this.update_in(cx, |this, window, _| {
                this.prefs_dirs = Some(dirs.clone());
                assert!(!this.prefs.debug, "debug mode defaults to off");
                this.menu_focus.focus(window);
            }).unwrap();
            key!("enter");
            key!("enter");
            wait(500).await;
            this.update_in(cx, |this, window, _| {
                assert!(this.settings_open && this.settings_focus.is_focused(window), "Enter on the drawer entry opens Settings");
            }).unwrap();
            shot("p184-off");
            key!("enter");
            wait(300).await;
            this.update_in(cx, |this, _, _| {
                assert!(this.prefs.debug, "Enter toggles debug on");
                assert!(saved(&dirs).debug, "the switch is saved to the preferences file");
            }).unwrap();
            shot("p184-on");
            key!("space");
            this.update_in(cx, |this, _, _| {
                assert!(!this.prefs.debug && !saved(&dirs).debug, "Space toggles it off and saves");
            }).unwrap();
            this.update_in(cx, |this, _, _| assert!(this.prefs.hide_redacted, "deleted messages are hidden by default")).unwrap();
            // Ring of three: debug, read receipts, hide deleted. Each key press must change exactly one switch.
            key!("tab");
            key!("space");
            this.update_in(cx, |this, _, _| {
                assert!(!this.prefs.read_receipts && !saved(&dirs).read_receipts, "Tab then Space turns read receipts off and saves");
                assert!(!this.prefs.debug && this.prefs.hide_redacted, "and nothing else");
            }).unwrap();
            key!("tab");
            key!("space");
            this.update_in(cx, |this, _, _| {
                assert!(!this.prefs.hide_redacted && !saved(&dirs).hide_redacted && !this.prefs.debug && !this.prefs.read_receipts, "second Tab then Space shows deleted messages and saves");
            }).unwrap();
            key!("enter");
            this.update_in(cx, |this, _, _| assert!(this.prefs.hide_redacted && saved(&dirs).hide_redacted, "Enter toggles it back")).unwrap();
            key!("shift-tab");
            key!("space");
            this.update_in(cx, |this, _, _| assert!(this.prefs.read_receipts && saved(&dirs).read_receipts && !this.prefs.debug, "Shift+Tab walks back to read receipts")).unwrap();
            key!("up");
            this.update_in(cx, |this, _, _| assert_eq!(this.settings_row, 0, "Up from read receipts reaches debug")).unwrap();
            key!("tab");
            key!("tab");
            key!("tab");
            this.update_in(cx, |this, _, _| assert_eq!(this.settings_row, 0, "the ring wraps after three stops")).unwrap();
            wait(300).await;
            shot("p242-receipts-off");
            key!("escape");
            this.update_in(cx, |this, window, _| {
                assert!(!this.settings_open && this.menu_open && this.drawer_focus.is_focused(window), "Esc returns to the drawer");
            }).unwrap();
            key!("escape");
            this.update_in(cx, |this, window, cx| {
                assert!(!this.menu_open && this.menu_focus.is_focused(window), "second Esc closes the drawer");
                let _ = std::fs::remove_dir_all(dirs.config.parent().unwrap());
                cx.quit();
            }).unwrap();
            return;
        }
        if matches!(DYNAMIC.load(Ordering::SeqCst), 32 | 33) {
            // Tab ring from the menu button; compact (33) has the button in the list header, wide (32) on the rail.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            macro_rules! key { ($k:expr) => { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap() }; }
            let names = |this: &mut Shell, window: &mut Window| -> &'static str {
                [("menu", &this.menu_focus), ("rail", &this.rail_ui.focus), ("unread", &this.unread_focus), ("search", &this.search_focus), ("list", &this.list_focus), ("spaces", &this.spaces_ui.focus)]
                    .into_iter().find(|(_, h)| h.is_focused(window)).map_or("other", |(n, _)| n)
            };
            this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
            wait(300).await;
            let mut forward = vec![];
            for _ in 0..6 {
                forward.push(this.update_in(cx, |this, window, _| names(this, window)).unwrap());
                key!("tab");
                wait(100).await;
            }
            eprintln!("TAB-ORDER forward {forward:?}");
            let wide = DYNAMIC.load(Ordering::SeqCst) == 32;
            // menu -> rail entries (+ their edit button) -> list header -> search -> list; compact has no rail but the Spaces strip after the list.
            let want: &[&str] = if wide { &["menu", "rail", "rail", "rail", "other", "unread"] } else { &["menu", "unread", "search", "list", "spaces", "menu"] };
            assert_eq!(forward, want);
            let mut back = vec![];
            this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
            for _ in 0..6 {
                back.push(this.update_in(cx, |this, window, _| names(this, window)).unwrap());
                key!("shift-tab");
                wait(100).await;
            }
            eprintln!("TAB-ORDER backward {back:?}");
            if wide {
                // Shift+Tab on the first rail entry leaves the rail for the menu button.
                this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
                key!("tab");
                key!("shift-tab");
                assert_eq!(this.update_in(cx, |this, window, _| names(this, window)).unwrap(), "menu");
            } else {
                assert_eq!(back, ["menu", "spaces", "list", "search", "unread", "menu"]);
            }
            // The button stays reachable (inside the window) and closing the drawer gives focus back to it.
            let menu = { PAINTED.lock().unwrap()["menu-button"] };
            let size = this.update_in(cx, |_, window, _| window.viewport_size()).unwrap();
            assert!(menu.left() >= px(0.) && menu.top() >= px(0.) && menu.right() <= size.width && menu.bottom() <= size.height, "menu {menu:?} inside {size:?}");
            this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
            for close in ["escape", "ctrl-shift-m"] {
                key!("enter");
                wait(200).await;
                this.update_in(cx, |this, window, _| assert!(this.menu_open && this.drawer_focus.is_focused(window))).unwrap();
                key!(close);
                wait(200).await;
                this.update_in(cx, |this, window, _| assert!(!this.menu_open && this.menu_focus.is_focused(window), "{close} returns focus to the menu button")).unwrap();
            }
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 26 {
            // Connected Shell: the menu button opens a drawer; Esc, the shortcut or the scrim close it; focus follows.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                let _ = ok; // grim exists only on Wayland; the X11 run has no screenshots
            };
            macro_rules! key { ($k:expr) => { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap() }; }
            this.update_in(cx, |this, window, _| {
                assert!(!this.menu_open, "drawer starts closed");
                this.menu_focus.focus(window);
            }).unwrap();
            wait(500).await;
            shot("p183-closed");
            // The button sits at the top of the rail, above «Tutte», and the list header no longer holds it.
            let (menu, first, avatar) = { let p = PAINTED.lock().unwrap(); (p["menu-button"], p.get("rail-None").copied(), p["list-avatar"]) };
            assert!(menu.right() <= px(72.) && first.is_none_or(|first| menu.bottom() <= first.top()), "menu {menu:?} above the first rail entry {first:?}");
            assert!(menu.top() >= px(0.) && avatar.left() >= px(72.), "list header icon {avatar:?} is right of the rail");
            // Real pointer clicks need an input device: xdotool on the private X11 display.
            let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout).unwrap().lines().next().expect("window").to_string();
            macro_rules! click { ($x:expr, $y:expr) => { assert!(std::process::Command::new("xdotool").args(["mousemove", "--window", &win, &(($x) as i32).to_string(), &(($y) as i32).to_string(), "click", "1"]).status().unwrap().success()) }; }
            let (bx, by) = (f32::from(menu.center().x), f32::from(menu.center().y));
            click!(bx, by);
            wait(500).await;
            this.update_in(cx, |this, window, _| assert!(this.menu_open && this.drawer_focus.is_focused(window), "click opens the drawer and moves focus")).unwrap();
            // Layering: the open drawer spans the full height from the left edge and covers the rail, which gets no click.
            let drawer = { PAINTED.lock().unwrap()["menu-drawer"] };
            // linux-a11y paints the rail through semantic observers: no «rail-None» bounds to click over, so only the layering check is skipped there.
            if let Some(first) = first {
                assert!(drawer.left() <= px(0.) && drawer.top() <= px(0.) && drawer.right() >= first.right() && drawer.bottom() >= first.bottom(), "drawer {drawer:?} covers menu {menu:?} and rail {first:?}");
                let selected = this.update_in(cx, |this, _, _| this.active_scope.clone()).unwrap();
                let ups = ACTIVATIONS[1].load(Ordering::SeqCst);
                // Just under the drawer's last row, still over the bottom edge of «Tutte».
                click!(f32::from(first.center().x), f32::from(first.bottom()) - 3.);
                wait(500).await;
                // The point may land on a drawer row (which closes it); either way the rail entry underneath stays untouched.
                this.update_in(cx, |this, _, _| assert!(this.active_scope == selected && ACTIVATIONS[1].load(Ordering::SeqCst) == ups, "the rail entry under the drawer received the click")).unwrap();
            }
            // The point sits on a drawer row (Notifiche), which closes the drawer and opens its panel: start the outside click from a plain open drawer.
            this.update_in(cx, |this, window, cx| { this.set_notifications_open(false); this.set_settings_open(false, window, cx); this.set_menu_open(false, window, cx); this.set_menu_open(true, window, cx); }).unwrap();
            wait(300).await;
            click!(600., 400.);
            wait(500).await;
            this.update_in(cx, |this, window, _| assert!(!this.menu_open && this.menu_focus.is_focused(window), "click outside closes, focus returns to the button: open={} menu_focused={} settings={}", this.menu_open, this.menu_focus.is_focused(window), this.settings_open)).unwrap();
            this.update_in(cx, |this, window, _| this.menu_focus.focus(window)).unwrap();
            key!("enter");
            wait(500).await;
            this.update_in(cx, |this, window, _| {
                assert!(this.menu_open, "Enter on the menu button opens the drawer");
                assert!(this.drawer_focus.is_focused(window), "focus moves into the drawer");
            }).unwrap();
            key!("tab");
            this.update_in(cx, |this, window, _| assert!(this.drawer_focus.is_focused(window), "Tab stays inside the drawer")).unwrap();
            key!("shift-tab");
            this.update_in(cx, |this, window, _| assert!(this.drawer_focus.is_focused(window), "Shift+Tab stays inside the drawer")).unwrap();
            wait(500).await;
            shot("p183-open");
            key!("escape");
            this.update_in(cx, |this, window, _| {
                assert!(!this.menu_open, "Esc closes the drawer");
                assert!(this.menu_focus.is_focused(window), "focus returns to the menu button");
            }).unwrap();
            key!("ctrl-shift-m");
            this.update_in(cx, |this, window, _| assert!(this.menu_open && this.drawer_focus.is_focused(window))).unwrap();
            key!("ctrl-shift-m");
            this.update_in(cx, |this, window, _| assert!(!this.menu_open && this.menu_focus.is_focused(window), "second toggle closes")).unwrap();
            // Outside click and button click share set_menu_open; the scrim path is checked through it.
            this.update_in(cx, |this, window, cx| {
                this.set_menu_open(true, window, cx);
                this.set_menu_open(false, window, cx);
                assert!(!this.menu_open && this.menu_focus.is_focused(window));
                cx.quit();
            }).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 25 {
            // Connected Shell: an empty room is hidden, a «Vuote (N)» control brings it back.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status().map(|s| s.success()).unwrap_or(false);
                assert!(ok, "screenshot {name}");
            };
            let visible = |this: &mut Shell| this.sidebar.visible_indices().iter().map(|&i| this.sidebar.rooms()[i].name.clone()).collect::<Vec<_>>();
            let hidden = this.update_in(cx, |this, _, cx| {
                let account = this.active_scope.account_id;
                for (id, name, members, preview) in [("!a:t", "Anna Bianchi", 2, Some("Ci vediamo domani")), ("!b:t", "Stanza vuota", 1, None)] {
                    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                        scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id) },
                        name: name.into(), unread: 0,
                        info: matrix_engine::RoomInfo { preview: preview.map(Into::into), joined_members: members, ..Default::default() },
                    }));
                }
                cx.notify();
                visible(this)
            }).unwrap();
            assert!(!hidden.iter().any(|n| n == "Stanza vuota") && hidden.iter().any(|n| n == "Anna Bianchi"), "{hidden:?}");
            wait(700).await;
            shot("p182-hidden");
            let shown = this.update_in(cx, |this, _, cx| {
                this.sidebar.set_show_empty(true);
                cx.notify();
                visible(this)
            }).unwrap();
            assert!(shown.iter().any(|n| n == "Stanza vuota"), "{shown:?}");
            wait(700).await;
            shot("p182-shown");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 24 {
            // Connected Shell: a room row shows the last real message under the name, none when it has no message.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let rows = this.update_in(cx, |this, _, cx| {
                let account = this.active_scope.account_id;
                for (id, name, preview) in [("!a:t", "Anna Bianchi", Some("Ci vediamo domani alle nove")), ("!b:t", "Anna Rossi", None)] {
                    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                        scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id) },
                        name: name.into(), unread: 0,
                        info: matrix_engine::RoomInfo { preview: preview.map(Into::into), ..Default::default() },
                    }));
                }
                this.sidebar.set_query("Anna");
                cx.notify();
                this.sidebar.rooms().iter().map(|r| (r.name.clone(), r.preview.clone())).collect::<Vec<_>>()
            }).unwrap();
            assert!(rows.contains(&("Anna Bianchi".into(), "Ci vediamo domani alle nove".into())), "{rows:?}");
            assert!(rows.contains(&("Anna Rossi".into(), String::new())), "{rows:?}");
            wait(700).await;
            let ok = std::process::Command::new("grim").arg("/tmp/gui-out/p181-preview.png").status().map(|s| s.success()).unwrap_or(false);
            assert!(ok, "screenshot");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 40 {
            // Room sheet with N members: draw time, a11y nodes, and the wheel over the sheet scrolls only the member list.
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let path = format!("/tmp/gui-out/{name}.png");
                let wl = std::env::var_os("WAYLAND_DISPLAY").is_some();
                let ok = if wl { std::process::Command::new("grim").arg(&path).status() } else { std::process::Command::new("import").args(["-window", "root", &path]).status() };
                assert!(ok.unwrap().success(), "shot {name}");
            };
            let members = |n: usize| (0..n).map(|i| matrix_engine::operations::Member {
                user_id: format!("@utente{i:04}:fixture.test"), name: format!("Utente di prova {i:04}"), avatar: None,
            }).collect::<Vec<_>>();
            let mut report = serde_json::Map::new();
            for (label, n) in [("closed", None), ("open-0", Some(0)), ("open-1000", Some(1000)), ("open-343", Some(343))] {
                this.update_in(cx, |this, _, cx| {
                    match n { Some(n) => { this.room_members = members(n); this.set_room_details_open(true); } None => this.set_room_details_open(false) }
                    cx.notify();
                }).unwrap();
                wait(300).await;
                let mut v: Vec<f64> = (0..40).map(|_| cx.update(|window, app| {
                    window.refresh();
                    let t = std::time::Instant::now(); let _ = window.draw(app); t.elapsed().as_secs_f64() * 1000.
                }).unwrap()).collect();
                v.drain(..5); v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let (med, p95, max) = (v[v.len() / 2], v[(v.len() as f64 * 0.95) as usize - 1], *v.last().unwrap());
                println!("AFTER draw {label}: median {med:.2} ms p95 {p95:.2} ms max {max:.2} ms");
                report.insert(format!("draw_{label}"), serde_json::json!({"median_ms": med, "p95_ms": p95, "max_ms": max, "samples": v.len()}));
            }
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                let nodes = this.update_in(cx, |this, _, _| {
                    let room = this.sidebar.selected().unwrap();
                    matrix_desktop::accessibility::details::DetailsSnapshot {
                        scope: room.scope.clone(), name: room.name.clone(), info: room.info.clone(),
                        people: this.people_labels(), back: false, focus: Default::default(), bounds: [None; 2],
                    }.tree().nodes.len()
                }).unwrap();
                println!("AFTER a11y nodes in sheet tree: {nodes}");
                report.insert("a11y_sheet_nodes_343".into(), nodes.into());
            }
            // Wheel over the sheet's member list: does the timeline behind it move too?
            this.update_in(cx, |this, _, cx| { this.timeline.as_ref().unwrap().scroll_to_row(20, px(7.)); cx.notify(); }).unwrap();
            wait(400).await;
            shot("sheet-343-top");
            let size = this.update_in(cx, |_, window, _| window.viewport_size()).unwrap();
            let start = this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().top_row()).unwrap();
            let (x, y) = ((f32::from(size.width) - 150.) as i32, (f32::from(size.height) * 0.7) as i32);
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                // Wayland: virtual pointer helper (tools/wayland/pointer.c) built by the runner; output 1920x1080 scale 1.
                use std::io::{BufRead, Write};
                let mut ptr = std::process::Command::new("/tmp/gui-out/pointer").env("DIME_ISOLATED_WAYLAND", "1")
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
                let mut input = ptr.stdin.take().unwrap();
                let mut output = std::io::BufReader::new(ptr.stdout.take().unwrap());
                wait(1500).await; // let GPUI bind wl_pointer for the new seat capability
                for action in ["move 0", "move 0", "scroll 1", "scroll 1", "scroll 1", "scroll 1", "scroll 1"] {
                    writeln!(input, "{} {} 1920 1080 {action}", 1920 - 150, (1080. * 0.7) as i32).unwrap();
                    let mut line = String::new(); output.read_line(&mut line).unwrap();
                    assert_eq!(line, "done\n");
                    wait(150).await;
                }
                drop(input); let _ = ptr.wait();
            } else {
                let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout).unwrap().lines().next().expect("window").to_string();
                assert!(std::process::Command::new("xdotool").args(["mousemove", "--window", &win, &x.to_string(), &y.to_string(), "click", "--repeat", "5", "--delay", "40", "5"]).status().unwrap().success());
            }
            wait(500).await;
            let end = this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().top_row()).unwrap();
            shot("sheet-343-after-wheel");
            println!("AFTER wheel over sheet at ({x},{y}) of {size:?}: timeline top {start:?} -> {end:?}");
            let listed = this.update_in(cx, |this, _, _| this.members_scroll.0.borrow().base_handle.offset().y).unwrap();
            assert!(listed < px(0.), "the member list did not scroll: {listed:?}");
            assert_eq!(start, end, "the timeline moved under a wheel over the sheet");
            report.insert("wheel_point".into(), serde_json::json!([x, y, f32::from(size.width), f32::from(size.height)]));
            report.insert("timeline_top_before".into(), format!("{start:?}").into());
            report.insert("timeline_top_after".into(), format!("{end:?}").into());
            report.insert("timeline_moved_by_wheel_over_sheet".into(), (start != end).into());
            std::fs::write("/tmp/gui-out/sheet-report.json", serde_json::to_string_pretty(&report).unwrap()).unwrap();
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        if DYNAMIC.load(Ordering::SeqCst) == 41 {
            // Exploratory matrix for the room sheet vs timeline wheel (Wayland only: needs the virtual pointer helper).
            use std::io::{BufRead, Write};
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            let shot = |name: &str| {
                let ok = std::process::Command::new("grim").arg(format!("/tmp/gui-out/{name}.png")).status();
                assert!(ok.unwrap().success(), "shot {name}");
            };
            let members = |n: usize| (0..n).map(|i| matrix_engine::operations::Member {
                user_id: format!("@utente{i:04}:fixture.test"), name: format!("Utente di prova {i:04}"), avatar: None,
            }).collect::<Vec<_>>();
            let mut ptr = std::process::Command::new("/tmp/gui-out/pointer").env("DIME_ISOLATED_WAYLAND", "1")
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            let mut input = ptr.stdin.take().unwrap();
            let mut output = std::io::BufReader::new(ptr.stdout.take().unwrap());
            let mut send = |x: i32, y: i32, a: &str| {
                writeln!(input, "{x} {y} 1920 1080 {a}").unwrap();
                let mut line = String::new(); output.read_line(&mut line).unwrap();
                assert_eq!(line, "done\n", "pointer helper died on: {x} {y} {a}");
            };
            let mut report = serde_json::Map::new();
            let mut fails: Vec<String> = vec![];
            macro_rules! check { ($ok:expr, $($m:tt)*) => {{
                let ok = $ok; let msg = format!($($m)*);
                println!("{} {msg}", if ok { "PASS" } else { "FAIL" });
                report.insert(msg.clone(), ok.into());
                if !ok { fails.push(msg); }
            }}; }
            macro_rules! state { () => { this.update_in(cx, |this, _, _| {
                let h = this.members_scroll.0.borrow().base_handle.clone();
                (f32::from(h.offset().y), f32::from(h.max_offset().height), this.timeline.as_ref().unwrap().top_row(),
                 f32::from(this.scroll.0.borrow().base_handle.offset().y))
            }).unwrap() }; }
            macro_rules! open { ($n:expr) => {{
                this.update_in(cx, |this, _, cx| {
                    match $n { Some(n) => { this.room_members = members(n); this.set_room_details_open(true); } None => this.set_room_details_open(false) }
                    this.members_scroll.0.borrow().base_handle.set_offset(gpui::point(px(0.), px(0.)));
                    this.timeline.as_ref().unwrap().scroll_to_row(20, px(7.));
                    cx.notify();
                }).unwrap();
                wait(400).await;
            }}; }
            // Bursts of wheel ticks; trackpad-with-inertia is approximated by decaying deltas sent back to back.
            macro_rules! wheel { ($x:expr, $y:expr, $n:expr, $d:expr) => {{ for _ in 0..$n { send($x, $y, &format!("scroll {}", $d)); } wait(300).await; }}; }
            wait(1500).await; // GPUI binds wl_pointer for the new seat
            send(10, 10, "move 0");
            let size = this.update_in(cx, |_, window, _| window.viewport_size()).unwrap();
            let (sx, sy) = (1920 - 150, (f32::from(size.height) * 0.7) as i32); // over the sheet
            let (tx, ty) = (700, sy); // over the timeline (sheet starts right of x=1400)

            open!(Some(343));
            let (m0, max, t0, _) = state!();
            check!(max > 0., "343 members: list has scroll extent (max {max:.0}px)");
            // S1 wheel down on the sheet
            wheel!(sx, sy, 5, 1);
            let (m1, _, t1, _) = state!();
            check!(m1 < m0 && t1 == t0, "S1 wheel down over sheet: list {m0:.0}->{m1:.0}, timeline unchanged");
            shot("s1-scrolled");
            // S2 to the bottom, then 40 more ticks past the end
            wheel!(sx, sy, 100, 5);
            let (m2, _, t2, _) = state!();
            check!((m2 + max).abs() < 1., "S2 bottom reached: offset {m2:.0} == -{max:.0}");
            wheel!(sx, sy, 40, 3);
            let (m2b, _, t2b, _) = state!();
            check!(m2b == m2 && t2b == t2 && t2 == t0, "S2 40 ticks past the bottom: list stays, timeline unchanged");
            shot("s2-bottom");
            // S3 back to top, then 40 more up
            wheel!(sx, sy, 100, -5);
            let (m3, _, t3, _) = state!();
            check!(m3 == 0., "S3 top reached: offset {m3:.0}");
            wheel!(sx, sy, 40, -3);
            let (m3b, _, t3b, _) = state!();
            check!(m3b == 0. && t3b == t0 && t3 == t0, "S3 40 ticks past the top: list stays, timeline unchanged");
            // S4 inertia-like burst: 30 decaying ticks without waits, from the middle
            wheel!(sx, sy, 3, 4);
            for d in (1..=30).rev() { send(sx, sy, &format!("scroll {}", (d / 3).max(1))); }
            wait(300).await;
            let (m4, _, t4, _) = state!();
            check!(m4 < 0. && t4 == t0, "S4 inertia-like burst: list {m4:.0}, timeline unchanged");
            // S5 pointer on the timeline: only the timeline moves
            let (ma, _, ta, _) = state!();
            wheel!(tx, ty, 5, -3);
            let (mb, _, tb, _) = state!();
            check!(mb == ma && tb != ta, "S5 wheel over timeline: timeline moved, list unchanged ({ma:.0})");
            this.update_in(cx, |this, _, _| this.timeline.as_ref().unwrap().scroll_to_row(20, px(7.))).unwrap();
            // S6 scrollbar thumb drag
            let pre = state!();
            wheel!(sx, sy, 3, 2); // makes the thumb visible/active
            println!("INFO S6 list offset before/after 3 ticks: {:.0} -> {:.0} (max {:.0})", pre.0, state!().0, pre.1);
            shot("s6-thumb-visible");
            let before = state!();
            let thumb = this.update_in(cx, |this, _, _| this.members_scrollbar.thumb_centre()).unwrap();
            check!(thumb.is_some(), "S6 members scrollbar has painted geometry");
            if let Some(c) = thumb {
                let (cxp, cyp) = (f32::from(c.x) as i32, f32::from(c.y) as i32);
                println!("INFO S6 thumb centre ({cxp},{cyp}) window {size:?}");
                send(cxp, cyp, "move 0"); wait(150).await;
                send(cxp, cyp, "press 0"); wait(150).await;
                for step in 1..=8 { send(cxp, cyp - step * 25, "move 0"); wait(60).await; }
                shot("s6-dragging");
                send(cxp, cyp - 200, "release 0"); wait(300).await;
                let after = state!();
                check!(after.0 > before.0 && after.2 == before.2, "S6 thumb drag (upwards): list {:.0}->{:.0}, timeline unchanged", before.0, after.0);
                send(10, 10, "move 0"); wait(1500).await;
                shot("s6-after-release-faded");
            }
            // S7 few members and empty room
            for n in [5usize, 1, 0] {
                open!(Some(n));
                let (m, mx, t, _) = state!();
                wheel!(sx, sy, 10, 5);
                wheel!(sx, sy, 10, -5);
                let (m_, _, t_, _) = state!();
                check!(m_ == m && t_ == t, "S7 {n} members (extent {mx:.0}): wheel over sheet moves neither list nor timeline");
                shot(&format!("s7-{n}-members"));
            }
            // S8 close the panel mid-scroll: the same wheel point now reaches the timeline; reopen resets nothing odd
            open!(Some(343));
            wheel!(sx, sy, 5, 3);
            let (mo, _, to, _) = state!();
            open!(None);
            let (_, _, tc, _) = state!();
            wheel!(sx, sy, 5, -3);
            let (_, _, tc2, _) = state!();
            check!(tc == to && tc2 != tc, "S8 panel closed: wheel at same point scrolls timeline ({:?}->{:?})", tc.1, tc2.1);
            this.update_in(cx, |this, _, cx| { this.set_room_details_open(true); cx.notify(); }).unwrap();
            wait(400).await;
            let (mr, _, _, _) = state!();
            println!("INFO S8 reopen: list offset before close {mo:.0}, after reopen {mr:.0}");
            report.insert("s8_offset_before_close_after_reopen".into(), serde_json::json!([mo, mr]));
            let (r0, _, rt0, _) = state!();
            wheel!(sx, sy, 5, 3);
            let (r1, _, rt1, _) = state!();
            check!(r1 < r0 && rt1 == rt0, "S8 reopened panel scrolls its list again, timeline unchanged");
            // S9 room list regression: wheel on the left list never moves the sheet list or the timeline
            let (l0, _, lt0, rl0) = state!();
            wheel!(150, sy, 5, 3);
            let (l1, _, lt1, rl1) = state!();
            println!("INFO S9 room list offset {rl0:.0}->{rl1:.0}");
            check!(l1 == l0 && lt1 == lt0, "S9 wheel over room list leaves sheet list and timeline alone");
            // S10 small windows: sheet taller than the window. The pointer helper works in output coordinates, so read where sway put the window.
            let origin = || -> (i32, i32) {
                fn find(v: &serde_json::Value) -> Option<(i64, i64)> {
                    if v["focused"] == true && v["rect"].is_object() { return Some((v["rect"]["x"].as_i64()?, v["rect"]["y"].as_i64()?)); }
                    v["nodes"].as_array().into_iter().chain(v["floating_nodes"].as_array()).flatten().find_map(find)
                }
                let out = std::process::Command::new("swaymsg").args(["-t", "get_tree"]).output().unwrap();
                let tree: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                let (x, y) = find(&tree).expect("focused window in sway tree");
                (x as i32, y as i32)
            };
            for (h_req, label) in [(700, "700"), (420, "420")] {
                let cmd = format!("floating enable, resize set width 1000 height {h_req}, move position 0 0");
                let st = std::process::Command::new("swaymsg").arg(cmd).status();
                wait(1000).await;
                let small = this.update_in(cx, |_, window, _| window.viewport_size()).unwrap();
                let (ox, oy) = origin();
                println!("INFO S10-{label} swaymsg {st:?} viewport {small:?} origin ({ox},{oy})");
                let (w, h) = (f32::from(small.width) as i32, f32::from(small.height) as i32);
                open!(Some(343));
                shot(&format!("s10-{label}-top"));
                let (a0, amax, ta0, _) = state!();
                wheel!(ox + w - 150, oy + 60, 6, 3); // over the info block
                shot(&format!("s10-{label}-after-wheel-on-info"));
                let (a1, _, ta1, _) = state!();
                wheel!(ox + w - 150, oy + h - 25, 6, 3); // over the member list (bottom edge of the window)
                let (a2, _, ta2, _) = state!();
                shot(&format!("s10-{label}-after-wheel-on-list"));
                println!("INFO S10-{label} list offset {a0:.0} (extent {amax:.0}) -> info-wheel {a1:.0} -> list-wheel {a2:.0}");
                check!(ta1 == ta0 && ta2 == ta0, "S10-{label} window {w}x{h}: timeline unchanged by wheel over the sheet");
                check!(a1 == a0, "S10-{label}: wheel over the info block does not move the member list");
                check!(amax > 0. && a2 < a0, "S10-{label}: member list scrolls under the pointer at the bottom edge (extent {amax:.0})");
            }
            // S11 resize larger while scrolled
            let _ = std::process::Command::new("swaymsg").arg("resize set width 1900 height 1000").status();
            wait(800).await;
            let (b, bmax, _, _) = state!();
            check!(b <= 0. && b >= -bmax - 1., "S11 resize larger while scrolled: offset {b:.0} within extent {bmax:.0}");
            shot("s11-resized-back");
            std::fs::write("/tmp/gui-out/matrix-report.json", serde_json::to_string_pretty(&report).unwrap()).unwrap();
            drop(send); drop(input); let _ = ptr.wait();
            assert!(fails.is_empty(), "failed: {fails:#?}");
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
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
            this.update_in(cx, |this, _window, cx| {
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
                assert!(this.unread_focus.is_focused(window));
                this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary { scope: beta, name: "Beta replacement".into(), unread: 0, info: matrix_engine::RoomInfo { is_space: true, ..Default::default() } }));
                cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(250)).await;
            this.update_in(cx, |this, window, cx| {
                assert!(this.unread_focus.is_focused(window), "replacement eligibility must not steal focus back");
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
                for (query, unread, group, count) in [("",false,false,300), ("",true,false,1), ("",true,true,1), ("no matches ",true,true,0), ("",false,true,1), ("",false,false,300)] {
                    this.update_in(cx, |this, _, cx| {
                        if this.theme.is_dark() != dark { this.toggle_theme(cx); }
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
                assert!(this.unread_focus.is_focused(window), "native Configure must move hidden compact focus to the visible unread filter before repaint");
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
                for label in ["unread", "search", "list"] {
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
                assert!(this.unread_focus.is_focused(window), "last joined Space removal must reach the visible unread filter, not hidden menu");
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert_eq!(this.sidebar.active_space(), None);
                assert!(history(this));
                eprintln!("compact: removed preceding choice retains Beta; last Space removal focuses the unread filter without activation");
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
            vec![("shift-tab", "space3"), ("shift-tab", "space2"), ("shift-tab", "space1"), ("shift-tab", "space0"), ("shift-tab", "list"), ("tab", "space0"), ("tab", "space1"), ("tab", "space2"), ("tab", "space3"), ("tab", "menu"), ("tab", "unread"), ("tab", "search"), ("tab", "list")]
        } else if !COMPACT.load(Ordering::SeqCst) && SPACES.load(Ordering::SeqCst) {
            vec![("tab", "edit"), ("shift-tab", "rail2"), ("shift-tab", "rail1"), ("shift-tab", "rail0"), ("tab", "rail1"), ("tab", "rail2"), ("tab", "edit"), ("tab", "unread"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "edit"), ("shift-tab", "rail2")]
        } else if COMPACT.load(Ordering::SeqCst) {
            if CONNECTED.load(Ordering::SeqCst) {
                vec![("tab", "unread"), ("tab", "search"), ("tab", "list"), ("tab", "space0"), ("shift-tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "menu"), ("shift-tab", "space0"), ("tab", "menu")]
            } else {
                vec![("tab", "search"), ("tab", "list"), ("tab", "menu"), ("shift-tab", "list"), ("shift-tab", "search"), ("shift-tab", "menu")]
            }
        } else if CONNECTED.load(Ordering::SeqCst) {
            vec![("tab", "edit"), ("tab", "unread"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "unread"), ("shift-tab", "edit"), ("shift-tab", "rail")]
        } else {
            vec![("tab", "edit"), ("tab", "search"), ("tab", "list"), ("shift-tab", "search"), ("shift-tab", "edit"), ("shift-tab", "rail")]
        };
        for (key, expected) in steps {
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
            }).unwrap();
            this.update_in(cx, |this, window, cx| {
                let focused = match expected {
                    "unread" => this.unread_focus.is_focused(window),
                    "menu" => this.menu_focus.is_focused(window),
                    "edit" => this.rail_ui.edit_focus.is_focused(window),
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
                assert!(focused, "{key} must reach {expected} through actual Shell dispatch (focus is on {})", ring_label(this, window, cx));
                assert_eq!(this.sidebar.selected().unwrap().scope, scopes[0]);
                assert_eq!(this.active_scope, scopes[0]);
                assert!(!this.theme.is_dark() && !this.sidebar.unread_only());
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
        // The list is ordered by recency, so the row below scopes[0] is not necessarily scopes[1].
        let below = this.update_in(cx, |this, _, _| {
            let visible = this.sidebar.visible_indices();
            let at = visible.iter().position(|&i| this.sidebar.rooms()[i].scope == scopes[0]).unwrap();
            this.sidebar.rooms()[visible[at + 1]].scope.clone()
        }).unwrap();
        for (key, expected) in [("down", &below), ("up", &scopes[0])] {
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx); }).unwrap();
            this.update_in(cx, |this, window, cx| {
                assert!(this.list_focus.is_focused(window));
                assert_eq!(&this.sidebar.selected().unwrap().scope, expected);
                assert_eq!(&this.active_scope, expected);
                if expected == &scopes[0] {
                    this.composer.update(cx, |v,cx| {
                        let mut actual=None;
                        assert_eq!(v.text_for_range(0..65536,&mut actual,window,cx).as_deref(), Some(DRAFTS[0]));
                    });
                    assert!(history(this));
                }
                eprintln!("list arrow {key}: exact room/history");
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(15, Ordering::SeqCst);
    COMPACT.store(
        std::env::var("P111_COMPACT").as_deref() == Ok("1"),
        Ordering::SeqCst,
    );
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_list_chat_seam_runs_through_footer_and_info_both_ways() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_list_seam_runs_through_footer_both_ways() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn compact_chat_seam_runs_from_back_through_info_both_ways() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(16, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CHAT.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_chat_end_wraps_to_menu_both_ways() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(17, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wide_chat_end_without_eligible_send_wraps_from_composer() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(18, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn empty_list_keeps_ring_and_selection() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
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
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(19, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn theme_change_keeps_every_ring_control_focused() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(20, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn page_up_and_home_keep_the_row_being_read_in_place() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(21, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wheel_over_room_sheet_scrolls_only_the_member_list() {
    DYNAMIC.store(40, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wheel_over_room_sheet_matrix_edges_inertia_scrollbar_resize() {
    DYNAMIC.store(41, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn wheel_and_scrollbar_drag_keep_the_row_being_read_in_place() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(22, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn older_history_strip_shows_only_while_loading_or_after_a_failure() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(23, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn room_row_shows_the_last_real_message_as_preview() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(24, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn empty_rooms_are_hidden_until_the_control_shows_them() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(25, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn list_header_puts_menu_icon_and_space_name_above_a_full_width_search() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(30, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn menu_button_opens_and_closes_the_drawer_and_moves_focus() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(26, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn settings_entry_opens_the_panel_and_the_debug_switch_persists() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(27, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn debug_mode_shows_event_ids_and_the_room_line() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(28, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn drawer_offers_devices_and_notifications_and_the_footer_does_not() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    DYNAMIC.store(29, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn tab_ring_wide_from_the_menu_button() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(32, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

#[test]
#[ignore = "requires private display"]
fn tab_ring_compact_from_the_menu_button() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    DYNAMIC.store(33, Ordering::SeqCst);
    COMPACT.store(true, Ordering::SeqCst);
    CONNECTED.store(true, Ordering::SeqCst);
    SPACES.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}
