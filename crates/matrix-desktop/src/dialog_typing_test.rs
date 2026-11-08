//! Typing in the dialogs that hold text fields («Modifica stanza», «Nuova stanza», «Rinomina» dispositivo,
//! «Crea sondaggio»), with REAL X11 key events (xdotool/XTEST) on the private display:
//!   the ct script 'the gui-test-x11 script "" dialog_typing_modifica_stanza'   (also _nuova_stanza, _rinomina_dispositivo, _crea_sondaggio)
//! One process per dialog (the fixture quits the app at the end). Every step is checked and every failure is
//! collected, so the output lists all that is wrong, not only the first. Opening paths that exist in the code:
//! «Modifica» in the info panel (panel open) and the same call with the panel closed. The room's right-click
//! menu only offers «Segna come letto/non letto» (room_menu_view::ITEMS): it has no edit entry, so no path.
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_engine::operations::OperationCommand;

type Open = fn(&mut Shell, bool, &mut Window, &mut Context<Shell>);

struct Case {
    name: &'static str,
    /// Opening paths: (label, panel open first).
    paths: &'static [(&'static str, bool)],
    open: Open,
    fields: fn(&Shell, &gpui::App) -> Vec<gpui::Entity<crate::login_field::LoginField>>,
    is_open: fn(&Shell) -> bool,
    /// Called before Enter; may fill what the dialog needs besides the two fields under test.
    prepare: fn(&mut Shell, &mut gpui::App),
    /// After Enter with name = `a`, second field = `b`: Err(what was found) when the submit is wrong.
    submitted: fn(&Shell, &str, &str) -> Result<(), String>,
    /// Only the first field is under test (no second text field to Tab to).
    single: bool,
}

const PANEL: &[(&str, bool)] = &[("pannello info aperto", true), ("pannello info chiuso", false)];
const DIRECT: &[(&str, bool)] = &[("apertura diretta", false)];

fn case(n: usize) -> Case {
    match n {
        92 => Case {
            name: "Modifica stanza",
            paths: PANEL,
            open: |s, panel, w, cx| {
                if panel {
                    s.set_room_details_open(true);
                }
                s.open_room_edit(w, cx);
            },
            fields: |s, _| s.room_edit.as_ref().map(|e| e.fields.clone()).unwrap_or_default(),
            is_open: |s| s.room_edit.is_some(),
            prepare: |_, _| {},
            submitted: |s, a, b| match &s.pending_operation_command {
                Some(OperationCommand::EditRoom { name, topic, .. }) if name.as_deref() == Some(a) && topic.as_deref() == Some(b) => Ok(()),
                Some(OperationCommand::EditRoom { name, topic, .. }) => Err(format!("EditRoom name={name:?} topic={topic:?}")),
                other => Err(format!("{:?}", other.as_ref().map(|_| "altro comando"))),
            },
            single: false,
        },
        93 => Case {
            name: "Nuova stanza",
            paths: DIRECT,
            open: |s, _, w, cx| s.open_new_room(false, w, cx),
            fields: |s, _| s.new_room.as_ref().map(|e| e.fields.clone()).unwrap_or_default(),
            is_open: |s| s.new_room.is_some(),
            prepare: |_, _| {},
            submitted: |s, a, b| match &s.pending_operation_command {
                Some(OperationCommand::CreateRoom { name, topic, .. }) if name == a && topic.as_deref() == Some(b) => Ok(()),
                Some(OperationCommand::CreateRoom { name, topic, .. }) => Err(format!("CreateRoom name={name:?} topic={topic:?}")),
                other => Err(format!("{:?}", other.as_ref().map(|_| "altro comando"))),
            },
            single: false,
        },
        94 => Case {
            name: "Rinomina dispositivo",
            paths: DIRECT,
            open: |s, _, w, cx| {
                let account_id = s.active_scope.account_id;
                s.verification.apply(matrix_engine::verification::VerificationEvent::OwnDevice {
                    account_id,
                    device: matrix_engine::verification::VerificationDevice {
                        user_id: "@me:fixture".into(),
                        device_id: "DEV".into(),
                        verified: true,
                        display_name: Some("Portatile".into()),
                        last_seen_ts: None,
                        last_seen_ip: None,
                    },
                });
                // The dialog is drawn inside the devices panel: without it the field is never painted.
                s.set_verification_open(true);
                s.open_device_rename(w, cx);
            },
            fields: |s, _| s.delete_field.iter().cloned().collect(),
            is_open: |s| s.verification.rename_dialog().is_some() && s.delete_field.is_some(),
            prepare: |_, _| {},
            submitted: |s, _, _| match s.verification.rename_dialog() {
                Some(d) if d.busy => Ok(()),
                other => Err(format!("rename busy={:?}", other.map(|d| d.busy))),
            },
            single: true,
        },
        _ => Case {
            name: "Crea sondaggio",
            paths: DIRECT,
            open: |s, _, w, cx| s.open_poll_dialog(w, cx),
            fields: |s, cx| s.poll_dialog.as_ref().map(|d| d.read(cx).fields.clone()).unwrap_or_default(),
            is_open: |s| s.poll_dialog.is_some(),
            // Needs two options to be valid: the third field is filled directly (not under test).
            prepare: |s, cx| {
                if let Some(f) = (s.poll_dialog.as_ref()).map(|d| d.read(cx).fields[2].clone()) {
                    f.update(cx, |f, _| f.input.replace(None, "seconda"));
                }
            },
            // A valid draft emits Create and the dialog closes (no send route in this fixture).
            submitted: |s, _, _| if s.poll_dialog.is_none() { Ok(()) } else { Err("il dialogo e' rimasto aperto con bozza valida".into()) },
            single: false,
        },
    }
}

