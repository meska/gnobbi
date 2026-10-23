//! Main-thread macOS probe for the sign-in field key callback (feature `perf-probe` + `live`).
//! Calls the ORIGINAL `LoginField::key` inside a real GPUI Application/window with a real
//! `KeyDownEvent`; the clipboard is the real GPUI platform clipboard, written only with PUBLIC
//! synthetic sentinels. NOT OS key injection. Prints booleans only, never field or clipboard text.
//! Clipboard ownership is checked through the NSPasteboard changeCount; that narrows but cannot
//! remove a check-then-use race with other writers.
use super::*;
use std::sync::Mutex;

static CHECKS: Mutex<Vec<(&'static str, bool)>> = Mutex::new(Vec::new());
const PUBLIC_SENTINEL: &str = "P115B public sentinel è 🙂";
const FINAL_SENTINEL: &str = "P115B public harmless sentinel";
const PROTECTED_SYNTH: &str = "synthétic-🙂-protected";
const MULTILINE: &str = "linea uno\nlinea due 🙂";

fn record(name: &'static str, ok: bool) {
    eprintln!("P115B check={name} ok={ok}");
    CHECKS.lock().unwrap().push((name, ok));
}

/// Native NSPasteboard changeCount (a counter, not contents) via JXA.
fn change_count() -> i64 {
    let out = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            "ObjC.import('AppKit'); $.NSPasteboard.generalPasteboard.changeCount",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(-1)
}

/// Write our own public text; returns the expected changeCount only if exactly one change
/// (ours) happened. The previous clipboard content is never read.
fn own_write(cx: &mut gpui::App, text: &str) -> Option<i64> {
    let before = change_count();
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into()));
    let after = change_count();
    (before >= 0 && after == before + 1).then_some(after)
}

