//! Real GPUI field callbacks on a private display; never the owner's clipboard.
use super::*;

const PUBLIC: &str = "a🔑è";
const MARKER: &str = "public clipboard marker";

#[test]
#[ignore = "requires private display"]
fn edit_menu_copy_dispatches_to_focused_public_field() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    gpui::Application::new().run(|cx| {
        let view = cx
            .open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|cx| {
                    let mut field = LoginField::new(false, cx);
                    field.enabled = true;
                    field.input.replace(None, PUBLIC);
                    field.input.edit("a", true, false);
                    field
                })
            })
            .unwrap();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            view.update(cx, |field, window, cx| {
                assert!(field.layout.is_some());
                field.focus.focus(window);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
                window.dispatch_action(crate::editing_command::tests::menu_action(1), cx);
            })
            .unwrap();
            executor.timer(std::time::Duration::from_millis(80)).await;
            view.update(cx, |_, _, cx| {
                let copied = clipboard(cx);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
                assert_eq!(copied.as_deref(), Some(PUBLIC));
            })
            .unwrap();
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}

#[test]
#[ignore = "requires private display"]
fn edit_menu_preserves_protected_and_ineligible_inputs() {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    gpui::Application::new().run(|cx| {
        let view = cx
            .open_window(gpui::WindowOptions::default(), |window, cx| {
                window.set_window_title("Dime private Edit regression");
                cx.new(|cx| LoginField::new(false, cx))
            })
            .unwrap();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            for protected in [false, true] {
                for eligibility in ["current", "disabled", "queued-disabled", "queued-blur"] {
                    for index in 0..4 {
                        view.update(cx, |field, window, cx| {
                            field.input = if protected {
                                LoginInput::secret()
                            } else {
                                LoginInput::public()
                            };
                            field.input.replace(None, "old");
                            if index != 3 { field.input.edit("a", true, false); }
                            field.enabled = eligibility != "disabled";
                            field.focus.focus(window);
                            cx.notify();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        cx.update(|cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                if index == 2 { PUBLIC } else { MARKER }.into(),
                            ));
                            let action = crate::editing_command::tests::menu_action(index);
                            assert!(cx.active_window().is_some(), "native App menu needs an active private window");
                            // Match the native menu's App entry outside any window update.
                            cx.dispatch_action(action.as_ref());
                            view.update(cx, |field, window, cx| {
                                if eligibility == "queued-disabled" { field.enabled = false; }
                                if eligibility == "queued-blur" { cx.focus_handle().focus(window); }
                            }).unwrap();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        view.update(cx, |field, window, cx| {
                            let expected = if eligibility != "current" {
                                "old"
                            } else {
                                match index {
                                    0 if !protected => "",
                                    2 => PUBLIC,
                                    _ => "old",
                                }
                            };
                            if index == 3 {
                                assert_eq!(field.input.selection_utf16(), if eligibility == "current" { 0..3 } else { 3..3 });
                            }
                            assert!(
                                field.input.take().as_str() == expected,
                                "protected-safe exact input assertion: {protected}/{eligibility}/{index}"
                            );
                            let expected_clipboard =
                                if eligibility == "current" && !protected && matches!(index, 0 | 1)
                                {
                                    "old"
                                } else if index == 2 {
                                    PUBLIC
                                } else {
                                    MARKER
                                };
                            let copied = clipboard(cx);
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
                            assert!(
                                copied.as_deref() == Some(expected_clipboard),
                                "no protected export"
                            );
                            if eligibility == "queued-blur" {
                                assert!(!field.focus.is_focused(window));
                            }
                        })
                        .unwrap();
                    }
                }
            }
            eprintln!(
                "32 public/protected menu-action eligibility cases passed; no OS key injection"
            );
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}

fn press(field: &mut LoginField, chord: &str, window: &mut Window, cx: &mut Context<LoginField>) {
    let key = field.painted_key.expect("actual painted field callback");
    key(
        field,
        &KeyDownEvent {
            keystroke: gpui::Keystroke::parse(chord).unwrap(),
            is_held: false,
        },
        window,
        cx,
    );
}

fn mac_press(field: &mut LoginField, chord: &str, cx: &mut Context<LoginField>) {
    // Same production handler as key(); explicit target, not OS-injected Cmd.
    field.key_for_platform(
        &KeyDownEvent {
            keystroke: gpui::Keystroke::parse(chord).unwrap(),
            is_held: false,
        },
        true,
        cx,
    );
}

fn clipboard(cx: &gpui::App) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn run(check: fn(&mut LoginField, &mut Window, &mut Context<LoginField>)) {
    assert_eq!(
        std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
        Ok("1")
    );
    gpui::Application::new().run(move |cx| {
        let view = cx
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
            view.update(cx, |field, window, cx| {
                assert!(field.layout.is_some(), "field must have painted");
                // Always clean up our private clipboard, even on assertion failure.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    check(field, window, cx)
                }));
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
                if let Err(error) = result {
                    std::panic::resume_unwind(error);
                }
            })
            .unwrap();
            cx.update(|cx| cx.quit()).unwrap();
        })
        .detach();
    });
}

