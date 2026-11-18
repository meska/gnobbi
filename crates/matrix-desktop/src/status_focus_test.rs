//! Status rows with the real keyboard: F9 enters the names of the first row, Tab / Shift+Tab move between them, Enter and Space open
//! the focused user; the focused name is ringed (accent) and the a11y tree reports its bounds. Real key events (xdotool, or wtype under
//! sway), pixels read back in the Wayland run (Xvfb paints black):
//!   the ct script 'the gui-test-x11 script "live,linux-a11y" status_rows_real_keyboard_focus'
//!   the ct script 'the gui-test-wayland script "live,linux-a11y" status_rows_real_keyboard_focus'   (SHOT_DIR=..., STATUS_DARK=1 for the dark theme)
use crate::members_view::ProfileState;
use crate::rail_drag_test::{pause, Input};
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_domain::EventId;
use matrix_engine::operations::{OperationError, UserProfile};
use matrix_engine::{history::{HistoryEvent, HistorySender}, rich_text::{MentionKind, MentionSpan}};

fn row(id: &str, who: &str, body: &str, names: &[(&str, &str)]) -> HistoryEvent {
    HistoryEvent {
        poll: None, redacted: false, utd: false, event_id: EventId::new(id), body: body.into(), outgoing: false, timestamp_ms: 0,
        sender: HistorySender::new(who, None), video: None, audio: None, styles: Vec::new(),
        mentions: names.iter().map(|(n, u)| { let i = body.find(n).unwrap(); MentionSpan { range: i..i + n.len(), kind: MentionKind::User { sender: HistorySender::new(u, Some(n)), me: false } } }).collect(),
        highlighted: false, thread: None, thread_root: None, status: true, attachment_name: None, attachment_size: None, media: None,
    }
}

/// (focused, has bounds) of the name buttons of the first message in the a11y tree.
fn names(this: &mut Shell, window: &mut Window) -> Vec<(bool, bool)> {
    let snap = this.timeline.as_ref().unwrap().semantic_snapshot(&mut this.a11y_history, window);
    snap.messages.first().map(|m| m.pills.iter().map(|p| (p.focused, p.bounds.is_some())).collect()).unwrap_or_default()
}

/// Window-space centres of the name controls of the message whose text starts with `text`, and the avatar's centre (it sits
/// left of the first name: the row's avatar is not an a11y stop, so it is placed from the name's left edge, 6 px gap + half the avatar).
fn spots(this: &mut Shell, window: &mut Window, text: &str) -> (Vec<(i32, i32)>, (i32, i32)) {
    let snap = this.timeline.as_ref().unwrap().semantic_snapshot(&mut this.a11y_history, window);
    let m = snap.messages.iter().find(|m| m.text.starts_with(text)).unwrap_or_else(|| panic!("message {text:?}"));
    let b0 = m.pills[0].bounds.expect("bounds");
    let pills = m.pills.iter().map(|p| { let b = p.bounds.expect("bounds"); (((b.x0 + b.x1) / 2.) as i32, ((b.y0 + b.y1) / 2.) as i32) }).collect();
    (pills, (b0.x0 as i32 - 14, ((b0.y0 + b0.y1) / 2.) as i32))
}

async fn press(this: &WeakEntity<Shell>, cx: &mut AsyncWindowContext, ex: &BackgroundExecutor, input: &Input, tag: &str, key: &str, shift: bool, want: [(bool, bool); 2], shot: &str) {
    if shift { input.chord("shift", key) } else { input.key(key) }
    pause(ex, 400).await;
    let got = this.update_in(cx, |this, window, _| names(this, window)).unwrap();
    assert_eq!(got, want, "after {}{key}", if shift { "Shift+" } else { "" });
    input.shot(tag, shot);
}

/// The card the app itself opened (Loading) for `who` is answered with a synthetic profile (this fixture has no engine), then shot.
async fn card(this: &WeakEntity<Shell>, cx: &mut AsyncWindowContext, ex: &BackgroundExecutor, input: &Input, tag: &str, who: &str, answer: Result<UserProfile, OperationError>, what: &str) {
    let mut loading = false;
    for _ in 0..30 {
        pause(ex, 100).await;
        loading = this.update_in(cx, |this, _, _| matches!(&this.profile, Some(ProfileState::Loading(u)) if u == who)).unwrap();
        if loading { break; }
        // The fixture's own loop may not drive requests: do what that loop does (main.rs), once, and say so.
        if !loading { this.update_in(cx, |this, _, cx| this.drive_profile(cx)).unwrap(); }
    }
    assert!(loading, "{what}: the app itself opened {who}'s card (Loading)");
    this.update_in(cx, |this, _, cx| this.profile_found(who.into(), answer, cx)).unwrap();
    pause(ex, 600).await;
    assert!(this.update_in(cx, |this, _, _| this.room_details_open && this.profile.is_some()).unwrap(), "{what}: the card is open");
    input.shot(tag, &format!("status-card-{what}"));
}