fn read_text(cx: &gpui::App) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn press(field: &mut LoginField, chord: &str, window: &mut Window, cx: &mut Context<LoginField>) {
    let event = KeyDownEvent {
        keystroke: gpui::Keystroke::parse(chord).unwrap(),
        is_held: false,
    };
    field.key(&event, window, cx);
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn run_checks(
    which: &str,
    field: &mut LoginField,
    secret: &gpui::Entity<LoginField>,
    window: &mut Window,
    cx: &mut Context<LoginField>,
) {
    match which {
        "select_all" => {
            field.input.replace(None, PUBLIC_SENTINEL);
            press(field, "cmd-a", window, cx);
            record(
                "cmd_a_selects_whole_public_field",
                field.input.selection_utf16() == (0..units(PUBLIC_SENTINEL)),
            );
        }
        "paste" => {
            let own = own_write(cx, PUBLIC_SENTINEL);
            let guard = own.is_some_and(|n| change_count() == n);
            record("clipboard_owned_before_public_paste", guard);
            if guard {
                press(field, "cmd-v", window, cx);
                record(
                    "cmd_v_pastes_unicode_into_public_field",
                    field.input.display_text() == PUBLIC_SENTINEL
                        && own.is_some_and(|n| change_count() == n),
                );
            }
            let own = own_write(cx, PROTECTED_SYNTH);
            let guard = own.is_some_and(|n| change_count() == n);
            record("clipboard_owned_before_protected_paste", guard);
            if guard {
                secret.update(cx, |f, cx| {
                    press(f, "cmd-v", window, cx);
                    let shown = f.input.display_text();
                    record(
                        "cmd_v_into_protected_is_masked",
                        shown.chars().count() == units(PROTECTED_SYNTH)
                            && shown.chars().all(|c| c == '\u{2022}')
                            && own.is_some_and(|n| change_count() == n),
                    );
                });
            }
            // Multiline input is rejected atomically by the existing single-line validator.
            for protected in [false, true] {
                let own = own_write(cx, MULTILINE);
                let guard = own.is_some_and(|n| change_count() == n);
                let name = if protected {
                    "multiline_paste_rejected_atomically_protected"
                } else {
                    "multiline_paste_rejected_atomically_public"
                };
                if !guard {
                    record(name, false);
                    continue;
                }
                let target = if protected {
                    secret.clone()
                } else {
                    cx.entity()
                };
                let ok = if protected {
                    target.update(cx, |f, cx| {
                        let before = f.input.display_text();
                        press(f, "cmd-v", window, cx);
                        f.input.display_text() == before
                    })
                } else {
                    let before = field.input.display_text();
                    press(field, "cmd-v", window, cx);
                    field.input.display_text() == before
                };
                record(name, ok && own.is_some_and(|n| change_count() == n));
            }
        }
        "copy_cut" => {
            const MARK: &str = "P115B pre-copy public marker";
            field.input.replace(None, PUBLIC_SENTINEL);
            press(field, "cmd-a", window, cx);
            for chord in ["cmd-c", "cmd-x"] {
                let own = own_write(cx, MARK);
                let guard = own.is_some_and(|n| change_count() == n);
                if !guard {
                    record("clipboard_owned_before_copy_or_cut", false);
                    continue;
                }
                let expected = own.unwrap() + 1;
                press(field, chord, window, cx);
                // Our own action wrote exactly once; read only if nobody else did.
                let exact = change_count() == expected
                    && read_text(cx).as_deref() == Some(PUBLIC_SENTINEL)
                    && change_count() == expected;
                if chord == "cmd-c" {
                    record(
                        "cmd_c_copies_exact_unicode_and_keeps_field",
                        exact && field.input.display_text() == PUBLIC_SENTINEL,
                    );
                } else {
                    record(
                        "cmd_x_cuts_exact_unicode_and_empties_field",
                        exact && field.input.display_text().is_empty(),
                    );
                }
            }
            secret.update(cx, |f, cx| {
                f.input.replace(None, PROTECTED_SYNTH);
                press(f, "cmd-a", window, cx);
                let before = f.input.display_text();
                for chord in ["cmd-c", "cmd-x"] {
                    let name = if chord == "cmd-c" {
                        "protected_cmd_c_never_exports"
                    } else {
                        "protected_cmd_x_never_exports_or_edits"
                    };
                    let own = own_write(cx, MARK);
                    if !own.is_some_and(|n| change_count() == n) {
                        record(name, false);
                        continue;
                    }
                    let expected = own.unwrap();
                    press(f, chord, window, cx);
                    let untouched = change_count() == expected
                        && read_text(cx).as_deref() == Some(MARK)
                        && change_count() == expected;
                    record(name, untouched && f.input.display_text() == before);
                }
            });
        }
        _ => record("known_behavior_selector", false),
    }
}

pub(crate) fn run() {
    let which = std::env::var("P115B_NATIVE").unwrap_or_default();
    gpui::Application::new().run(move |cx| {
        let window = cx
            .open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|cx| {
                    let mut field = LoginField::new(false, cx);
                    field.enabled = true;
                    field
                })
            })
            .unwrap();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            let outcome = window.update(cx, |field, window, cx| {
                let secret = cx.new(|cx| {
                    let mut f = LoginField::new(true, cx);
                    f.enabled = true;
                    f
                });
                run_checks(&which, field, &secret, window, cx);
            });
            // Always leave a public harmless sentinel, even when a check failed.
            cx.update(|cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(FINAL_SENTINEL.into()))
            })
            .ok();
            let checks = CHECKS.lock().unwrap();
            let failed: Vec<_> = checks
                .iter()
                .filter(|(_, ok)| !*ok)
                .map(|(n, _)| *n)
                .collect();
            if outcome.is_err() || checks.is_empty() || !failed.is_empty() {
                eprintln!("P115B-FAIL failed checks: {failed:?}");
                std::process::exit(1);
            }
            eprintln!("P115B-MARKER all {} checks passed", checks.len());
            drop(checks);
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}
