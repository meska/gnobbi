//! Real-pointer test of the space edit panel: drag a row, drop outside, Alt+↑/↓, auto-scroll.
//! X11 (xdotool, functional run, Xvfb paints Vulkan black so no screenshots):
//!   the ct script 'the gui-test-x11 script "<features>" rail_edit_panel_real_pointer'
//! Wayland (sway headless, same test, pointer over wayvnc, screenshots with grim):
//!   the ct script 'the gui-test-wayland script "<features>" rail_edit_panel_real_pointer'
//! macOS: libtest runs tests off the main thread and gpui needs it, so build the plain binary instead:
//!   cargo build -p matrix-desktop --features gui-fixture && RAIL_DRAG_MANUAL=1 target/debug/gnobbi
//! macOS has no scripted pointer here (screencapture and CGEvent are denied in the VM): with RAIL_DRAG_MANUAL=1 the
//! test only builds the fixture, then waits for a drag and for Alt+Down done by hand (cua-vm) and checks the result.
//! Screenshots go to $SHOT_DIR (default $HOME/shots); P109_DARK=1 runs the dark theme.
use crate::*;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
const ROWS: usize = 24;
const X: i32 = 120; // inside the drawer (left 0, width 280), clear of the Nascondi button
const HEAD: f32 = 56.; // fixed «Gruppi / Crea / Chiudi» header
const SLOT: f32 = 46.; // 2 px line + 44 px row

/// Pointer and keys through xdotool (X11) or, when WL_INPUT is set, a held-button RFB session to wayvnc + wtype.
pub(crate) enum Input {
    X(String),
    Wl(std::process::Child, std::io::BufReader<std::process::ChildStdout>),
}