async fn close(this: &WeakEntity<Shell>, cx: &mut AsyncWindowContext, ex: &BackgroundExecutor) {
    this.update_in(cx, |this, _, cx| { this.profile = None; this.pending_operation_command = None; this.set_room_details_open(false); cx.notify(); }).unwrap();
    pause(ex, 300).await;
}

fn profile(id: &str, name: &str) -> UserProfile { UserProfile { user_id: id.into(), name: name.into(), avatar: None, verified: Some(true), ignored: false } }

fn click(input: &mut Input, at: (i32, i32)) { input.mv(at.0, at.1); input.button(true); input.button(false); }

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, ex: BackgroundExecutor) {
    let (anna, bea, ghost) = ("@anna:fixture.invalid", "@bea:fixture.invalid", "@ghost:fixture.invalid");
    let tag = if std::env::var("STATUS_DARK").as_deref() == Ok("1") { "dark" } else { "light" };
    pause(&ex, 500).await;
    this.update_in(cx, |this, window, cx| {
        if tag == "dark" { this.set_theme(Theme::Dark, cx); }
        crate::link_jump_test::open_room(this, vec![
            row("$invite", anna, "Anna ha invitato Bea", &[("Anna", anna), ("Bea", bea)]),
            row("$left", ghost, "Ghost è uscito dalla stanza", &[("Ghost", ghost)]),
            row("$made", anna, "Anna ha creato questa stanza.", &[("Anna", anna)]),
        ], false, cx);
        let id = matrix_domain::ScopedTimelineId { scope: this.active_scope.clone(), id: matrix_domain::TimelineId::Server(EventId::new("$invite")) };
        assert!(this.timeline.as_ref().unwrap().focus_semantic_message(&id, window), "the first row takes the cursor");
        cx.notify();
    }).unwrap();
    pause(&ex, 600).await;
    let mut input = Input::new();
    let state = this.update_in(cx, |this, window, _| names(this, window)).unwrap();
    assert_eq!(state, [(false, true), (false, true)], "painted names report bounds before any key");
    input.shot(tag, "status-0-nofocus");
    press(&this, cx, &ex, &input, tag, "F9", false, [(true, true), (false, true)], "status-1-f9-actor").await;
    press(&this, cx, &ex, &input, tag, "Tab", false, [(false, true), (true, true)], "status-2-tab-target").await;
    press(&this, cx, &ex, &input, tag, "Tab", true, [(true, true), (false, true)], "status-3-shift-tab-actor").await;
    press(&this, cx, &ex, &input, tag, "Tab", false, [(false, true), (true, true)], "status-2-tab-target").await;
    // Keyboard: the real key opens the card, and the shot is taken after the card is on screen.
    for (key, label) in [("Return", "enter"), ("space", "space")] {
        this.update_in(cx, |this, _, _| { this.profile = None; this.pending_operation_command = None; }).unwrap();
        press(&this, cx, &ex, &input, tag, key, false, [(false, true), (true, true)], &format!("status-4-{label}-focus-before-card")).await;
        card(&this, cx, &ex, &input, tag, bea, Ok(profile(bea, "Bea")), &format!("key-{label}")).await;
        close(&this, cx, &ex).await;
    }
    // Real pointer over the invite row (actor + target): pointing hand over names and the actor's avatar (grim -c paints the cursor).
    let (n, av) = this.update_in(cx, |this, window, _| spots(this, window, "Anna ha invitato Bea")).unwrap();
    for (what, at) in [("actor-name", n[0]), ("target-name", n[1]), ("avatar", av)] {
        input.mv(at.0, at.1);
        pause(&ex, 500).await;
        input.shot_cursor(tag, &format!("status-hover-{what}"));
        // Second shot after a 1 px move inside the same control: tells a late cursor-shape update from a control that never asks for the hand.
        input.mv(at.0 + 1, at.1);
        pause(&ex, 500).await;
        input.shot_cursor(tag, &format!("status-hover-{what}-moved"));
    }
    for (what, at, who, name) in [("target-name", n[1], bea, "Bea"), ("actor-name", n[0], anna, "Anna"), ("avatar", av, anna, "Anna")] {
        click(&mut input, at);
        card(&this, cx, &ex, &input, tag, who, Ok(profile(who, name)), &format!("click-{what}")).await;
        close(&this, cx, &ex).await;
    }
    // The user who left: the name still opens a card; the engine cannot answer for them → the failed row.
    let (g, _) = this.update_in(cx, |this, window, _| spots(this, window, "Ghost è uscito")).unwrap();
    input.mv(g[0].0, g[0].1);
    pause(&ex, 500).await;
    input.shot_cursor(tag, "status-left-hover-name");
    click(&mut input, g[0]);
    card(&this, cx, &ex, &input, tag, ghost, Err(OperationError::Unavailable), "left-card-failed").await;
    close(&this, cx, &ex).await;
    input.shot(tag, "status-6-after-close");
    this.update_in(cx, |_, _, cx| cx.quit()).unwrap();
}
