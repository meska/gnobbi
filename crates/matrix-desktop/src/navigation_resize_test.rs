//! Native event-boundary regression; run only on an isolated display.
use crate::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static FROZEN: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Option<[bool; 5]>> = Mutex::new(None);
static STAMPS: Mutex<Option<[bool; 6]>> = Mutex::new(None);
static SEND: AtomicBool = AtomicBool::new(false);
#[cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
static SIGNIN: AtomicBool = AtomicBool::new(false);

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn stamp_intents(
    this: &Shell,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> Vec<matrix_desktop::accessibility::event_id_copy::Intent> {
    let history = this
        .timeline
        .as_ref()
        .unwrap()
        .semantic_snapshot(&mut Default::default(), window);
    let target = history.messages[0]
        .event_id_copy
        .as_ref()
        .unwrap()
        .target
        .clone();
    let intents =
        [true, false].map(
            |focus_only| matrix_desktop::accessibility::event_id_copy::Intent {
                target: target.clone(),
                focus_only,
            },
        );
    for intent in &intents {
        assert!(
            this.timeline
                .as_ref()
                .unwrap()
                .apply_event_id_copy(intent, window, cx)
        );
    }
    intents.into()
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn reject_stamp_intents(
    this: &Shell,
    intents: &[matrix_desktop::accessibility::event_id_copy::Intent],
    window: &mut Window,
    cx: &mut Context<Shell>,
) {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("queued sentinel".into()));
    for intent in intents {
        assert!(
            !this
                .timeline
                .as_ref()
                .unwrap()
                .apply_event_id_copy(intent, window, cx)
        );
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("queued sentinel")
        );
    }
}

const SEND_DRAFT: &str = " caffè 🦀\nbozza ";