#[test]
#[ignore = "requires private display"]
fn mac_target_handler_copies_and_cuts_exact_unicode_subselection() {
    run(|field, window, cx| {
        field.input.replace(None, PUBLIC);
        press(field, "left", window, cx);
        press(field, "shift-left", window, cx);
        assert_eq!(field.input.selection_utf16(), 1..3);
        for chord in ["cmd-c", "cmd-x"] {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
            mac_press(field, chord, cx);
            assert_eq!(clipboard(cx).as_deref(), Some("🔑"));
            assert_eq!(
                field.input.display_text(),
                if chord == "cmd-c" { PUBLIC } else { "aè" }
            );
        }
        mac_press(field, "cmd-a", cx);
        assert_eq!(field.input.selection_utf16(), 0..2);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(PUBLIC.into()));
        mac_press(field, "cmd-v", cx);
        assert_eq!(field.input.display_text(), PUBLIC);
        eprintln!(
            "Mac-target production handler exact Unicode selection/copy/cut/paste passed (not native Cmd)"
        );
    });
}

#[test]
#[ignore = "requires private display"]
fn mac_target_handler_pastes_protected_exactly_without_export_or_cut() {
    run(|field, _, cx| {
        field.input = LoginInput::secret();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(PUBLIC.into()));
        mac_press(field, "cmd-v", cx);
        assert_eq!(field.input.display_text(), "••••");
        assert!(
            field.input.take().as_str() == PUBLIC,
            "exact protected roundtrip, never formatted"
        );
        field.input.replace(None, PUBLIC);
        mac_press(field, "cmd-a", cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
        for chord in ["cmd-c", "cmd-x"] {
            mac_press(field, chord, cx);
            assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
            assert_eq!(field.input.display_text(), "••••");
            assert_eq!(field.input.selection_utf16(), 0..4);
        }
        assert!(field.input.take().as_str() == PUBLIC);
        eprintln!(
            "Mac-target protected exact/masked paste and no-export/no-cut passed (not native Cmd)"
        );
    });
}

#[test]
#[ignore = "requires private display"]
fn mac_target_handler_respects_disabled_marked_empty_and_multiline_guards() {
    run(|field, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
        for chord in ["cmd-c", "cmd-x"] {
            mac_press(field, chord, cx);
            assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
        }
        field.input.mark(None, PUBLIC, Some(0..4));
        for chord in ["cmd-a", "cmd-c", "cmd-x"] {
            mac_press(field, chord, cx);
            assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
            assert_eq!(field.input.marked_utf16(), Some(0..4));
            assert_eq!(field.input.selection_utf16(), 0..4);
            assert_eq!(field.input.display_text(), PUBLIC);
        }
        field.input.unmark();
        for protected in [false, true] {
            field.input = if protected {
                LoginInput::secret()
            } else {
                LoginInput::public()
            };
            field.input.replace(None, PUBLIC);
            mac_press(field, "cmd-a", cx);
            for chord in ["alt-cmd-a", "alt-cmd-v", "alt-cmd-c", "alt-cmd-x"] {
                mac_press(field, chord, cx);
                assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
                assert_eq!(field.input.selection_utf16(), 0..4);
            }
            field.enabled = false;
            for chord in ["cmd-a", "cmd-v", "cmd-c", "cmd-x"] {
                mac_press(field, chord, cx);
                assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
                assert_eq!(field.input.selection_utf16(), 0..4);
            }
            field.enabled = true;
            for text in ["one\ntwo", "one\r\ntwo"] {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into()));
                mac_press(field, "cmd-v", cx);
                assert_eq!(field.input.selection_utf16(), 0..4);
                assert!(field.input.take().as_str() == PUBLIC);
                field.input.replace(None, PUBLIC);
                mac_press(field, "cmd-a", cx);
            }
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
        }
        eprintln!("Mac-target disabled/marked/empty/Alt/multiline guards passed (not native Cmd)");
    });
}

