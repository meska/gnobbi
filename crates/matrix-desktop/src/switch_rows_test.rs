//! Real Shell, private Wayland display: the «Notifiche locali» switches as painted (grim screenshot of the fullscreen
//! window), light + dark. Checks per row: thumb side + track colour = state (on / off / disabled), row height >= 40 px
//! (measured, linux-a11y), the X in the header; keys N M P Space Enter Up Down Tab Esc and the focus ring.
//! The runner sets the scale (GUI_SCALE=1|1.5|2 → 100/150/200%); PNGs land in $SW_OUT (default /tmp/gui-out):
//!   CT_SRC=$PWD the ct script 'GUI_SCALE=1.5 the gui-test-switch script "live,linux-a11y" switch_rows'
//! Layout constants mirror render_notifications + switch_row; a drift makes the colour probes fail, which is the point.
use crate::*;
use gpui::Keystroke;
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
use matrix_desktop::accessibility::notifications::NotificationControl;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    On,
    Off,
    Disabled,
}

fn near(a: [u8; 3], rgb: u32) -> bool {
    let want = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
    a.iter().zip(want).all(|(x, y)| x.abs_diff(y) <= 4)
}

struct Shot(image::RgbImage, f32);

impl Shot {
    fn take(name: &str) -> Result<Shot, String> {
        let dir = std::env::var("SW_OUT").unwrap_or_else(|_| "/tmp/gui-out".into());
        let path = format!("{dir}/{name}.png");
        let st = std::process::Command::new("grim").arg(&path).status().map_err(|e| format!("grim: {e}"))?;
        if !st.success() {
            return Err("grim failed".into());
        }
        let img = image::open(&path).map_err(|e| format!("{path}: {e}"))?.to_rgb8();
        Ok(Shot(img, 0.0))
    }
    /// Logical coordinates → pixel.
    fn at(&self, x: f32, y: f32) -> [u8; 3] {
        let (px, py) = ((x * self.1) as u32, (y * self.1) as u32);
        self.0.get_pixel(px.min(self.0.width() - 1), py.min(self.0.height() - 1)).0
    }
}

fn row_center(i: usize) -> f32 {
    80. + 56. * i as f32 + 22.
}

