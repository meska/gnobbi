//! Regression D1: settings.json unreadable -> «Notifiche» must not show ON without saving.
//! After a failed save the shown state must match the disk (OFF), or a one-line error must be in the Notifiche panel.
//! Entries: tray (`apply_tray_action` -> `set_notifications_enabled`, private to tray_view, covered through the shared
//! function) and panel click (`change_notification_preference(0)`). The painted tray menu and panel stay in the gui-tray-notif script.
//!   the ct script 'the gui-test-x11 script "live,linux-a11y" notification_save_failure'
use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        let mut fails: Vec<String> = vec![];
        let root = std::env::temp_dir().join(format!("d1-{}", std::process::id()));
        let dirs = matrix_desktop::AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("settings.json"), "not json at all").unwrap();
        for (entry, toggle) in [
            ("tray", (|s: &mut Shell, _: &mut Context<Shell>| s.set_notifications_enabled(!s.notifications.enabled())) as fn(&mut Shell, &mut Context<Shell>)),
            ("panel", |s: &mut Shell, cx: &mut Context<Shell>| s.change_notification_preference(0, cx)),
        ] {
            let (shown, error) = this.update_in(cx, |this, _, cx| {
                this.prefs_dirs = Some(dirs.clone());
                this.notifications.set_enabled(false);
                this.prefs.notifications = false;
                this.settings_error = None;
                toggle(this, cx);
                (this.notifications.enabled(), this.settings_error)
            }).unwrap();
            // The error is rendered only in Impostazioni, never in the Notifiche panel: the shown state must stay OFF like the disk.
            if shown {
                fails.push(format!("{entry}: save failed ({error:?}) but Notifiche shows ON; disk was not written"));
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        cx.update(|_, cx| cx.quit()).unwrap();
        FAILED.lock().unwrap().extend(fails);
    })
    .detach();
}

#[test]
#[ignore = "requires private display"]
fn notification_save_failure_keeps_state_consistent_with_disk() {
    assert_eq!(std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(), Ok("1"));
    ACTIVE.store(true, Ordering::SeqCst);
    crate::main();
    let failed = FAILED.lock().unwrap();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