fn xdotool(args: &[&str]) {
    assert!(std::process::Command::new("xdotool").args(args).status().unwrap().success(), "xdotool {args:?}");
}

pub(crate) async fn scenario(n: usize, this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout)
        .unwrap()
        .lines()
        .next()
        .expect("window")
        .to_string();
    let c = case(n);
    let mut fails: Vec<String> = Vec::new();
    wait(800).await;
    let _ = std::process::Command::new("xdotool").args(["windowfocus", "--sync", &win]).status();
    macro_rules! key {
        ($($k:expr),+) => {{ xdotool(&["key", "--delay", "40", $($k),+]); wait(150).await; }};
    }
    macro_rules! typ {
        ($t:expr) => {{ xdotool(&["type", "--delay", "40", $t]); wait(150).await; }};
    }
    macro_rules! shell {
        ($body:expr) => { this.update_in(cx, $body).unwrap() };
    }

    for (label, panel) in c.paths {
        for run in ["digitazione+Invio", "Esc"] {
            let tag = format!("[{} / {label} / {run}]", c.name);
            shell!(|this, window, cx| {
                this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx));
                (c.open)(this, *panel, window, cx);
            });
            wait(500).await;
            if !shell!(|this, _, _| (c.is_open)(this)) {
                fails.push(format!("{tag} il dialogo non si apre"));
                continue;
            }
            // Current text of field `i`, and whether it holds the focus.
            macro_rules! text {
                ($i:expr) => { shell!(|this, _, cx| (c.fields)(this, cx).get($i).map(|f| f.read(cx).input.display_text())).unwrap_or_default() };
            }
            macro_rules! focused {
                ($i:expr) => { shell!(|this, window, cx| (c.fields)(this, cx).get($i).is_some_and(|f| f.read(cx).focus.is_focused(window))) };
            }
            macro_rules! expect {
                ($what:expr, $i:expr, $want:expr) => {{
                    let got: String = text!($i);
                    if got != $want {
                        fails.push(format!("{tag} {}: campo {} = {got:?}, atteso {:?}", $what, $i, $want));
                    }
                }};
            }
            if !focused!(0) {
                fails.push(format!("{tag} il primo campo non ha il focus all'apertura"));
            }
            if run == "Esc" {
                let before = text!(0);
                typ!("abc");
                expect!("digitazione prima di Esc", 0, format!("{before}abc"));
                key!("Escape");
                if shell!(|this, _, _| (c.is_open)(this)) {
                    fails.push(format!("{tag} Esc non chiude il dialogo"));
                    shell!(|this, window, cx| { this.room_edit = None; this.new_room = None; this.poll_dialog = None; this.verification.cancel_rename(); this.delete_field = None; this.list_focus.focus(window); cx.notify(); });
                }
                // The panel opened for this path must not leak into the next "pannello chiuso" run.
                shell!(|this, _, _| { this.set_room_details_open(false); this.set_verification_open(false); });
                continue;
            }

            // First field: type, delete, caret, select-all, copy/cut/paste, selection replace.
            key!("ctrl+a");
            typ!("Alfa");
            expect!("selezionare tutto e digitare", 0, "Alfa");
            typ!(" Beta");
            expect!("digitazione", 0, "Alfa Beta");
            key!("BackSpace");
            expect!("Backspace", 0, "Alfa Bet");
            key!("Left", "Left", "Delete");
            expect!("frecce + Canc", 0, "Alfa Bt");
            key!("Home");
            typ!("X");
            key!("End");
            typ!("Y");
            expect!("Home/End", 0, "XAlfa BtY");
            key!("ctrl+a", "ctrl+c");
            let clip = shell!(|_, _, cx| cx.read_from_clipboard().and_then(|i| i.text()));
            if clip.as_deref() != Some("XAlfa BtY") {
                fails.push(format!("{tag} Ctrl+A,Ctrl+C: appunti = {clip:?}, atteso \"XAlfa BtY\""));
            }
            expect!("copia non modifica il testo", 0, "XAlfa BtY");
            key!("ctrl+x");
            expect!("taglia", 0, "");
            shell!(|_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("Incolla".into())));
            key!("ctrl+a", "ctrl+v");
            expect!("incolla sopra la selezione", 0, "Incolla");
            key!("shift+Left", "shift+Left");
            typ!("ZZ");
            expect!("selezione con shift+frecce sostituita", 0, "IncolZZ");

            // The same four commands as the Edit menu sends them on macOS (Cmd+A/C/X/V), without OS key injection.
            macro_rules! menu {
                ($i:expr) => { shell!(|_, window, cx| window.dispatch_action(crate::editing_command::tests::menu_action($i), cx)) };
            }
            menu!(3);
            menu!(1);
            wait(150).await;
            let clip = shell!(|_, _, cx| cx.read_from_clipboard().and_then(|i| i.text()));
            if clip.as_deref() != Some("IncolZZ") {
                fails.push(format!("{tag} menu Modifica > Seleziona tutto + Copia: appunti = {clip:?}, atteso \"IncolZZ\""));
            }
            menu!(0);
            wait(150).await;
            expect!("menu Modifica > Taglia", 0, "");
            menu!(2);
            wait(150).await;
            expect!("menu Modifica > Incolla", 0, "IncolZZ");

            // Second field: Tab moves there, Shift+Tab comes back, and each keeps its own text.
            if !c.single {
                key!("Tab");
                if !focused!(1) {
                    fails.push(format!("{tag} Tab dal primo campo non porta al secondo"));
                }
                key!("ctrl+a");
                typ!("Argo");
                expect!("secondo campo: digitazione", 1, "Argo");
                key!("BackSpace");
                expect!("secondo campo: Backspace", 1, "Arg");
                key!("ctrl+a", "ctrl+v");
                expect!("secondo campo: incolla", 1, "IncolZZ");
                key!("shift+Tab");
                if !focused!(0) {
                    fails.push(format!("{tag} Shift+Tab non riporta al primo campo"));
                }
                typ!("!");
                expect!("primo campo dopo il giro Tab", 0, "IncolZZ!");
                expect!("il secondo campo non e' toccato", 1, "IncolZZ");
            } else {
                key!("Tab", "shift+Tab");
                if !focused!(0) {
                    fails.push(format!("{tag} Tab + Shift+Tab non tornano al campo"));
                }
                typ!("!");
                expect!("campo dopo il giro Tab", 0, "IncolZZ!");
            }

            // Enter submits what is in the fields.
            let (a, b) = (text!(0), if c.single { String::new() } else { text!(1) });
            shell!(|this, _, cx| (c.prepare)(this, cx));
            key!("Return");
            let verdict = shell!(|this, _, _| (c.submitted)(this, &a, &b));
            if let Err(found) = verdict {
                fails.push(format!("{tag} Invio/Salva non applica i valori digitati ({a:?}, {b:?}): {found}"));
            }
            shell!(|this, window, cx| {
                this.room_edit = None; this.new_room = None; this.poll_dialog = None;
                this.pending_operation_command = None;
                this.verification.cancel_rename(); this.delete_field = None;
                this.set_room_details_open(false);
                this.set_verification_open(false);
                this.list_focus.focus(window);
                cx.notify();
            });
            wait(200).await;
        }
    }

    for f in &fails {
        println!("DIALOG-TYPING FAIL {f}");
    }
    println!("DIALOG-TYPING {}: {} problemi", c.name, fails.len());
    // Quit before asserting, so a failure does not leave this window to the next test in the batch.
    shell!(|_, _, cx| cx.quit());
    assert!(fails.is_empty(), "{}: {} problemi:\n{}", c.name, fails.len(), fails.join("\n"));
}
