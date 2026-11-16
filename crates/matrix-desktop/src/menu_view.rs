//! Menu button and side drawer: opens over the list, closes on outside click, Esc or the button;
//! focus moves into the drawer and returns to the button.
use crate::*;

pub(crate) const DRAWER_WIDTH: f32 = 280.;

/// Full-window scrim (click = `on_close`) holding `panel` as a left drawer that slides in; shared by the menu and the rail editor.
pub(crate) fn drawer_shell(
    scrim_id: &'static str,
    panel: gpui::Stateful<gpui::Div>,
    t: matrix_desktop::DesignTokens,
    on_close: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    use gpui::{Animation, AnimationExt};
    div().id(scrim_id).occlude().absolute().inset_0().bg(gpui::rgba(matrix_desktop::SCRIM)).on_click(on_close).child(
        panel
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
            .with_animation(
                "drawer-slide",
                Animation::new(std::time::Duration::from_millis(140)),
                |d, delta| d.left(px(-DRAWER_WIDTH * (1. - delta))),
            ),
    )
}
/// Full-window scrim (click = `on_close`) with `dialog` centred, `width` px wide; shared by the device dialogs and the verification popup.
/// Keys and focus stay with the caller.
pub(crate) fn dialog_shell(
    layer_id: &'static str,
    dialog: gpui::Stateful<gpui::Div>,
    width: f32,
    t: matrix_desktop::DesignTokens,
    on_close: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div().id(layer_id).occlude().absolute().inset_0().flex().items_center().justify_center()
        .bg(gpui::rgba(matrix_desktop::SCRIM))
        .on_click(on_close)
        .child(
            dialog.w(px(width)).max_w_full().p_4().gap_2().flex().flex_col().rounded_lg()
                .bg(rgb(t.surface)).border_1().border_color(rgb(t.border))
                .on_click(|_, _, cx| cx.stop_propagation()),
        )
}
/// Settings, Devices, Notifications, Add account, New chat, New room, Join, Quick search, version line (after the account rows).
pub(crate) const DRAWER_ROWS: usize = 9;

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
        self.privacy_open = false;
        self.about_open = false;
        self.confirm_removal = None;
        self.switch_error = None;
        // A stored outcome replaces the update row here, once: it is cleared as it is read.
        self.update_error = self.prefs_dirs.as_ref().filter(|_| open).and_then(matrix_desktop::update::startup::take_outcome).map(|(r, _)| {
            matrix_desktop::update::outcome_line(&r, matrix_desktop::build_info::BUILD_INFO.version)
        });
        // Initial focus goes to the shown account, not to the first catalog row.
        self.drawer_row = self.drawer_accounts().iter().position(|a| a.current).unwrap_or(0);
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

    /// The open drawer for the semantic tree; the settings page keeps the plain page tree.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_drawer(
        &self,
    ) -> Option<matrix_desktop::accessibility::drawer::DrawerSnapshot> {
        use matrix_desktop::accessibility::drawer::{DrawerRow, DrawerSnapshot};
        if !self.menu_open || self.settings_open {
            return None;
        }
        let accounts = self.drawer_accounts();
        let confirm = accounts
            .iter()
            .find(|a| Some(a.id) == self.confirm_removal)
            .map(|a| (a.id, a.name.clone()));
        let rows = accounts
            .into_iter()
            .map(|a| DrawerRow {
                switchable: self.can_switch_to(a.id),
                id: a.id,
                name: a.name,
                user_id: a.user_id,
                unread: a.unread,
                current: a.current,
            })
            .collect();
        Some(DrawerSnapshot { rows, confirm, connected: self.connected, update: matrix_desktop::update::announced() })
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_drawer(
        &mut self,
        action: matrix_desktop::accessibility::drawer::DrawerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::drawer::{DrawerAction, DrawerItem};
        if !self.semantic_drawer().is_some_and(|d| d.allows(&action)) {
            return;
        }
        match action {
            DrawerAction::Switch(id) => self.switch_account(id, window, cx),
            DrawerAction::AskRemove(id) => self.confirm_removal = Some(id),
            DrawerAction::ConfirmRemove => self.remove_account(window, cx),
            DrawerAction::CancelRemove => self.confirm_removal = None,
            DrawerAction::UpdateNow => self.update_now(cx),
            DrawerAction::UpdateLater => self.update_later(cx),
            DrawerAction::Close => self.set_menu_open(false, window, cx),
            DrawerAction::Item(item) => match item {
                DrawerItem::Settings => self.set_settings_open(true, window, cx),
                DrawerItem::Devices => self.open_from_drawer(window, cx, Shell::open_devices_panel),
                DrawerItem::Notifications => {
                    self.open_from_drawer(window, cx, Shell::open_notifications_panel)
                }
                DrawerItem::AddAccount => self.add_account(window, cx),
                DrawerItem::NewChat => self.open_new_room(true, window, cx),
                DrawerItem::NewRoom => self.open_new_room(false, window, cx),
                DrawerItem::About => self.set_about_open(true, window, cx),
            },
        }
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
        self.active_id = catalog
            .id_for_root(dirs, root)
            .or_else(|| catalog.id_for_user(user_id));
        if let Some(id) = self.active_id
            && catalog.record_avatar(id, self.verification.profile_avatar())
            && catalog.save(dirs).is_ok()
        {
            self.accounts = catalog;
        }
    }

    /// Starts the sign-in for a new account in its own window and profile folder; this window
    /// and its account stay connected. The new window registers itself once it is signed in.
    pub(crate) fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launch_signin() {
            self.set_menu_open(false, window, cx);
        }
    }

    /// Spawns the sign-in window for the next free profile folder; false when it cannot start.
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

    /// Runs before every other key handler; true when the key was consumed.
    pub(crate) fn menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        #[cfg(target_os = "linux")]
        if !m.modified() && self.install_offer_key(&key.key, cx) {
            return true;
        }
        if self.connected && m.control && m.shift && key.key == "m" {
            self.set_menu_open(!self.menu_open, window, cx);
            return true;
        }
        if self.connected
            && m.secondary()
            && !m.shift
            && !m.alt
            && key.key == "k"
            && self.new_room.is_none()
            && self.room_edit.is_none()
            && self.groups_edit.is_none()
            && self.confirm_removal.is_none()
        {
            if self.quick.is_some() {
                self.close_quick_search(window, cx)
            } else {
                self.open_quick_search(window, cx)
            }
            return true;
        }
        if self.quick.is_some() {
            return self.quick_key(event, window, cx);
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
        if self.menu_open && self.about_open {
            return self.about_key(event, window, cx);
        }
        if self.menu_open {
            let accounts = self.drawer_accounts();
            let update = matrix_desktop::update::announced().is_some() && !self.settings_open;
            let activate = !m.modified() && matches!(key.key.as_str(), "enter" | "space");
            match key.key.as_str() {
                "escape" if self.privacy_open => {
                    self.privacy_open = false;
                    cx.notify()
                }
                "escape" if self.settings_open => self.set_settings_open(false, window, cx),
                // The privacy page has one stop, «Dispositivi».
                "enter" | "space" if activate && self.privacy_open => {
                    self.swallow_menu_key_up = true;
                    self.open_from_drawer(window, cx, Shell::open_devices_panel)
                }
                "up" | "down" | "tab" if self.privacy_open => {}
                "escape" => self.set_menu_open(false, window, cx),
                "enter" | "space" if activate && self.settings_open => {
                    self.toggle_setting(self.settings_row, cx)
                }
                // Settings keep one focus stop; Up/Down/Tab (Shift+Tab backwards) walk its four switches.
                "up" | "down" | "tab" if self.settings_open => {
                    let back = key.key == "up" || m.shift;
                    let ring = self.settings_ring();
                    let at = ring.iter().position(|r| *r == self.settings_row).unwrap_or(0);
                    self.settings_row = ring[(at + if back { ring.len() - 1 } else { 1 }) % ring.len()];
                    self.settings_focus.focus(window);
                    cx.notify();
                }
                "enter" | "space" if activate => {
                    // The key-up of this press would click the menu button we hand focus back to.
                    self.swallow_menu_key_up = true;
                    match self.drawer_row.checked_sub(accounts.len()) {
                        None => self.switch_account(accounts[self.drawer_row].id, window, cx),
                        Some(1) => self.open_from_drawer(window, cx, Shell::open_devices_panel),
                        Some(2) => {
                            self.open_from_drawer(window, cx, Shell::open_notifications_panel)
                        }
                        Some(3) if self.connected => self.add_account(window, cx),
                        Some(4) if self.connected => self.open_new_room(true, window, cx),
                        Some(5) if self.connected => self.open_new_room(false, window, cx),
                        Some(6) if self.connected => self.open_join_room(window, cx),
                        Some(7) if self.connected => self.open_quick_search(window, cx),
                        Some(8) => self.set_about_open(true, window, cx),
                        Some(9) if update => self.update_now(cx),
                        Some(10) if update => self.update_later(cx),
                        _ => self.set_settings_open(true, window, cx),
                    }
                }
                "delete" if !self.settings_open && self.drawer_row < accounts.len() => {
                    self.confirm_removal = Some(accounts[self.drawer_row].id);
                    cx.notify();
                }
                // The drawer holds one focus stop; Up/Down/Tab move between its rows.
                "up" | "down" | "tab" => {
                    let back = key.key == "up" || m.shift;
                    let rows = DRAWER_ROWS + accounts.len() + if update { 2 } else { 0 };
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
        if key.key == "tab"
            && !m.control
            && !m.alt
            && !m.platform
            && self.menu_focus.is_focused(window)
            && !(self.notifications_open || self.verification_open)
        {
            self.menu_tab(m.shift, window, cx);
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
            .track_focus(&self.menu_focus)
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
            .children({
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    Some(self.space_observer(matrix_desktop::accessibility::MENU, cx))
                }
                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                {
                    None::<gpui::AnyElement>
                }
            })
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
            .border_color(rgb(if input_modality::only_keys(self.menu_focus.is_focused(window), cx) {
                t.accent
            } else {
                t.surface
            }))
            .child(line())
            .child(line())
            .child(line())
            .when(matrix_desktop::update::announced().is_some(), |d| {
                d.child(div().absolute().top(px(4.)).right(px(4.)).size(px(8.)).rounded_full().bg(rgb(t.accent)))
            })
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
        drawer_shell(
            "menu-scrim",
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
                .when(!self.settings_open && !self.about_open, |d| d.children(self.render_drawer_header(t)))
                // Scrolls so that Settings stays reachable on short windows (150%); the footer stays put.
                .child(div().id("drawer-body").flex_1().min_h_0().overflow_y_scroll().child(self.render_drawer_body(t, window, cx)))
                .children(self.render_update_row(t, window, cx))
                .when(!self.about_open, |d| d.child(self.render_version_line(t, window, cx))),
            t,
            cx.listener(|this, _, window, cx| this.set_menu_open(false, window, cx)),
        )
            .children(self.render_removal_dialog(t, cx))
            .into_any_element()
    }

    /// «Aggiornamento disponibile · <versione>» (or the one-line failure) with its two actions, above the version line.
    fn render_update_row(&self, t: matrix_desktop::DesignTokens, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.settings_open || self.about_open {
            return None;
        }
        let version = matrix_desktop::update::announced();
        if version.is_none() && self.update_error.is_none() {
            return None;
        }
        let n = self.drawer_accounts().len();
        let ring = input_modality::ring(&self.drawer_focus, window, cx);
        let action = |id: &'static str, label: &'static str, i: usize| {
            div()
                .id(id)
                .flex_1()
                .px_2()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(rgb(if ring && self.drawer_row == i { t.accent } else { t.surface }))
                .text_size(px(12.))
                .text_color(rgb(t.text))
                .hover(|s| s.bg(rgb(t.border)))
                .cursor_pointer()
                .child(label)
        };
        Some(
            div()
                .id("menu-update")
                .flex_shrink_0()
                .mx_2()
                .px_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(t.accent))
                        .child(self.update_error.clone().or_else(|| version.as_deref().map(matrix_desktop::update::indicator_row)).unwrap_or_default()),
                )
                .when(version.is_some(), |d| d.child(
                    div()
                        .flex()
                        .gap_1()
                        .child(action("menu-update-now", "Aggiorna ora", n + 9).on_click(cx.listener(|this, _, _, cx| this.update_now(cx))))
                        .child(action("menu-update-later", "Più tardi", n + 10).on_click(cx.listener(|this, _, _, cx| this.update_later(cx)))),
                ))
                .into_any_element(),
        )
    }

    /// «Aggiorna ora»: swap and restart; on failure the returned line is shown as is.
    /// shortcut: runs on the UI thread (a few seconds), move it off when the swap gets slower.
    fn update_now(&mut self, cx: &mut Context<Self>) {
        use matrix_desktop::update::{channel::{Env, detect_channel}, install::install_and_restart};
        let ready = matrix_desktop::update::shared_state().lock().unwrap_or_else(|e| e.into_inner()).ready.clone();
        let (Some(dirs), Some(ready)) = (self.prefs_dirs.clone(), ready) else { return };
        let unsent = self.composer.read(cx).has_unsent()
            || self.delivery.borrow().has_unsent()
            || self.parked.values().any(|a| a.delivery.has_unsent());
        self.update_error = Some(matrix_desktop::update::install_unless_unsent(unsent, || {
            install_and_restart(&dirs, &detect_channel(&Env::from_process()), &ready)
        }));
        cx.notify();
    }

    fn update_later(&mut self, cx: &mut Context<Self>) {
        matrix_desktop::update::dismiss_indicator();
        self.update_error = None;
        self.drawer_row = self.drawer_row.min(DRAWER_ROWS + self.drawer_accounts().len() - 1);
        cx.notify();
    }

    /// First frame: confirm a just-installed build off the UI thread (it may sleep 2 s), then
    /// repaint every 30 s so a freshly staged update shows up without user input.
    pub(crate) fn start_update_watch(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.update_confirmed, true) {
            return;
        }
        std::thread::spawn(matrix_desktop::update::startup::confirm_update);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(30)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// «Gnobbi 0.1.0» at the bottom: looks clickable only on hover or keyboard focus.
    fn render_version_line(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let focused = input_modality::ring(&self.drawer_focus, window, cx)
            && self.drawer_row == self.drawer_accounts().len() + 8;
        div()
            .id("menu-about")
            .flex_shrink_0()
            .mx_2()
            .my_1()
            .px_2()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if focused { t.accent } else { t.surface }))
            .text_size(px(12.))
            .text_color(rgb(t.secondary))
            .hover(|s| s.bg(rgb(t.border)).text_color(rgb(t.text)))
            .child(format!("Gnobbi {}", matrix_desktop::build_info::BUILD_INFO.version))
            .on_click(cx.listener(|this, _, window, cx| this.set_about_open(true, window, cx)))
            .into_any_element()
    }

    /// Telegram-style profile block: picture (initials as fallback), name and Matrix ID, both from the SDK.
    fn render_drawer_header(&self, t: matrix_desktop::DesignTokens) -> Option<gpui::AnyElement> {
        let (id, name) = self.verification.profile()?;
        let name = name.clone().unwrap_or_else(|| id.clone());
        Some(
            div()
                .id("menu-profile")
                .flex_shrink_0()
                .p_4()
                .flex()
                .flex_col()
                .gap_1()
                .border_b_1()
                .border_color(rgb(t.border))
                .child(match self.avatars.get(self.verification.profile_avatar()) {
                    Some(image) => div()
                        .mb_2()
                        .child(avatar_view::round(image, 56.))
                        .into_any_element(),
                    None => div()
                        .size(px(56.))
                        .mb_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(matrix_desktop::AVATAR_FALLBACK))
                        .text_size(px(20.))
                        .text_color(rgb(t.avatar_text))
                        .child(matrix_desktop::initials(&name))
                        .into_any_element(),
                })
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

impl Shell {
    /// Ring stop before the rail (wide) or the list header (compact): forward enters the ring, backward leaves it.
    fn menu_tab(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let wide = window.viewport_size().width >= px(800.);
        match (wide, shift) {
            (true, false) => {
                self.rail_ui.on_unread = true;
                self.reveal_rail_target(window, cx);
                self.rail_ui.focus.focus(window);
            }
            (false, false) => self.first_list_stop().focus(window),
            (true, true) => {
                if !self.focus_last_chat_control(window, cx) {
                    self.list_focus.focus(window);
                }
            }
            (false, true) if self.connected && self.sidebar.spaces().next().is_some() => {
                self.compact_spaces_tab(true, window, cx);
            }
            (false, true) => {
                if !(self.connected
                    && self.focus_opener(matrix_desktop::panel_openers::Control::Notifications, window, cx))
                {
                    self.list_focus.focus(window);
                }
            }
        }
        cx.notify();
    }
}