fn capture_send(
    this: &mut Shell,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + use<> {
    use gpui::EntityInputHandler;
    this.composer.update(cx, |v, cx| {
        v.replace_text_in_range(Some(0..65536), SEND_DRAFT, window, cx)
    });
    let pointer = this.composer.read(cx).send_callback();
    let key = this.composer.read(cx).key_callback();
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    let intents = send_intents(this, window, cx);
    move |this, window, cx| {
        let outside = cx.focus_handle();
        outside.focus(window);
        this.composer.update(cx, |v, cx| {
            pointer(v, window, cx);
            key(
                v,
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("enter").unwrap(),
                    is_held: false,
                },
                window,
                cx,
            );
            let mut actual = None;
            assert_eq!(
                v.text_for_range(0..65536, &mut actual, window, cx)
                    .as_deref(),
                Some(SEND_DRAFT)
            );
        });
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        for intent in &intents {
            assert!(this.dispatch_composer_action(intent, window, cx));
        }
        assert!(
            outside.is_focused(window),
            "retired Send must not alter focus"
        );
        this.composer.update(cx, |v, cx| {
            let mut actual = None;
            assert_eq!(
                v.text_for_range(0..65536, &mut actual, window, cx)
                    .as_deref(),
                Some(SEND_DRAFT)
            );
        });
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
fn send_intents(
    this: &Shell,
    window: &Window,
    cx: &Context<Shell>,
) -> [matrix_desktop::accessibility::NavigationAction; 2] {
    use matrix_desktop::accessibility::{NavigationSnapshot, semantic_session::SemanticSession};
    let snapshot = NavigationSnapshot {
        title: matrix_desktop::window::TITLE.into(),
        new_chat: Default::default(),
        #[cfg(feature = "live")]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: Some(
            this.composer
                .read(cx)
                .semantic_snapshot(this.active_scope.clone(), window),
        ),
    };
    let session = SemanticSession::new(snapshot);
    let send = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Invia messaggio"))
        .unwrap()
        .0;
    [accesskit::Action::Focus, accesskit::Action::Click].map(|action| {
        session
            .action(&accesskit::ActionRequest {
                action,
                target_tree: accesskit::TreeId::ROOT,
                target_node: send,
                data: None,
            })
            .unwrap()
    })
}

fn install_send(window: &mut Window, cx: &mut Context<Shell>) {
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        executor.timer(std::time::Duration::from_millis(400)).await;
        let captured = this.update_in(cx, |this, window, cx| {
            FROZEN.store(true, Ordering::SeqCst);
            let old = capture_send(this, window, cx);
            this.set_notifications_open(true);
            old(this, window, cx);
            this.set_notifications_open(false);
            old(this, window, cx);
            let old = capture_send(this, window, cx);
            this.navigation_wide = false;
            this.set_compact_chat(true);
            this.set_compact_chat(false);
            old(this, window, cx);
            this.set_compact_chat(true);
            old(this, window, cx);
            this.navigation_wide = true;
            this.set_compact_chat(false);
            let old = capture_send(this, window, cx);
            resize("360", "480");
            old
        }).unwrap();
        executor.timer(std::time::Duration::from_millis(400)).await;
        this.update_in(cx, |this, window, cx| {
            assert_eq!(window.viewport_size().width, px(360.));
            captured(this, window, cx);
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                this.set_compact_chat(true);
                this.composer.update(cx, |v, cx| v.focus_send(window, cx));
                let key = KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                    is_held: false,
                };
                assert!(this.composer.read(cx).semantic_snapshot(this.active_scope.clone(), window).send_focused);
                assert!(!this.back_key(&key, window, cx), "compact Back must not capture Send reverse Tab");
                let callback = this.composer.read(cx).key_callback();
                this.composer.update(cx, |v, cx| callback(v, &key, window, cx));
                assert!(this.composer.read(cx).semantic_snapshot(this.active_scope.clone(), window).focused);
                assert!(this.back_key(&key, window, cx), "editor reverse Tab must still reach compact Back");
                assert!(this.back_ui.focus.is_focused(window));
                this.set_compact_chat(false);
            }
            resize("1120", "760");
        }).unwrap();
        executor.timer(std::time::Duration::from_millis(400)).await;
        this.update_in(cx, |this, window, cx| {
            assert_eq!(window.viewport_size().width, px(1120.));
            captured(this, window, cx);
            eprintln!("Shell hidden/compact/real Configure away-back with FROZEN projection: captured pointer/key and translated semantic sources rejected");
            FROZEN.store(false, Ordering::SeqCst);
            cx.notify();
        }).unwrap();
        executor.timer(std::time::Duration::from_millis(400)).await;
        this.update_in(cx, |this, window, cx| {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                use gpui::EntityInputHandler;
                let snapshot = this.composer.read(cx).semantic_snapshot(this.active_scope.clone(), window);
                let bounds = snapshot.send_bounds.expect("actual Shell repaint reacquires current Send bounds");
                assert!(bounds.width() > 0. && bounds.height() > 0.);
                assert!(!snapshot.send_focused);
                let intents = send_intents(this, window, cx);
                assert!(this.dispatch_composer_action(&intents[0], window, cx));
                assert!(this.composer.read(cx).semantic_snapshot(this.active_scope.clone(), window).send_focused);
                assert!(this.dispatch_composer_action(&intents[1], window, cx));
                assert!(this.dispatch_composer_action(&intents[1], window, cx));
                this.composer.update(cx, |v, cx| {
                    let mut actual = None;
                    assert_eq!(v.text_for_range(0..65536, &mut actual, window, cx).as_deref(), Some(""));
                });
                eprintln!("actual Shell repaint: current bounds and translated SemanticSession Focus/Click usable; repeated intent retired");
            }
            let _ = (this, window);
            cx.quit();
        }).unwrap();
    }).detach();
}

#[test]
#[ignore = "requires private display; runner"]
fn send_sources_retire_on_unpainted_shell_and_native_bounds_transitions() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    SEND.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}

/// Under the Wayland test runner (WL_INPUT set) the runner starts one process per test, so the only window is tiled at the output origin and window and output coordinates agree.
#[cfg(feature = "live")]
pub(crate) fn click(x: i32, y: i32) {
    let ok = match std::env::var("WL_INPUT") {
        Ok(helper) => std::process::Command::new(helper)
            .arg(format!("click:{x},{y}"))
            .status()
            .unwrap()
            .success(),
        Err(_) => {
            let ids = std::process::Command::new("xdotool")
                .args([
                    "search",
                    "--onlyvisible",
                    "--pid",
                    &std::process::id().to_string(),
                ])
                .output()
                .unwrap();
            let win = String::from_utf8(ids.stdout)
                .unwrap()
                .lines()
                .next()
                .expect("window")
                .to_string();
            std::process::Command::new("xdotool")
                .args([
                    "mousemove",
                    "--window",
                    &win,
                    &x.to_string(),
                    &y.to_string(),
                    "click",
                    "1",
                ])
                .status()
                .unwrap()
                .success()
        }
    };
    assert!(ok);
}

