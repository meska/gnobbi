//! Menu button and side drawer: opens over the list, closes on outside click, Esc or the button;
//! focus moves into the drawer and returns to the button.
use crate::*;

pub(crate) const DRAWER_WIDTH: f32 = 280.;
/// Settings, Devices, Notifications, Add account, New chat, New room (after the account rows).
pub(crate) const DRAWER_ROWS: usize = 6;

impl Shell {
    pub(crate) fn set_menu_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        if open && let Some(dirs) = &self.prefs_dirs {
            self.accounts = matrix_desktop::AccountCatalog::load(dirs);
        }
        self.settings_open = false;
        self.confirm_removal = None;
        self.drawer_row = 0;
        if open {
            self.drawer_focus.focus(window)
        } else {
            self.menu_focus.focus(window)
        }
        cx.notify();
    }

    /// Closes the drawer, then lets `open` show a panel (which takes focus).
    pub(crate) fn open_from_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        open: fn(&mut Self, &mut Window),
    ) {
        self.set_menu_open(false, window, cx);
        open(self, window);
        cx.notify();
    }

    /// Records this window's account in the catalog once the SDK has told us who it is.
    /// The catalog is re-read first: another window may have added an account meanwhile.
    pub(crate) fn record_profile(&mut self) {
        let (Some(dirs), Some((root, _)), Some((user_id, name))) = (
            &self.prefs_dirs,
            &self.account_root,
            self.verification.profile(),
        ) else {
            return;
        };
        let mut catalog = matrix_desktop::AccountCatalog::load(dirs);
        let label = name.as_deref().unwrap_or_default();
        if catalog.record_login(dirs, root, user_id, label) && catalog.save(dirs).is_ok() {
            self.accounts = catalog.clone();
        }
        self.active_id = catalog.id_for_root(dirs, root);
    }

    /// Starts the sign-in for a new account in its own window and profile folder; this window
    /// and its account stay connected. The new window registers itself once it is signed in.
    #[cfg(unix)]
    pub(crate) fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launch_signin() {
            self.set_menu_open(false, window, cx);
        }
    }

    /// Spawns the sign-in window for the next free profile folder; false when it cannot start.
    #[cfg(unix)]
    pub(crate) fn launch_signin(&self) -> bool {
        let (Some(dirs), Some((_, ca_file))) = (&self.prefs_dirs, &self.account_root) else {
            return false;
        };
        let catalog = matrix_desktop::AccountCatalog::load(dirs);
        let root = catalog.next_root(dirs);
        if catalog.is_damaged() || matrix_desktop::create_private_dir(&root).is_err() {
            return false;
        }
        let mut command = std::process::Command::new(std::env::current_exe().unwrap_or_default());
        command.arg("--signin").arg(&root);
        if let Some(ca) = ca_file {
            command.arg("--ca-file").arg(ca);
        }
        let Ok(mut child) = command.spawn() else {
            return false;
        };
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        true
    }

    #[cfg(not(unix))]
    pub(crate) fn add_account(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    #[cfg(not(unix))]
    pub(crate) fn launch_signin(&self) -> bool {
        false
    }

    /// Runs before every other key handler; true when the key was consumed.
    pub(crate) fn menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        if self.connected && m.control && m.shift && key.key == "m" {
            self.set_menu_open(!self.menu_open, window, cx);
            return true;
        }
        if self.menu_open && self.confirm_removal.is_some() {
            match key.key.as_str() {
                "escape" => self.confirm_removal = None,
                "enter" => self.remove_account(window, cx),
                _ => return true,
            }
            cx.notify();
            return true;
        }
        if self.menu_open {
            let accounts = self.drawer_accounts();
            let activate = !m.modified() && matches!(key.key.as_str(), "enter" | "space");
            match key.key.as_str() {
                "escape" if self.settings_open => self.set_settings_open(false, window, cx),
                "escape" => self.set_menu_open(false, window, cx),
                "enter" | "space" if activate && self.settings_open => self.toggle_debug(cx),
                "enter" | "space" if activate => {
                    match self.drawer_row.checked_sub(accounts.len()) {
                        None => self.switch_account(accounts[self.drawer_row].id, window, cx),
                        Some(1) => self.open_from_drawer(window, cx, Shell::open_devices_panel),
                        Some(2) => {
                            self.open_from_drawer(window, cx, Shell::open_notifications_panel)
                        }
                        Some(3) if self.connected => self.add_account(window, cx),
                        Some(4) if self.connected => self.open_new_room(true, window, cx),
                        Some(5) if self.connected => self.open_new_room(false, window, cx),
                        _ => self.set_settings_open(true, window, cx),
                    }
                }
                "delete" if !self.settings_open && self.drawer_row < accounts.len() => {
                    self.confirm_removal = Some(accounts[self.drawer_row].id);
                    cx.notify();
                }
                // The drawer holds one focus stop; Up/Down/Tab move between its rows.
                "tab" if self.settings_open => self.settings_focus.focus(window),
                "up" | "down" | "tab" => {
                    let back = key.key == "up" || m.shift;
                    let rows = DRAWER_ROWS + accounts.len();
                    self.drawer_row = (self.drawer_row + if back { rows - 1 } else { 1 }) % rows;
                    self.drawer_focus.focus(window);
                    cx.notify();
                }
                _ => (),
            }
            return matches!(
                key.key.as_str(),
                "escape" | "tab" | "up" | "down" | "delete"
            ) || activate;
        }
        if !m.modified()
            && matches!(key.key.as_str(), "enter" | "space")
            && self.menu_focus.is_focused(window)
        {
            self.set_menu_open(true, window, cx);
            return true;
        }
        false
    }

    pub(crate) fn render_menu_button(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let line = || div().w(px(16.)).h(px(2.)).rounded_sm().bg(rgb(t.text));
        div()
            .id("menu-button")
            .relative()
            .children({
                #[cfg(test)]
                {
                    crate::sidebar_navigation_test::observe("menu-button".into())
                }
                #[cfg(not(test))]
                {
                    None::<gpui::AnyElement>
                }
            })
            .track_focus(&self.menu_focus)
            .w(px(36.))
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .rounded_lg()
            .cursor_pointer()
            .border_1()
            .border_color(rgb(if self.menu_focus.is_focused(window) {
                t.accent
            } else {
                t.surface
            }))
            .child(line())
            .child(line())
            .child(line())
            .on_click(
                cx.listener(|this, _, window, cx| this.set_menu_open(!this.menu_open, window, cx)),
            )
            .into_any_element()
    }

    /// Scrim and drawer cover the whole window (rail and list included); the scrim takes every click.
    pub(crate) fn render_drawer(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use gpui::{Animation, AnimationExt};
        div()
            .id("menu-scrim")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .top_0()
            .bg(gpui::rgba(0x00000066))
            .on_click(cx.listener(|this, _, window, cx| this.set_menu_open(false, window, cx)))
            .child(
                div()
                    .id("menu-drawer")
                    .track_focus(&self.drawer_focus)
                    .children({
                        #[cfg(test)]
                        {
                            crate::sidebar_navigation_test::observe("menu-drawer".into())
                        }
                        #[cfg(not(test))]
                        {
                            None::<gpui::AnyElement>
                        }
                    })
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .w(px(DRAWER_WIDTH))
                    .bg(rgb(t.surface))
                    .border_r_1()
                    .border_color(rgb(t.border))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .when(!self.settings_open, |d| {
                        d.children(self.render_drawer_header(t))
                    })
                    .child(div().flex_1().child(self.render_drawer_body(t, window, cx)))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .text_size(px(12.))
                            .text_color(rgb(t.secondary))
                            .child(concat!("Gnobbi ", env!("CARGO_PKG_VERSION"))),
                    )
                    .with_animation(
                        "menu-drawer-slide",
                        Animation::new(std::time::Duration::from_millis(140)),
                        |d, delta| d.left(px(-DRAWER_WIDTH * (1. - delta))),
                    ),
            )
            .children(self.render_removal_dialog(t, cx))
            .into_any_element()
    }

    /// Telegram-style profile block: initials avatar, name and Matrix ID, both from the SDK.
    fn render_drawer_header(&self, t: matrix_desktop::DesignTokens) -> Option<gpui::AnyElement> {
        let (id, name) = self.verification.profile()?;
        let name = name.clone().unwrap_or_else(|| id.clone());
        Some(
            div()
                .id("menu-profile")
                .p_4()
                .flex()
                .flex_col()
                .gap_1()
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .size(px(56.))
                        .mb_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(0x5288b5))
                        .text_size(px(20.))
                        .text_color(rgb(t.avatar_text))
                        .child(matrix_desktop::initials(&name)),
                )
                .child(
                    div()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(name.clone()),
                )
                .when(name != *id, |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(rgb(t.secondary))
                            .child(id.clone()),
                    )
                })
                .into_any_element(),
        )
    }
}
