//! «Modifica stanza» > Argomento (multi-line field), with REAL X11 key events (xdotool) on the private display:
//!   the ct script 'the gui-test-x11 script "" room_topic_multiline'
//! Load/save round trip of a 3-line topic, empty field = removal, long paste (wrap, max height, inner scroll),
//! Tab inserts no tab, Esc cancels, Enter is a new line (Ctrl+Enter saves). Every failure is collected.
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_engine::operations::OperationCommand;

fn xdotool(args: &[&str]) {
    assert!(std::process::Command::new("xdotool").args(args).status().unwrap().success(), "xdotool {args:?}");
}

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    let win = String::from_utf8(std::process::Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &std::process::id().to_string()]).output().unwrap().stdout)
        .unwrap()
        .lines()
        .next()
        .expect("window")
        .to_string();
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
    macro_rules! text {
        ($i:expr) => { shell!(|this, _, cx| this.room_edit.as_ref().map(|e| e.fields[$i].read(cx).input.display_text())).unwrap_or_default() };
    }
    macro_rules! focused {
        ($i:expr) => { shell!(|this, window, cx| this.room_edit.as_ref().is_some_and(|e| e.fields[$i].read(cx).focus.is_focused(window))) };
    }
    macro_rules! is_open {
        () => { shell!(|this, _, _| this.room_edit.is_some()) };
    }
    macro_rules! expect {
        ($tag:expr, $what:expr, $got:expr, $want:expr) => {{
            let got: String = $got;
            if got != $want {
                fails.push(format!("{} {}: {got:?}, atteso {:?}", $tag, $what, $want));
            }
        }};
    }
    let scope = shell!(|this, _, _| this.sidebar.selected().map(|r| r.scope.clone())).expect("a selected room");
    // Opens the dialog with the topic focused; `original` is what the room "already has".
    macro_rules! open {
        ($original:expr) => {{
            shell!(|this, window, cx| {
                this.composer.update(cx, |v, cx| v.focus_from_accessibility(window, cx));
                this.pending_operation_command = None;
                this.set_room_details_open(true);
                if this.sidebar.selected().is_none() {
                    this.sidebar.select(&scope);
                }
                this.open_room_edit(window, cx);
                if let Some(edit) = this.room_edit.as_mut() {
                    edit.set_original_topic($original);
                    let topic = edit.fields[1].clone();
                    topic.update(cx, |f, _| { f.input.take(); f.input.replace(None, $original); });
                    topic.read(cx).focus.focus(window);
                }
            });
            wait(500).await;
        }};
    }
    // The topic of the EditRoom the dialog queued (None = no command at all; Some(None) = name only).
    macro_rules! queued {
        () => { shell!(|this, _, _| match &this.pending_operation_command {
            Some(OperationCommand::EditRoom { name, topic, .. }) => Ok((name.clone(), topic.clone())),
            other => Err(other.is_some()),
        }) };
    }
    macro_rules! reset {
        () => {{
            shell!(|this, window, cx| {
                this.room_edit = None;
                this.pending_operation_command = None;
                this.set_room_details_open(false);
                this.list_focus.focus(window);
                cx.notify();
            });
            wait(200).await;
        }};
    }

    // 1. Load + save: a 3-line topic with leading/inner spaces comes back identical, trailing space/newlines go.
    let load = "  uno \ndue\ntre";
    {
        let tag = "[carica/salva 3 righe]";
        open!(load);
        if !is_open!() {
            fails.push(format!("{tag} il dialogo non si apre"));
        } else {
            expect!(tag, "caricamento", text!(1), load);
            key!("ctrl+Return");
            match queued!() {
                Err(false) => {}
                other => fails.push(format!("{tag} senza modifiche non deve partire nessun comando: {other:?}")),
            }
            reset!();
            open!(load);
            // Append trailing junk: only the end is trimmed, the head and inner spaces stay.
            typ!("  ");
            key!("Return");
            typ!(" ");
            expect!(tag, "digitazione in coda", text!(1), format!("{load}  \n "));
            key!("ctrl+Return");
            // Saved text == loaded text once the tail is trimmed: nothing to send.
            if queued!() != Err(false) {
                fails.push(format!("{tag} spazi solo in coda: nessun comando atteso, trovato {:?}", queued!()));
            }
            reset!();
            open!(load);
            typ!("X");
            key!("ctrl+Return");
            if queued!() != Ok((None, Some("  uno \ndue\ntreX".to_string()))) {
                fails.push(format!("{tag} dopo una modifica: {:?}", queued!()));
            }
        }
        reset!();
    }

    // 2. Typing 3 lines from scratch: Enter = newline (dialog stays open, nothing sent), Tab inserts no tab.
    {
        let tag = "[Invio/Tab]";
        open!("");
        typ!("uno");
        key!("Return");
        typ!("due");
        key!("Return");
        typ!("tre");
        expect!(tag, "Invio inserisce a capo", text!(1), "uno\ndue\ntre");
        if !is_open!() || queued!() != Err(false) {
            fails.push(format!("{tag} Invio nell'Argomento non deve salvare né chiudere"));
        }
        key!("Tab");
        expect!(tag, "Tab non inserisce tab", text!(1), "uno\ndue\ntre");
        if focused!(1) {
            fails.push(format!("{tag} Tab non sposta il focus fuori dall'Argomento"));
        }
        key!("shift+Tab");
        if !focused!(1) {
            fails.push(format!("{tag} Shift+Tab non riporta all'Argomento"));
        }
        key!("ctrl+Return");
        if queued!() != Ok((None, Some("uno\ndue\ntre".to_string()))) {
            fails.push(format!("{tag} salvataggio: {:?}", queued!()));
        }
        reset!();
    }

    // 3. Empty field (also only blanks) on a room that had a topic: the topic is removed (sent as "").
    for (what, keys) in [("campo svuotato", false), ("solo spazi e a capo", true)] {
        let tag = format!("[argomento vuoto: {what}]");
        open!("vecchio\nargomento");
        key!("ctrl+a", "BackSpace");
        if keys {
            typ!("  ");
            key!("Return");
            typ!(" ");
        }
        key!("ctrl+Return");
        if queued!() != Ok((None, Some(String::new()))) {
            fails.push(format!("{tag} atteso topic Some(\"\"), trovato {:?}", queued!()));
        }
        reset!();
    }

    // 4. Esc cancels from the topic: nothing is queued and the dialog closes.
    {
        let tag = "[Esc]";
        open!("originale");
        typ!(" modificato");
        key!("Escape");
        if is_open!() {
            fails.push(format!("{tag} Esc non chiude il dialogo"));
        }
        if queued!() != Err(false) {
            fails.push(format!("{tag} Esc non deve accodare comandi: {:?}", queued!()));
        }
        reset!();
    }

    // 5. Long paste: CR/LF normalised, wraps, box capped at ~6 lines (154 px), text scrolls inside.
    {
        let tag = "[incolla lungo]";
        open!("");
        let crlf = "a\r\nb\r\nc";
        shell!(|_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(crlf.into())));
        key!("ctrl+v");
        expect!(tag, "incolla con CRLF", text!(1), "a\nb\nc");
        key!("ctrl+a", "BackSpace");
        macro_rules! measure {
            () => { shell!(|this, _, cx| this.room_edit.as_ref().and_then(|e| e.fields[1].read(cx).painted())) };
        }
        // Three lines: the box is the minimum height, no wrapping needed.
        typ!("uno");
        key!("Return");
        typ!("due");
        key!("Return");
        typ!("tre");
        wait(300).await;
        match measure!() {
            Some((b, _)) if (f32::from(b.size.height) - 80.).abs() <= 1. => {}
            other => fails.push(format!("{tag} 3 righe: altezza box {:?}, attesa 80 (82 con bordo)", other.map(|(b, _)| b.size.height))),
        }
        let long: String = (0..120).map(|i| format!("parola{i} ")).collect::<String>().trim_end().to_owned();
        shell!(|_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(long.clone())));
        key!("ctrl+a", "ctrl+v");
        wait(400).await;
        expect!(tag, "testo incollato per intero", text!(1), long);
        match measure!() {
            Some((b, t)) => {
                let (bh, th, bw, tw) = (f32::from(b.size.height), f32::from(t.size.height), f32::from(b.size.width), f32::from(t.size.width));
                if th <= 6. * 24. {
                    fails.push(format!("{tag} il testo non va a capo su piu' di 6 righe (altezza testo {th})"));
                }
                if !(bh > 80. && bh <= 152.5) {
                    fails.push(format!("{tag} altezza box {bh}: attesa 152 (154 con bordo, max ~6 righe)"));
                }
                if tw > bw {
                    fails.push(format!("{tag} il testo esce in larghezza: {tw} > {bw}"));
                }
                // Inner scroll: wheel over the field moves the text up, the box keeps its size.
                let (cx0, cy0) = (f32::from(b.origin.x + b.size.width / 2.), f32::from(b.origin.y + b.size.height / 2.));
                let top = f32::from(t.origin.y);
                xdotool(&["mousemove", "--window", &win, &format!("{}", cx0 as i32), &format!("{}", cy0 as i32)]);
                xdotool(&["click", "--repeat", "3", "--delay", "60", "5"]);
                wait(400).await;
                match measure!() {
                    Some((b2, t2)) => {
                        if f32::from(t2.origin.y) >= top {
                            fails.push(format!("{tag} la rotella non scorre il testo nel campo (y {top} -> {})", f32::from(t2.origin.y)));
                        }
                        if (f32::from(b2.size.height) - bh).abs() > 1. {
                            fails.push(format!("{tag} lo scroll cambia l'altezza del box"));
                        }
                    }
                    None => fails.push(format!("{tag} campo non dipinto dopo lo scroll")),
                }
            }
            None => fails.push(format!("{tag} campo non dipinto")),
        }
        reset!();
    }

    for f in &fails {
        println!("ROOM-TOPIC FAIL {f}");
    }
    println!("ROOM-TOPIC: {} problemi", fails.len());
    shell!(|_, _, cx| cx.quit());
    assert!(fails.is_empty(), "{} problemi:\n{}", fails.len(), fails.join("\n"));
}