pub(crate) fn resize(width: &str, height: &str) {
    if std::env::var_os("SWAYSOCK").is_some() {
        let cmd = format!(
            "[pid={}] floating enable, resize set width {width} px height {height} px",
            std::process::id()
        );
        assert!(
            std::process::Command::new("swaymsg")
                .arg(cmd)
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    let output = std::process::Command::new("xdotool")
        .args([
            "search",
            "--onlyvisible",
            "--pid",
            &std::process::id().to_string(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let ids = String::from_utf8(output.stdout).unwrap();
    let id = ids.lines().next().unwrap();
    assert!(
        std::process::Command::new("xdotool")
            .args(["windowsize", id, width, height])
            .status()
            .unwrap()
            .success()
    );
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    if SEND.load(Ordering::SeqCst) {
        install_send(window, cx);
        return;
    }
    #[cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
    if SIGNIN.load(Ordering::SeqCst) {
        install_signin_queue(window, cx);
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        executor.timer(std::time::Duration::from_millis(500)).await;
        let captured = this
            .update_in(cx, |this, window, cx| {
                // Suppress projection between real Configure events, not the event loop.
                FROZEN.store(true, Ordering::SeqCst);
                let timeline =
                    TimelineView::connected(this.active_scope.clone(), cx.focus_handle());
                timeline.append_server(vec![matrix_engine::history::HistoryEvent {
                    redacted: false,
                    event_id: matrix_domain::EventId::new("$visibility:test"),
                    body: "deterministic native visibility fixture".into(),
                    outgoing: true,
                    timestamp_ms: 0,
                    sender: matrix_engine::history::HistorySender::new("@peer:test", None),
                    video: None,
                    styles: Vec::new(),
                    thread: None,
                    thread_root: None,
                    status: false,
                    attachment_name: None,
                    attachment_size: None,
                }]);
                this.timeline = Some(timeline);
                this.navigation_wide = false;
                this.set_compact_chat(true);
                let compact_stamp = this.timeline.as_ref().unwrap().event_stamp_probe();
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let compact_intents = stamp_intents(this, window, cx);
                assert!(compact_stamp(false));
                this.set_compact_chat(false);
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                reject_stamp_intents(this, &compact_intents, window, cx);
                let compact_away = compact_stamp(false);
                this.set_compact_chat(true);
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                reject_stamp_intents(this, &compact_intents, window, cx);
                let compact_return = compact_stamp(false);
                let compact_current = compact_stamp(true);
                this.navigation_wide = true;
                this.set_compact_chat(false);
                let resize_stamp = this.timeline.as_ref().unwrap().event_stamp_probe();
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                let resize_intents = stamp_intents(this, window, cx);
                let _ = window;
                assert!(resize_stamp(false));
                let callback = this.sidebar.navigation_callback();
                resize("360", "480");
                (
                    callback,
                    resize_stamp,
                    [compact_away, compact_return, compact_current],
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    resize_intents,
                )
            })
            .unwrap();
        executor.timer(std::time::Duration::from_millis(500)).await;
        let away = this
            .update_in(cx, |this, window, _cx| {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                reject_stamp_intents(this, &captured.3, window, _cx);
                println!(
                    "away viewport: {:?}, platform: {:?}",
                    window.viewport_size(),
                    window.bounds()
                );
                let result = (
                    window.viewport_size().width == px(360.),
                    captured.0(&this.sidebar),
                    (captured.1)(false),
                );
                resize("1120", "760");
                result
            })
            .unwrap();
        executor.timer(std::time::Duration::from_millis(500)).await;
        this.update_in(cx, |this, window, cx| {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            reject_stamp_intents(this, &captured.3, window, cx);
            let returned = (
                window.viewport_size().width == px(1120.),
                captured.0(&this.sidebar),
                this.sidebar.navigation_callback()(&this.sidebar),
            );
            *RESULT.lock().unwrap() = Some([away.0, away.1, returned.0, returned.1, returned.2]);
            *STAMPS.lock().unwrap() = Some([
                captured.2[0],
                captured.2[1],
                captured.2[2],
                away.2,
                (captured.1)(false),
                (captured.1)(true),
            ]);
            cx.quit();
        })
        .unwrap();
    })
    .detach();
}

#[test]
#[ignore = "requires an isolated native display; no host input"]
fn native_resize_retires_a_queued_source_without_an_intermediate_projection() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1"),
        "use the isolated-display runner, never the host display"
    );
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let result = RESULT
        .lock()
        .unwrap()
        .take()
        .expect("native probe completed");
    println!("native resize source result: {result:?}");
    assert_eq!(result, [true, false, true, false, true]);
    let stamps = STAMPS
        .lock()
        .unwrap()
        .take()
        .expect("stamp probe completed");
    println!("compact and Configure stamp sources: {stamps:?}");
    assert_eq!(stamps, [false, false, true, false, false, true]);
}

#[cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
fn install_signin_queue(window: &mut Window, cx: &mut Context<Shell>) {
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        executor.timer(std::time::Duration::from_millis(300)).await;
        this.update_in(cx, |this, window, cx| {
            use matrix_desktop::accessibility::{NavigationAction as A, signin::{Control, Intent}};
            use matrix_engine::signin::{SignInUi, SignInEvent, SignInPrompt, SignInMode};
            for mode in [SignInMode::Password, SignInMode::Unlock] {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let prompt = SignInEvent::Prompt(SignInPrompt { mode, error: None });
            events.try_send(prompt).unwrap();
            let view = cx.new(|cx| {
                let mut view = crate::signin_view::SignInView::new(SignInUi { events: event_rx, setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1, commands }, cx);
                view.poll(cx);
                view
            });
            this.signin = Some(view.clone());
            let source = view.read(cx).semantic_snapshot(window,cx).source.unwrap();
            let old = Intent { source, control: Control::Phrase, focus_only: true };
            assert!(this.dispatch_signin_action(&A::SignIn(old),window,cx));
            assert_eq!(view.read(cx).semantic_snapshot(window,cx).focused,Some(Control::Phrase));
            let selected = this.active_scope.clone();
            let dark = this.theme.is_dark();
            for action in [A::FocusMenu, A::FocusSearch, A::FocusComposer, A::SubmitComposer { source: 0, scope: selected.clone(), text: "hidden chat sentinel".into() }] {
                assert!(this.dispatch_signin_action(&action,window,cx));
            }
            assert_eq!(this.active_scope, selected);
            assert_eq!(this.theme.is_dark(),dark);
            assert_eq!(view.read(cx).semantic_snapshot(window,cx).focused,Some(Control::Phrase));
            events.try_send(prompt).unwrap(); view.update(cx,|v,cx| { v.poll(cx); });
            let replacement = view.read(cx).semantic_snapshot(window,cx).source.unwrap();
            assert_ne!(source,replacement);
            assert!(this.dispatch_signin_action(&A::SignIn(old),window,cx));
            assert_eq!(view.read(cx).semantic_snapshot(window,cx).focused,None);
            let submit = A::SignIn(Intent { source: replacement, control: Control::Submit, focus_only: false });
            assert!(this.dispatch_signin_action(&submit,window,cx));
            let retry = view.read(cx).semantic_snapshot(window,cx).source.unwrap();
            assert_ne!(retry,replacement);
            assert!(this.dispatch_signin_action(&submit,window,cx));
            assert_eq!(view.read(cx).semantic_snapshot(window,cx).source,Some(retry));
            events.try_send(SignInEvent::Authenticated(matrix_domain::AccountId::new(9))).unwrap();
            assert_eq!(view.update(cx,|v,cx| v.poll(cx)), Some(matrix_domain::AccountId::new(9)));
            this.signin = None;
            assert!(this.dispatch_signin_action(&submit,window,cx));
            assert!(this.dispatch_signin_action(&A::SignIn(old),window,cx));
            assert!(!this.dispatch_signin_action(&A::FocusComposer,window,cx));
            assert!(requests.try_recv().is_err());
            assert_eq!(this.active_scope,selected);
            eprintln!("Shell: current field focus; hidden translated chat actions swallowed; identical prompt and local retry duplicates rejected; post-success sign-in actions swallowed, chat actions released");
            }
            cx.quit();
        }).unwrap();
    }).detach();
}

#[test]
#[cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
#[ignore = "requires isolated display"]
fn shell_signin_queue_boundary_does_not_dispatch_to_hidden_chat() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    SIGNIN.store(true, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}