#[test]
#[ignore = "requires private display"]
fn painted_ctrl_select_and_paste_preserve_exact_public_and_protected_input() {
    run(|field, window, cx| {
        for protected in [false, true] {
            field.input = if protected {
                LoginInput::secret()
            } else {
                LoginInput::public()
            };
            assert!(field.input.replace(None, "old synthetic value"));
            press(field, "ctrl-a", window, cx);
            assert_eq!(field.input.selection_utf16(), 0..19);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(PUBLIC.into()));
            press(field, "ctrl-v", window, cx);
            if protected {
                assert_eq!(field.input.display_text(), "••••");
                assert!(
                    field.input.take().as_str() == PUBLIC,
                    "exact synthetic value must survive protected paste"
                );
            } else {
                assert_eq!(field.input.display_text(), PUBLIC);
            }
        }
        eprintln!("painted Ctrl+A/V public+protected exact/masked passed");
    });
}

#[test]
#[ignore = "requires private display"]
fn painted_linux_cmd_is_not_a_command_and_ctrl_copy_cut_remain_unchanged() {
    run(|field, window, cx| {
        field.input.replace(None, PUBLIC);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(MARKER.into()));
        #[cfg(not(target_os = "macos"))]
        for chord in ["cmd-a", "cmd-v", "cmd-c", "cmd-x"] {
            press(field, chord, window, cx);
            assert_eq!(field.input.selection_utf16(), 4..4);
            assert_eq!(field.input.display_text(), PUBLIC);
        }
        press(field, "ctrl-a", window, cx);
        for chord in ["ctrl-c", "ctrl-x"] {
            press(field, chord, window, cx);
            assert_eq!(clipboard(cx).as_deref(), Some(MARKER));
            assert_eq!(field.input.display_text(), PUBLIC);
            assert_eq!(field.input.selection_utf16(), 0..4);
        }
        eprintln!("Linux Cmd exclusion and unchanged Ctrl copy/cut passed");
    });
}

#[test]
#[ignore = "requires private display"]
fn painted_disabled_and_multiline_paste_preserve_input_atomically() {
    run(|field, window, cx| {
        for protected in [false, true] {
            field.input = if protected {
                LoginInput::secret()
            } else {
                LoginInput::public()
            };
            field.input.replace(None, PUBLIC);
            press(field, "ctrl-a", window, cx);
            for text in ["line one\nline two", "line\r\nline", "line\tline"] {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into()));
                press(field, "ctrl-v", window, cx);
                assert_eq!(field.input.selection_utf16(), 0..4);
                assert!(field.input.marked_utf16().is_none());
                assert!(field.input.take().as_str() == PUBLIC);
                field.input.replace(None, PUBLIC);
                press(field, "ctrl-a", window, cx);
            }
            field.enabled = false;
            for chord in ["ctrl-v", "ctrl-a", "cmd-v", "cmd-x", "backspace"] {
                press(field, chord, window, cx);
                assert_eq!(field.input.selection_utf16(), 0..4);
            }
            assert!(field.input.take().as_str() == PUBLIC);
            field.enabled = true;
        }
        eprintln!("disabled/multiline guards passed");
    });
}