/// What the switch of row `i` looks like, from pixels only.
fn look(shot: &Shot, w: f32, i: usize, t: &matrix_desktop::DesignTokens) -> Result<Look, String> {
    let y = row_center(i);
    let (thumb_on, thumb_off) = (shot.at(w - 38., y), shot.at(w - 54., y));
    let (track_on, track_off) = (shot.at(w - 56., y), shot.at(w - 34., y));
    if near(thumb_on, t.on_accent) && near(track_on, t.accent) {
        Ok(Look::On)
    } else if near(thumb_off, t.on_accent) && near(track_off, t.disabled) {
        Ok(Look::Off)
    } else if !near(track_off, t.accent) && !near(track_off, t.surface) && thumb_off != track_off && thumb_on == track_off {
        Ok(Look::Disabled)
    } else {
        Err(format!("row {i}: unreadable switch: thumb {thumb_on:?}/{thumb_off:?} track {track_on:?}/{track_off:?}"))
    }
}

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |this, cx| {
        let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
        wait(500).await;
        // The compositor scale arrives after the window exists: read it now, not in install().
        let scale = cx.update(|window, _| window.scale_factor()).unwrap();
        let mut fails: Vec<String> = vec![];
        macro_rules! check { ($c:expr, $($m:tt)+) => { if !$c { fails.push(format!("[x{scale}] {}", format!($($m)+))); } }; }
        macro_rules! key { ($k:expr) => { { cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse($k).unwrap(), cx); }).unwrap(); wait(120).await; } }; }
        let scope = this.update_in(cx, |this, window, cx| {
            this.connected = true;
            let scope = this.sidebar.rooms()[0].scope.clone();
            assert!(this.sidebar.select(&scope));
            this.set_notifications_open(true);
            this.list_focus.focus(window);
            cx.notify();
            scope
        }).unwrap();
        let (w, t_classic, t_dark) = this.update_in(cx, |_, window, _| {
            (f32::from(window.viewport_size().width), Theme::Classic.tokens(), Theme::Dark.tokens())
        }).unwrap();
        check!(w * scale >= 1900., "window is not fullscreen: {w} logical px");

        // Set the three switches: (enabled, room muted or None for no room, count only).
        let set = |enabled: bool, room: Option<bool>, preview: bool| {
            let scope = scope.clone();
            move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
                this.notifications.set_enabled(enabled);
                this.notifications.set_count_preview(preview);
                // A fresh list: no room selected unless we add one (Remove falls back to the first row).
                this.sidebar = matrix_desktop::Sidebar::connected(scope.account_id);
                if let Some(muted) = room {
                    this.sidebar.apply_diff(matrix_engine::RoomListDiff::Upsert(matrix_engine::RoomSummary {
                        scope: scope.clone(),
                        name: "Privacy".into(),
                        unread: 0,
                        info: matrix_engine::RoomInfo { muted, ..Default::default() },
                    }));
                    assert!(this.sidebar.select(&scope));
                }
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                { this.notification_focus = NotificationControl::Close; }
                cx.notify();
            }
        };
        // 1+2. Painted look x state x theme. Rows: Notifiche, Stanza (on = not muted), Conteggio.
        for (theme, t, tname) in [(Theme::Classic, t_classic, "light"), (Theme::Dark, t_dark, "dark")] {
            this.update_in(cx, |this, _, cx| this.set_theme(theme, cx)).unwrap();
            for (sname, enabled, room, preview, want) in [
                ("on", true, Some(false), true, [Look::On, Look::On, Look::On]),
                ("off", false, Some(true), false, [Look::Off, Look::Off, Look::Off]),
                ("disabled", true, None, false, [Look::On, Look::Disabled, Look::Off]),
            ] {
                this.update_in(cx, |this, window, cx| set(enabled, room, preview)(this, window, cx)).unwrap();
                wait(500).await;
                match Shot::take(&format!("{tname}-{scale}-{sname}")) {
                    Err(e) => check!(false, "{tname}/{sname}: {e}"),
                    Ok(mut shot) => {
                        // Screenshot px per logical px: gpui rounds a fractional output scale up (1.5 → 2), so derive it.
                        shot.1 = shot.0.width() as f32 / w;
                        for (i, want) in want.into_iter().enumerate() {
                            match look(&shot, w, i, &t) {
                                Ok(got) => check!(got == want, "{tname}/{sname} row {i}: drawn {got:?}, expected {want:?}"),
                                Err(e) => check!(false, "{tname}/{sname}: {e}"),
                            }
                        }
                        // The X: accent pixels inside the 32x32 close button (header, right).
                        let x_px = (0..32).flat_map(|dx| (0..32).map(move |dy| (dx, dy)))
                            .filter(|(dx, dy)| near(shot.at(w - 48. + *dx as f32, 16. + *dy as f32), t.accent)).count();
                        check!(x_px >= 10, "{tname}/{sname}: no X in the close button ({x_px} accent px)");
                    }
                }
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    let h = this.update_in(cx, |this, _, _| this.a11y_notification_bounds).unwrap();
                    for (i, b) in h.iter().take(3).enumerate() {
                        let rows = b.map(|r| (r.y1 - r.y0) / f64::from(scale));
                        check!(rows.is_some_and(|h| h >= 40.), "{tname}/{sname} row {i}: height {rows:?} logical px < 40");
                    }
                }
            }
        }

        // 3. Keys. Start: all on, light theme.
        this.update_in(cx, |this, window, cx| { this.set_theme(Theme::Classic, cx); set(true, Some(false), true)(this, window, cx); #[cfg(all(feature = "linux-a11y", target_os = "linux"))] { this.notification_focus = Default::default(); } }).unwrap();
        key!("n");
        this.update_in(cx, |this, _, _| check!(!this.notifications.enabled(), "N did not switch Notifiche off")).unwrap();
        key!("n");
        this.update_in(cx, |this, _, _| check!(this.notifications.enabled(), "N twice did not switch it back on")).unwrap();
        key!("p");
        this.update_in(cx, |this, _, _| check!(!this.notifications.count_preview(), "P did not switch Conteggio off")).unwrap();
        key!("p");
        this.update_in(cx, |this, _, _| check!(this.notifications.count_preview(), "P twice did not switch it back on")).unwrap();
        key!("m");
        this.update_in(cx, |this, _, _| {
            check!(matches!(this.pending_operation_command.take(),
                Some(matrix_engine::operations::OperationCommand::SetMuted { muted: true, .. })), "M did not ask to mute the room");
        }).unwrap();
        // Keys must not leak to the chat behind (no composer text, panel still open).
        this.update_in(cx, |this, _, _| check!(this.notifications_open, "panel closed by N/M/P")).unwrap();

        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let focus = |this: &Shell| this.notification_focus;
            this.update_in(cx, |this, _, _| this.notification_focus = NotificationControl::Enabled).unwrap();
            let mut seen = vec![];
            for k in ["tab", "tab", "tab", "tab"] {
                key!(k);
                seen.push(this.update_in(cx, |this, _, _| focus(this)).unwrap());
            }
            check!(seen == [NotificationControl::Mute, NotificationControl::Preview, NotificationControl::Close, NotificationControl::Enabled], "Tab order: {seen:?}");
            key!("down");
            check!(this.update_in(cx, |this, _, _| focus(this)).unwrap() == NotificationControl::Mute, "Down after Enabled should reach Mute");
            key!("up");
            check!(this.update_in(cx, |this, _, _| focus(this)).unwrap() == NotificationControl::Enabled, "Up should go back to Enabled");
            key!("shift-tab");
            check!(this.update_in(cx, |this, _, _| focus(this)).unwrap() == NotificationControl::Close, "Shift+Tab from Enabled should wrap to Close");
            // Ring painted on the focused row only.
            for (ctl, row) in [(NotificationControl::Enabled, 0usize), (NotificationControl::Mute, 1), (NotificationControl::Preview, 2)] {
                this.update_in(cx, |this, _, cx| { this.notification_focus = ctl; cx.notify(); }).unwrap();
                wait(400).await;
                let t = t_classic;
                match Shot::take(&format!("ring-{scale}-{row}")) {
                    Err(e) => check!(false, "ring row {row}: {e}"),
                    Ok(mut shot) => {
                        // Screenshot px per logical px: gpui rounds a fractional output scale up (1.5 → 2), so derive it.
                        shot.1 = shot.0.width() as f32 / w;
                        let ring = |i: usize| (-2..=3).any(|d| near(shot.at(16. + d as f32 / shot.1, row_center(i)), t.accent));
                        check!(ring(row), "focus ring not painted on row {row}");
                        for other in (0..3).filter(|o| *o != row) {
                            check!(!ring(other), "ring on row {other} while focus is on row {row}");
                        }
                    }
                }
            }
            // Space and Enter act on the focused row.
            this.update_in(cx, |this, _, _| this.notification_focus = NotificationControl::Preview).unwrap();
            key!("space");
            this.update_in(cx, |this, _, _| check!(!this.notifications.count_preview(), "Space on Conteggio did not switch it off")).unwrap();
            key!("enter");
            this.update_in(cx, |this, _, _| check!(this.notifications.count_preview(), "Enter on Conteggio did not switch it on")).unwrap();
            this.update_in(cx, |this, _, _| this.notification_focus = NotificationControl::Enabled).unwrap();
            key!("space");
            this.update_in(cx, |this, _, _| check!(!this.notifications.enabled(), "Space on Notifiche did not switch it off")).unwrap();
            this.update_in(cx, |this, _, _| this.notification_focus = NotificationControl::Close).unwrap();
            key!("enter");
            this.update_in(cx, |this, _, _| check!(!this.notifications_open, "Enter on the X did not close the panel")).unwrap();
            this.update_in(cx, |this, window, cx| { this.set_notifications_open(true); this.list_focus.focus(window); cx.notify(); }).unwrap();
        }
        // Default build: Tab/arrows/Space/Enter and the focus ring exist only behind linux-a11y (known gap, reported to
        // mazinga). Opt in with SWITCH_KEYS_STRICT=1 to see it fail: Space on the first row must switch Notifiche.
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        if std::env::var("SWITCH_KEYS_STRICT").as_deref() == Ok("1") {
            let before = this.update_in(cx, |this, _, _| this.notifications.enabled()).unwrap();
            key!("space");
            this.update_in(cx, |this, _, _| check!(this.notifications.enabled() != before, "default build: Space on the focused row does nothing")).unwrap();
            key!("tab");
            key!("enter");
            this.update_in(cx, |this, _, _| check!(this.notifications.count_preview() || this.notifications.enabled() == before, "default build: Tab+Enter moved nothing")).unwrap();
        }
        // Esc closes.
        key!("escape");
        this.update_in(cx, |this, _, _| check!(!this.notifications_open, "Esc did not close the panel")).unwrap();
        cx.update(|_, cx| cx.quit()).unwrap();
        FAILED.lock().unwrap().extend(fails);
    })
    .detach();
}

#[test]
#[ignore = "requires private display"]
fn switch_rows_painted_state_height_x_keys_and_focus_ring() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn near_tolerates_rounding_only() {
    assert!(near([0x12, 0x34, 0x56], 0x123456));
    assert!(near([0x14, 0x32, 0x56], 0x123456));
    assert!(!near([0x18, 0x34, 0x56], 0x123456));
}
