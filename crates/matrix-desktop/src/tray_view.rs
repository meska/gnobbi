//! Wires the status icon into the window: menu clicks are drained here (never run UI code on
//! the tray's thread), and icon/menu follow unread, account, notifications and visibility.
use crate::Shell;
use gpui::{Context, Window};
use matrix_desktop::tray_menu::{CloseAction, TrayAction, close_action, total_unread, tray_menu};

impl Shell {
    /// «Riduci nel tray alla chiusura»: only with the setting on and a tray that really exists.
    pub(crate) fn close_hides_to_tray(&self) -> bool {
        let reachable = self.tray.as_ref().is_some_and(|t| t.host_present());
        close_action(self.prefs.close_to_tray, reachable) == CloseAction::HideToTray
    }

    pub(crate) fn sync_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tray) = &self.tray else { return };
        for action in tray.actions() {
            self.apply_tray_action(action, window, cx);
        }
        let accounts = self.drawer_accounts();
        let items = tray_menu(self.window_visible, self.notifications.enabled(), &accounts);
        let unread = total_unread(&self.account_counts());
        let picture = |mxc: &str| {
            let image = self.avatars.get(Some(mxc))?;
            let (size, mut rgba) = (image.size(0), image.as_bytes(0)?.to_vec());
            rgba.as_chunks_mut::<4>()
                .0
                .iter_mut()
                .for_each(|p| p.swap(0, 2)); // BGRA -> RGBA
            image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, rgba)
        };
        let pictures = matrix_desktop::tray::pictures_of(&accounts, picture);
        if let Some(tray) = &mut self.tray {
            tray.update(&items, unread, pictures);
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
                window.hide_window();
                self.window_visible = false;
            }
            TrayAction::ToggleWindow | TrayAction::ShowWindow => self.show_window(window, cx),
            TrayAction::ToggleNotifications => {
                self.notifications
                    .set_enabled(!self.notifications.enabled());
                if let Some(handle) = &self.notification_handle {
                    handle.cancel_pending();
                }
            }
            TrayAction::Quit => cx.quit(),
            TrayAction::SwitchAccount(id) => {
                self.show_window(window, cx);
                self.switch_account(id, window, cx);
            }
        }
        cx.notify();
    }

    fn show_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.activate(true); // macOS: the app itself, not only its window
        window.activate_window();
        self.window_visible = true;
    }
}
