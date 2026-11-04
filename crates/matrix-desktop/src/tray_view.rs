//! Wires the status icon into the window: menu clicks are drained here (never run UI code on
//! the tray's thread), and icon/menu follow unread, account, notifications and visibility.
use crate::Shell;
use gpui::{Context, Window};
use matrix_desktop::tray_menu::{TrayAction, total_unread, tray_menu};

impl Shell {
    /// «Riduci nel tray alla chiusura»: only with the setting on and a tray that really exists.
    pub(crate) fn close_hides_to_tray(&self) -> bool {
        self.prefs.close_to_tray && self.tray.is_some()
    }

    pub(crate) fn sync_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tray) = &self.tray else { return };
        for action in tray.actions() {
            self.apply_tray_action(action, window, cx);
        }
        let items = tray_menu(
            self.window_visible,
            self.notifications.enabled(),
            &self.drawer_accounts(),
        );
        let unread = total_unread(&self.account_counts());
        if let Some(tray) = &mut self.tray {
            tray.update(&items, unread, self.system_dark);
        }
    }

    fn apply_tray_action(
        &mut self,
        action: TrayAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            TrayAction::ToggleWindow if self.window_visible => {
                window.minimize_window();
                self.window_visible = false;
            }
            TrayAction::ToggleWindow => self.show_window(window),
            TrayAction::ToggleNotifications => {
                self.notifications
                    .set_enabled(!self.notifications.enabled());
                if let Some(handle) = &self.notification_handle {
                    handle.cancel_pending();
                }
            }
            TrayAction::Quit => cx.quit(),
            TrayAction::SwitchAccount(id) => {
                self.show_window(window);
                self.switch_account(id, window, cx);
            }
        }
        cx.notify();
    }

    fn show_window(&mut self, window: &mut Window) {
        window.activate_window();
        self.window_visible = true;
    }
}