fn run(cmd: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(cmd).args(args).output().unwrap_or_else(|e| panic!("{cmd}: {e}"));
    assert!(out.status.success(), "{cmd} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

impl Input {
    pub(crate) fn new() -> Self {
        if let Ok(helper) = std::env::var("WL_INPUT") {
            let script = std::path::Path::new(&helper).with_file_name("rfb-send.py");
            let port = std::env::var("WL_VNC_PORT").unwrap_or_else(|_| "5902".into());
            let mut child = std::process::Command::new("python3")
                .arg(script)
                .args(["127.0.0.1", &port, "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("rfb-send.py");
            let out = std::io::BufReader::new(child.stdout.take().unwrap());
            return Input::Wl(child, out);
        }
        let ids = run("xdotool", &["search", "--onlyvisible", "--pid", &std::process::id().to_string()]);
        Input::X(ids.lines().next().expect("window").to_string())
    }

    fn rfb(&mut self, line: &str) {
        let Input::Wl(child, out) = self else { unreachable!() };
        writeln!(child.stdin.as_mut().unwrap(), "{line}").unwrap();
        let mut ack = String::new();
        out.read_line(&mut ack).unwrap();
        assert_eq!(ack.trim(), "ok", "rfb {line}");
    }

    pub(crate) fn mv(&mut self, x: i32, y: i32) {
        match self {
            Input::X(win) => drop(run("xdotool", &["mousemove", "--window", win, &x.to_string(), &y.to_string()])),
            Input::Wl(..) => self.rfb(&format!("move {x} {y}")),
        }
    }

    pub(crate) fn button(&mut self, down: bool) {
        match self {
            Input::X(_) => drop(run("xdotool", &[if down { "mousedown" } else { "mouseup" }, "1"])),
            Input::Wl(..) => self.rfb(if down { "down" } else { "up" }),
        }
    }

    /// `key` is "Up" or "Down"; Alt is held.
    fn alt_key(&self, key: &str) {
        match self {
            Input::X(_) => drop(run("xdotool", &["key", &format!("alt+{key}")])),
            Input::Wl(..) => drop(run("wtype", &["-M", "alt", "-k", key, "-m", "alt"])),
        }
    }

    pub(crate) fn key(&self, key: &str) {
        match self {
            Input::X(_) => drop(run("xdotool", &["key", key])),
            Input::Wl(..) => drop(run("wtype", &["-k", key])),
        }
    }

    /// Pointer path; every step lets the app handle the event before the next one.
    pub(crate) async fn glide(&mut self, ex: &gpui::BackgroundExecutor, from: (i32, i32), to: (i32, i32), steps: i32) {
        for i in 1..=steps {
            self.mv(from.0 + (to.0 - from.0) * i / steps, from.1 + (to.1 - from.1) * i / steps);
            pause(ex, 40).await;
        }
    }

    pub(crate) fn shot(&self, tag: &str, name: &str) {
        if matches!(self, Input::X(_)) {
            eprintln!("SHOT {tag}-{name} skipped: Xvfb shows Vulkan presentation black (use the Wayland run)");
            return;
        }
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| format!("{}/shots", std::env::var("HOME").unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = format!("{dir}/{tag}-{name}.png");
        run("grim", &[&path]);
        eprintln!("SHOT {path}");
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if let Input::Wl(child, _) = self {
            drop(child.stdin.take());
            let _ = child.wait();
        }
    }
}

fn names(this: &Shell) -> Vec<String> {
    this.rail_arranged().into_iter().skip(1).map(|(e, _)| e.name).collect()
}

pub(crate) async fn pause(ex: &gpui::BackgroundExecutor, ms: u64) {
    ex.timer(Duration::from_millis(ms)).await;
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) && std::env::var_os("RAIL_DRAG_MANUAL").is_none() {
        return;
    }
    let ex = cx.background_executor().clone();
    let tag = if std::env::var("P109_DARK").as_deref() == Ok("1") { "dark" } else { "light" };
    cx.spawn_in(window, async move |this, cx| {
        pause(&ex, 500).await;
        let height = this
            .update_in(cx, |this, window, cx| {
                let account = this.sidebar.rooms()[0].scope.account_id;
                for i in 1..=ROWS {
                    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(Box::new(matrix_engine::RoomSummary {
                        scope: RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(format!("!s{i:02}:fixture")) },
                        name: format!("Spazio {i:02}"),
                        unread: 0,
                        info: matrix_engine::RoomInfo { is_space: true, ..Default::default() },
                    })));
                }
                if tag == "dark" {
                    this.set_theme(Theme::Dark, cx);
                }
                if std::env::var_os("RAIL_DRAG_MANUAL").is_some() {
                    window.resize(gpui::size(px(960.), px(640.))); // fits the 1024x768 VM screen
                }
                this.connected = true;
                this.rail_ui.edit_open = true;
                this.rail_ui.edit_panel_focus.focus(window);
                cx.notify();
                f32::from(window.viewport_size().height)
            })
            .unwrap();
        pause(&ex, 500).await;
        if std::env::var_os("RAIL_DRAG_MANUAL").is_some() {
            let original: Vec<String> = (1..=ROWS).map(|i| format!("Spazio {i:02}")).collect();
            let mut want = original.clone();
            let moved = want.remove(3);
            want.insert(1, moved);
            let mut after_key = want.clone();
            after_key.swap(1, 2);
            let mut prev = original;
            for (label, expect) in [("drag row 4 onto row 2", want), ("Alt+Down", after_key)] {
                let mut now = prev.clone();
                for _ in 0..900 {
                    pause(&ex, 200).await;
                    now = this.update_in(cx, |this, _, _| names(this)).unwrap();
                    if now != prev {
                        break;
                    }
                }
                assert_eq!(now, expect, "manual step: {label}");
                eprintln!("MANUAL ok: {label}");
                std::fs::write(format!("{}/manual-{}.ok", std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into()), label.replace(' ', "-")), "").unwrap();
                prev = now;
            }
            pause(&ex, 60_000).await;
            this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
            return;
        }
        let mut input = Input::new();
        let row_y = |i: usize| (HEAD + SLOT * i as f32 + 24.) as i32;
        let get = |cx: &mut gpui::AsyncWindowContext| this.update_in(cx, |this, _, _| names(this)).unwrap();
        let original: Vec<String> = (1..=ROWS).map(|i| format!("Spazio {i:02}")).collect();
        assert_eq!(get(cx), original, "fixture order");

        // 1. Drag row 3 onto row 1: the line shows above row 1, the drop lands there.
        input.mv(X, row_y(3));
        input.button(true);
        input.glide(&ex, (X, row_y(3)), (X, row_y(1)), 8).await;
        pause(&ex, 300).await;
        input.shot(tag, "during");
        assert_eq!(get(cx), original, "nothing moves before the release");
        input.button(false);
        pause(&ex, 400).await;
        let mut want = original.clone();
        let moved = want.remove(3);
        want.insert(1, moved);
        assert_eq!(get(cx), want, "drag row 3 -> position 1");
        input.shot(tag, "after");

        // 2. Release outside the list: nothing changes.
        input.mv(X, row_y(0));
        input.button(true);
        input.glide(&ex, (X, row_y(0)), (900, 400), 8).await;
        pause(&ex, 200).await;
        input.shot(tag, "outside");
        input.button(false);
        pause(&ex, 300).await;
        assert_eq!(get(cx), want, "drop outside the list changes nothing");

        // 3. Alt+Down / Alt+Up move the row under the cursor (row 1 after the drop) and the cursor follows.
        this.update_in(cx, |this, window, _| this.rail_ui.edit_panel_focus.focus(window)).unwrap();
        let mut prev = 1;
        for (key, to) in [("Down", 2), ("Down", 3), ("Up", 2), ("Up", 1), ("Up", 0), ("Up", 0), ("Down", 1)] {
            // Each wtype installs a new keymap and the client may miss the first key: resend while nothing happened.
            let (before, wanted_move) = (get(cx), to != prev);
            for _ in 0..3 {
                input.alt_key(key);
                pause(&ex, 300).await;
                if !wanted_move || get(cx) != before {
                    break;
                }
            }
            prev = to;
            let cursor = this.update_in(cx, |this, _, _| this.rail_ui.edit_cursor).unwrap();
            assert_eq!(cursor, to, "{key}: cursor");
            assert_eq!(get(cx)[to], "Spazio 04", "{key}: the moved row sits at {to}");
        }
        let now = get(cx);
        assert_eq!(now.len(), ROWS);
        assert_eq!(now.iter().collect::<std::collections::BTreeSet<_>>().len(), ROWS, "no row lost or duplicated");

        // 4. Auto-scroll: drag the first row into the bottom edge zone, then onto a row in the middle.
        let first = now[0].clone();
        let offset = |cx: &mut gpui::AsyncWindowContext| this.update_in(cx, |this, _, _| f32::from(this.rail_ui.edit_scroll.offset().y)).unwrap();
        assert_eq!(offset(cx), 0.);
        input.mv(X, row_y(0));
        input.button(true);
        let edge = height as i32 - 20;
        input.glide(&ex, (X, row_y(0)), (X, edge), 10).await;
        input.glide(&ex, (X, edge), (X, edge - 6), 12).await; // small moves inside the zone
        let moving = offset(cx);
        pause(&ex, 600).await; // pointer still: the list must keep scrolling on its own
        input.shot(tag, "autoscroll-during");
        let still = offset(cx);
        let note = format!("AUTOSCROLL after moving in the edge zone: {moving}; after 600 ms of a still pointer: {still}\n");
        eprint!("{note}");
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| format!("{}/shots", std::env::var("HOME").unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::OpenOptions::new().create(true).append(true).open(format!("{dir}/{tag}-notes.txt")).unwrap().write_all(note.as_bytes()).unwrap();
        assert!(moving < 0., "moving in the bottom edge zone must scroll the list");
        assert!(still < moving, "a still pointer in the edge zone keeps scrolling ({moving} -> {still})");
        let mid = (height / 2.) as i32;
        input.glide(&ex, (X, edge - 6), (X, mid), 8).await;
        let off = offset(cx);
        pause(&ex, 200).await;
        input.button(false);
        pause(&ex, 400).await;
        let slot = ((mid as f32 - HEAD - off) / SLOT).floor() as usize;
        let got = get(cx);
        assert_eq!(got[slot - 1], first, "row dropped on slot {slot} (offset {off}) after auto-scroll");
        input.shot(tag, "autoscroll-after");

        // 5. Closing: a click on the scrim, then Esc.
        let open = |cx: &mut gpui::AsyncWindowContext| this.update_in(cx, |this, _, _| this.rail_ui.edit_open).unwrap();
        assert!(open(cx), "drawer open before closing");
        input.mv(700, 300);
        input.button(true);
        input.button(false);
        pause(&ex, 300).await;
        assert!(!open(cx), "click on the scrim closes the drawer");
        this.update_in(cx, |this, window, cx| {
            this.rail_ui.edit_open = true;
            this.rail_ui.edit_panel_focus.focus(window);
            cx.notify();
        })
        .unwrap();
        pause(&ex, 300).await;
        input.key("Escape");
        pause(&ex, 300).await;
        assert!(!open(cx), "Esc closes the drawer");

        this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
    })
    .detach();
}

#[cfg(test)]
#[test]
#[ignore = "requires private display"]
fn rail_edit_panel_real_pointer_drag_alt_keys_and_autoscroll() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
}
