#![recursion_limit = "256"]
//! Gnobbi, egui frontend. S1: shell with the saved theme and accent, window identity and the
//! single-instance gate shared with the GPUI app (same profile, never two processes on it).
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, ViewportBuilder, Visuals};
use matrix_desktop::{
    AppDirs, DesignTokens, FormStatus, Platform, Preferences, Theme,
    app_id::{APP_ID, TITLE},
};
use matrix_engine::RoomSyncStatus;
use std::time::Duration;

mod accounts;
mod pick;
mod composer;
mod devices;
mod info;
mod list;
mod newchat;
mod poll;
mod roomedit;
mod session;
mod settings;
mod timeline;
use session::Session;

/// Avatar fallback colour (identity, not theme).
pub const AVATAR: u32 = matrix_desktop::AVATAR_FALLBACK;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// egui visuals from the same tokens the GPUI app paints with.
fn visuals(t: &DesignTokens, dark: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = rgb(t.timeline);
    v.window_fill = rgb(t.surface);
    v.extreme_bg_color = rgb(t.input);
    v.faint_bg_color = rgb(t.surface);
    v.override_text_color = Some(rgb(t.text));
    v.hyperlink_color = rgb(t.accent);
    v.selection.bg_fill = rgb(t.selection);
    v.selection.stroke = Stroke::new(1., rgb(t.text));
    v.error_fg_color = rgb(t.error);
    v.widgets.noninteractive.bg_fill = rgb(t.surface);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1., rgb(t.border));
    v.widgets.inactive.bg_fill = rgb(t.input);
    v.widgets.inactive.weak_bg_fill = rgb(t.input);
    v.widgets.hovered.bg_fill = rgb(t.selected);
    v.widgets.hovered.weak_bg_fill = rgb(t.selected);
    v.widgets.active.bg_fill = rgb(t.accent);
    v.widgets.active.weak_bg_fill = rgb(t.accent);
    v.widgets.open.bg_fill = rgb(t.selected);
    v.window_corner_radius = CornerRadius::same(10);
    v
}

/// An account that is not on screen: its profile folder and its live session.
struct Parked {
    root: std::path::PathBuf,
    session: Session,
    recorded: bool,
}

struct App {
    prefs: Preferences,
    dirs: Option<AppDirs>,
    settings_open: bool,
    accounts_open: bool,
    /// Profile folder this window runs on, and whether its login is already in the catalog.
    root: std::path::PathBuf,
    recorded: bool,
    /// The other accounts of the catalog, running at the same time as the shown one (own worker, own notifications).
    parked: Vec<Parked>,
    trust: Option<Vec<u8>>,
    /// A parked account to bring forward at the end of the frame (a click on one of its notifications).
    wake: Option<usize>,
    tray: Option<matrix_desktop::tray::Tray>,
    quitting: bool,
    window_visible: bool,
    new_form: newchat::Form,
    device_fields: devices::Fields,
    poll_form: poll::Form,
    confirm: Option<&'static str>,
    theme: Theme,
    tokens: DesignTokens,
    /// What `visuals` was last built for: rebuilt only when it changes.
    applied: Option<(Theme, u32)>,
    session: Option<Session>,
    /// Test hook: GNOBBI_EGUI_SHOT=<file.png> saves what the window paints after a few frames and exits.
    shot: Option<(std::path::PathBuf, std::time::Instant)>,
    /// Test hook: GNOBBI_EGUI_LOGIN=<json {homeserver,user,password,phrase?}> signs in by itself (disposable servers only).
    auto_login: Option<serde_json::Value>,
    shot_sent: bool,
    pending_link: Option<(matrix_domain::MatrixLink, std::time::Instant)>,
    hook_sent: bool,
    hook_added: bool,
    auto_login2: Option<serde_json::Value>,
    hook_type: bool,
    hook_alias: bool,
    alias_input: String,
    hook_keys: bool,
    hook_menu_at: Option<std::time::Instant>,
    /// Why the backend could not start, shown instead of the form.
    start_error: Option<String>,
}

impl App {
    fn new(dirs: Option<&AppDirs>, root: std::path::PathBuf, trust: Option<Vec<u8>>, link: Option<matrix_domain::MatrixLink>, multi: bool) -> App {
        let prefs = dirs.map(Preferences::load).unwrap_or_else(Preferences::default);
        let theme = Theme::Classic;
        let (session, start_error) = match dirs.map(|d| Session::start(d, root.clone(), trust.clone())) {
            Some(Ok(s)) => (Some(s), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, Some("Cartella dati non disponibile".to_owned())),
        };
        // The default window runs every account of the catalog at once; `--signin <folder>` runs just that one.
        let mut parked = Vec::new();
        if multi && let Some(d) = dirs {
            let catalog = matrix_desktop::AccountCatalog::load(d);
            for entry in catalog.accounts() {
                if let Some(dir) = catalog.data_dir(d, entry.id)
                    && dir != root
                    && let Ok(session) = Session::start(d, dir.clone(), trust.clone())
                {
                    parked.push(Parked { root: dir, session, recorded: true });
                }
            }
        }
        App { prefs, dirs: dirs.cloned(), settings_open: false, accounts_open: false, root: root.clone(), recorded: false, parked, trust: trust.clone(), wake: None, tray: matrix_desktop::tray::Tray::start(), quitting: false, window_visible: true, new_form: Default::default(), device_fields: Default::default(), poll_form: Default::default(), confirm: None, theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, shot: std::env::var_os("GNOBBI_EGUI_SHOT").map(|p| (p.into(), std::time::Instant::now())),
            pending_link: link.map(|l| (l, std::time::Instant::now())), shot_sent: false, hook_sent: false, hook_added: false, auto_login2: std::env::var_os("GNOBBI_EGUI_LOGIN2").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), hook_type: false, hook_alias: false, alias_input: String::new(), hook_keys: false, hook_menu_at: None,
            auto_login: std::env::var_os("GNOBBI_EGUI_LOGIN").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), session, start_error }
    }

    /// Brings parked account `i` on screen; the shown one takes its place in the list.
    fn switch_to(&mut self, i: usize) {
        let Some(p) = self.parked.get_mut(i) else { return };
        if let Some(shown) = &mut self.session {
            std::mem::swap(shown, &mut p.session);
            std::mem::swap(&mut self.root, &mut p.root);
            std::mem::swap(&mut self.recorded, &mut p.recorded);
            // The window states of the account that left must not follow the new one.
            self.new_form = Default::default();
            self.device_fields = Default::default();
            self.poll_form = Default::default();
            self.confirm = None;
            self.settings_open = false;
        }
    }

    /// Opens the profile in `dir` next to the others (or shows it when it already runs) and brings it on screen.
    fn open_account(&mut self, dir: std::path::PathBuf) {
        if dir == self.root {
            return;
        }
        if let Some(i) = self.parked.iter().position(|p| p.root == dir) {
            return self.switch_to(i);
        }
        let Some(d) = &self.dirs else { return };
        if let Ok(session) = Session::start(d, dir.clone(), self.trust.clone()) {
            self.parked.push(Parked { root: dir, session, recorded: false });
            let last = self.parked.len() - 1;
            self.switch_to(last);
        }
    }

    /// Theme and accent for now (OS appearance, saved choice, automatic night); re-read every minute.
    fn refresh(&mut self, ctx: &egui::Context) {
        let system_dark = ctx.system_theme().is_some_and(|t| t == egui::Theme::Dark);
        self.theme = self.prefs.effective_theme(system_dark, matrix_desktop::local_hour());
        self.tokens = self.theme.tokens_with(self.prefs.effective_accent());
        let key = (self.theme, self.tokens.accent);
        if self.applied != Some(key) {
            ctx.set_visuals(visuals(&self.tokens, self.theme.is_dark()));
            self.applied = Some(key);
        }
    }
}

fn bar(t: &DesignTokens) -> egui::Frame {
    egui::Frame::NONE.fill(rgb(t.surface)).stroke(Stroke::new(1., rgb(t.border)))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.refresh(&ctx);
        ctx.request_repaint_after(Duration::from_secs(60));
        if let Some((path, frame)) = &mut self.shot {
            let secs: u64 = std::env::var("GNOBBI_EGUI_SHOT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
            if frame.elapsed().as_secs() >= secs && !self.shot_sent {
                self.shot_sent = true;
                let path = path.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
                    std::process::exit(0);
                })));
            }
            ctx.request_repaint();
        }
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(session) = &mut self.session {
            if let Some(path) = dropped.first() {
                // With the devices window open a dropped picture is the profile photo, otherwise an attachment.
                match &mut session.verification {
                    _ if session.edit.is_some() => {
                        if let Some(e) = &mut session.edit {
                            e.avatar = Some(path.clone());
                        }
                    }
                    Some(v) if session.devices_open => {
                        v.save_profile(None, matrix_engine::verification::ProfileAvatar::Set(path.clone()));
                    }
                    _ => session.attach(path),
                }
            }
            // Ctrl/Cmd+K: jump to the room search.
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
                session.focus_search = true;
            }
            session.reading = (ctx.input(|i| i.focused), session.heights.at_bottom, self.prefs.read_receipts);
            session.poll(&ctx, self.prefs.notifications, self.prefs.message_preview);
            // The other accounts keep syncing: unread counts and notifications go on, read markers do not.
            let mut wake = None;
            for (i, p) in self.parked.iter_mut().enumerate() {
                p.session.reading = (false, false, self.prefs.read_receipts);
                p.session.poll(&ctx, self.prefs.notifications, self.prefs.message_preview);
                if let Some(v) = &p.session.verification
                    && let Some((id, name)) = v.profile()
                {
                    let shown = name.clone().unwrap_or_else(|| id.clone());
                    if p.session.account_name != shown {
                        p.session.account_name = shown;
                    }
                }
                // A click on one of its notifications brings that account to the front.
                if p.session.pending_select.is_some() {
                    wake = Some(i);
                }
            }
            self.wake = wake;
            if !self.recorded
                && let (Some(v), Some(dirs)) = (&session.verification, &self.dirs)
                && let Some((id, name)) = v.profile()
            {
                let mut catalog = matrix_desktop::AccountCatalog::load(dirs);
                if catalog.record_login(dirs, &self.root, id, name.as_deref().unwrap_or(id)) {
                    let _ = catalog.save(dirs);
                }
                self.recorded = true;
            }
            if let Some(v) = &session.verification
                && let Some((id, name)) = v.profile()
            {
                let shown = name.clone().unwrap_or_else(|| id.clone());
                if session.account_name != shown {
                    session.account_name = shown;
                }
            }
            if let Some(messages) = &mut session.messages
                && messages.set_hide_redacted(self.prefs.hide_redacted)
            {
                session.heights = Default::default();
            }
            // The backend answers off-thread and cannot wake the UI: poll it ten times a second.
            // shortcut: a fixed tick, replace by a repaint signal from the workers when idle power matters.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.tray(&ctx);
        self.test_hooks(&ctx);
        // A second launch hands its `matrix:` link (or just «raise») to this instance.
        #[cfg(target_os = "linux")]
        for incoming in matrix_desktop::single_instance::take_incoming() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            if let Ok(link) = matrix_engine::matrix_link::parse_matrix_link(&incoming) {
                self.pending_link = Some((link, std::time::Instant::now()));
            }
        }
        if let Some(i) = self.wake.take() {
            self.switch_to(i);
        }
        let t = self.tokens;
        if self.session.as_ref().is_some_and(|s| s.account.is_some() && !s.recovering()) {
            self.shell(ui, &t);
        } else {
            self.sign_in(ui, &t);
        }
    }
}

impl App {
    /// Test hooks (env only): sign in with the given account, then open the view named in GNOBBI_EGUI_VIEW.
    fn test_hooks(&mut self, ctx: &egui::Context) {
        // GNOBBI_EGUI_ADD=1: once the first account is in the catalog, add a second one (signed in by GNOBBI_EGUI_LOGIN2).
        if !self.hook_added
            && self.recorded
            && std::env::var_os("GNOBBI_EGUI_ADD").is_some()
            && let Some(d) = &self.dirs
        {
            self.hook_added = true;
            let next = matrix_desktop::AccountCatalog::load(d).next_root(d);
            self.open_account(next);
        }
        let Some(s) = &mut self.session else { return };
        // The test accounts have nothing to recover: skip the step so the other views can be reached.
        if self.auto_login.is_some() && s.recovering() && std::env::var_os("GNOBBI_EGUI_KEEP_RECOVERY").is_none() {
            s.defer_recovery();
        }
        // shortcut: one account, no «Unisciti» dialog and no jump to the message of the link; a room we are not in is only logged. Give up after 60 s.
        if let Some((link, since)) = &self.pending_link
            && s.account.is_some()
            && !s.sidebar.rooms().is_empty()
        {
            use matrix_domain::MatrixLink;
            match link {
                MatrixLink::Room { target, .. } => {
                    let found = s.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == target || r.info.aliases.iter().any(|a| a == target)).map(|r| r.scope.clone());
                    match found {
                        Some(scope) => drop(s.sidebar.select(&scope)),
                        None => {
                            s.new_open = true;
                            self.new_form.prefill_join(target);
                        }
                    }
                }
                MatrixLink::User { user_id } => {
                    s.info_open = true;
                    s.open_profile(user_id.clone());
                }
            }
            self.pending_link = None;
        } else if self.pending_link.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() > 60) {
            self.pending_link = None;
        }
        if let Some(login) = if self.hook_added { &self.auto_login2 } else { &self.auto_login }
            && s.account.is_none()
            && matches!(s.status(), FormStatus::Prompt(_))
        {
            let get = |k: &str| login.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
            s.homeserver = get("homeserver");
            s.user = get("user");
            s.password = get("password");
            s.phrase = get("phrase");
            s.submit();
        }
        if s.account.is_some() && std::env::var_os("GNOBBI_EGUI_VIEW").is_some() {
            let view = std::env::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
            // The first room with something to read, once the list has arrived.
            if s.sidebar.selected().is_none()
                && let Some(&first) = s.sidebar.visible_indices().first()
            {
                let scope = s.sidebar.rooms()[first].scope.clone();
                s.sidebar.select(&scope);
            }
            // GNOBBI_EGUI_EXPORT=<passphrase> / GNOBBI_EGUI_NEWKEY=1: the two key actions of the Devices window, once.
            if !self.hook_keys
                && let Some(v) = &mut s.verification
                && v.own().is_some()
            {
                if let Ok(pass) = std::env::var("GNOBBI_EGUI_EXPORT") {
                    let path = std::env::var_os("GNOBBI_EGUI_EXPORT_PATH").map(std::path::PathBuf::from).unwrap_or_default();
                    devices::start_export(v, path, &zeroize::Zeroizing::new(pass));
                    self.hook_keys = true;
                } else if std::env::var_os("GNOBBI_EGUI_NEWKEY").is_some() {
                    v.keys = Some(matrix_desktop::key_export::KeysPage::new(matrix_desktop::key_export::KeysKind::Reset));
                    v.reset_recovery(None);
                    self.hook_keys = true;
                }
            }
            // GNOBBI_EGUI_ALIAS=<local>: adds that address to the open room once its addresses are loaded.
            if !self.hook_alias
                && s.aliases.is_some()
                && let Ok(local) = std::env::var("GNOBBI_EGUI_ALIAS")
            {
                s.alias_action(matrix_engine::alias::AliasAction::Add { local, make_canonical: None });
                self.hook_alias = true;
            }
            // GNOBBI_EGUI_TYPE=<text>: puts the text in the composer and focuses it (to see the «@» list).
            if !self.hook_type
                && s.messages.is_some()
                && s.members.is_some()
                && let Ok(text) = std::env::var("GNOBBI_EGUI_TYPE")
            {
                s.draft = text;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
                self.hook_type = true;
            }
            // A headless seat never focuses the window, and egui drops the text focus with it: say it is focused.
            if self.hook_type && std::env::var_os("GNOBBI_EGUI_TYPE").is_some() {
                ctx.input_mut(|i| i.events.push(egui::Event::WindowFocused(true)));
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
            }
            // GNOBBI_EGUI_SEND=<text>: sends it once into the open room (disposable servers only).
            if !self.hook_sent
                && s.messages.as_ref().is_some_and(|m| !m.events().is_empty())
                && let Ok(text) = std::env::var("GNOBBI_EGUI_SEND")
            {
                s.send(text);
                self.hook_sent = true;
                self.hook_menu_at = Some(std::time::Instant::now());
            }
            // View «menu»: right-click menu of the newest message, a few seconds after the send.
            if view == "menu"
                && let Some(at) = self.hook_menu_at
                && at.elapsed().as_secs() >= 6
                && s.menu.is_none()
                && let Some(id) = s.messages.as_ref().and_then(|m| m.events().last().map(|e| e.id.clone()))
            {
                s.open_menu(id, egui::pos2(900., 300.));
                self.hook_menu_at = None;
            }
            match view.as_str() {
                "info" => s.info_open = true,
                "search" => s.search_open = true,
                "devices" => s.devices_open = true,
                "new" => s.new_open = true,
                "poll" => s.poll_open = true,
                "settings" => self.settings_open = true,
                "accounts" => self.accounts_open = true,
                _ => {}
            }
        }
    }

    /// Tray clicks, the menu and icon state, and what closing the window does (hide when the tray can be reached).
    fn tray(&mut self, ctx: &egui::Context) {
        use matrix_desktop::tray_menu::{CloseAction, TrayAction, close_action, tray_menu};
        let show = |ctx: &egui::Context| {
            #[cfg(target_os = "macos")]
            mac_app(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        };
        // The Dock click (or Cmd+Tab) shows the app without a tray action.
        if ctx.input(|i| i.focused) {
            self.window_visible = true;
        }
        let reachable = self.tray.as_ref().is_some_and(|t| t.host_present());
        if let Some(tray) = &self.tray {
            for action in tray.actions() {
                match action {
                    TrayAction::ToggleWindow if self.window_visible => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    TrayAction::ToggleWindow | TrayAction::ShowWindow => {
                        show(ctx);
                        self.window_visible = true;
                    }
                    TrayAction::ToggleNotifications => {
                        let on = !self.prefs.notifications;
                        self.prefs.notifications = on;
                        if let Some(dirs) = &self.dirs {
                            let _ = Preferences::update(dirs, |p| p.notifications = on);
                        }
                    }
                    TrayAction::Quit => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    // One account per window until multi-account arrives.
                    TrayAction::SwitchAccount(_) => {}
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && close_action(self.prefs.close_to_tray, reachable) == CloseAction::HideToTray
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            #[cfg(target_os = "macos")]
            mac_app(true);
            #[cfg(not(target_os = "macos"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_visible = false;
        }
        if let Some(tray) = &mut self.tray {
            let unread = self.session.as_ref().and_then(|s| s.sidebar.navigation_entries().first().map(|e| e.unread)).unwrap_or(0);
            let items = tray_menu(self.window_visible, self.prefs.notifications, &[]);
            tray.update(&items, unread, Vec::new());
        }
    }

    fn shell(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        let state = match self.session.as_ref().and_then(|s| s.sync) {
            Some(RoomSyncStatus::Ready) => "Connesso",
            Some(RoomSyncStatus::AuthenticationFailed) => "Accesso non riuscito",
            Some(RoomSyncStatus::SyncFailed) => "Sincronizzazione interrotta",
            _ => "Connessione…",
        };
        // One round button per account when there is more than one; the shown one is highlighted.
        let mut pick_account = None;
        if !self.parked.is_empty() {
            let unread = |s: &Session| s.sidebar.navigation_entries().first().map_or(0, |e| e.unread);
            let mut entries: Vec<(Option<usize>, String, u64)> = Vec::new();
            if let Some(s) = &self.session {
                entries.push((None, s.account_name.clone(), unread(s)));
            }
            for (i, p) in self.parked.iter().enumerate() {
                entries.push((Some(i), p.session.account_name.clone(), unread(&p.session)));
            }
            egui::Panel::left("accounts").exact_size(52.).resizable(false).frame(bar(t)).show(ui, |ui| {
                ui.add_space(8.);
                for (which, name, n) in &entries {
                    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(52., 48.), egui::Sense::click());
                    let c = rect.center();
                    if which.is_none() {
                        ui.painter().rect_filled(egui::Rect::from_center_size(c, egui::Vec2::new(44., 44.)), 10., rgb(t.selected));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(egui::Rect::from_center_size(c, egui::Vec2::new(44., 44.)), 10., rgb(t.input));
                    }
                    let initial: String = name.trim_start_matches('@').chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned());
                    ui.painter().circle_filled(c, 16., rgb(AVATAR));
                    ui.painter().text(c, egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(15.), rgb(t.on_accent));
                    if *n > 0 {
                        list::badge(ui, c + egui::Vec2::new(14., -14.), *n, t);
                    }
                    resp.clone().on_hover_text(name.as_str());
                    if resp.clicked() {
                        pick_account = *which;
                    }
                }
            });
        }
        if let Some(i) = pick_account {
            self.switch_to(i);
        }
        if let Some(session) = &mut self.session {
            egui::Panel::left("rail").exact_size(64.).resizable(false).frame(bar(t)).show(ui, |ui| {
                list::rail(ui, &mut session.sidebar, t);
            });
            egui::Panel::left("list").exact_size(t.sidebar).resizable(false).frame(bar(t)).show(ui, |ui| {
                let mut new_chat = false;
                list::rooms(ui, &mut session.sidebar, &mut session.query, &mut new_chat, &mut session.focus_search, t);
                session.new_open |= new_chat;
            });
        }
        let has_room = self.session.as_ref().is_some_and(|s| s.messages.is_some());
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        let badge = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| (r.initials.clone(), r.color)));
        egui::Panel::top("header").exact_size(t.header).frame(bar(t)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(16.);
                if let Some((initials, color)) = &badge {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(32.), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 16., rgb(*color));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, egui::FontId::proportional(13.), rgb(t.avatar_text));
                    ui.add_space(4.);
                }
                ui.label(title.as_deref().unwrap_or(state));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.);
                    if ui.button("⚙").clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    if ui.button("👤").clicked() {
                        self.accounts_open = !self.accounts_open;
                    }
                    if ui.button("🖥").clicked()
                        && let Some(s) = &mut self.session
                    {
                        s.devices_open = !s.devices_open;
                        if let Some(v) = &mut s.verification {
                            v.refresh();
                        }
                    }
                    if has_room && ui.button("ⓘ").clicked()
                        && let Some(s) = &mut self.session
                    {
                        s.info_open = !s.info_open;
                        s.search_open = false;
                    }
                    if has_room && ui.button("🔍").clicked()
                        && let Some(s) = &mut self.session
                    {
                        s.search_open = !s.search_open;
                        s.info_open = false;
                    }
                });
            });
        });
        if let Some(s) = &self.session
            && s.info_open
            && let Some(room) = s.sidebar.selected()
        {
            let members = s.members.as_ref().map(|(_, m)| m.as_slice());
            let mut out = info::Out::default();
            let mut alias_input = std::mem::take(&mut self.alias_input);
            let mut alias_action = None;
            let mut view = info::AliasView { data: s.aliases.as_ref(), error: s.alias_error, input: &mut alias_input, action: None };
            egui::Panel::right("info").exact_size(300.).resizable(false).frame(bar(t)).show(ui, |ui| out = info::show(ui, room, members, &mut view, t));
            alias_action = view.action.take();
            self.alias_input = alias_input;
            if let (Some(action), Some(s)) = (alias_action, &mut self.session) {
                s.alias_action(action);
            }
            if let (Some(user), Some(s)) = (out.open_profile, &mut self.session) {
                s.open_profile(user);
            }
            if out.edit && let Some(s) = &mut self.session {
                s.open_edit();
            }
        }
        if let Some(s) = &mut self.session
            && s.info_open
            && let Some((id, profile)) = s.profile.clone()
        {
            let (mut close, mut flip, mut moderation) = (false, None, None);
            let (rights, failed) = (s.rights.clone(), s.moderation_failed);
            egui::Panel::right("member").exact_size(300.).resizable(false).frame(bar(t)).show(ui, |ui| flip = info::card(ui, &id, profile.as_ref(), rights.as_ref(), failed, &mut self.confirm, &mut moderation, &mut close, t));
            if let Some(action) = moderation {
                s.moderate(id.clone(), action);
            }
            if let Some(ignore) = flip {
                s.set_ignored(id, ignore);
            }
            if close {
                s.profile = None;
            }
        }
        if let Some(s) = &mut self.session
            && s.search_open
        {
            let state = s.search.clone();
            egui::Panel::right("search").exact_size(300.).resizable(false).frame(bar(t)).show(ui, |ui| info::search(ui, &mut s.search_term, &state, t));
        }
        if let Some(s) = &mut self.session
            && let Some(mut th) = s.thread.take()
        {
            let (mut close, mut body) = (false, None);
            egui::Panel::right("thread").exact_size(320.).resizable(false).frame(bar(t)).show(ui, |ui| body = info::thread(ui, &mut th, &mut close, t));
            if !close {
                s.thread = Some(th);
            }
            if let Some(body) = body {
                s.send_thread(body);
            }
        }
        let mut to_send = None;
        egui::Panel::bottom("composer").min_size(t.bottom_bar).frame(bar(t)).show(ui, |ui| {
            if let Some(s) = &mut self.session
                && s.messages.is_some()
            {
                let mode = s.target.as_ref().map_or(composer::Mode::Send, |(_, m)| *m);
                let mut cancel = false;
                let mut drop_file = false;
                let mut open_poll = false;
                let mut picked_file = None;
                let mut voice_act = None;
                let name = s.attachment.as_ref().map(|(n, _)| n.clone());
                if let Some(notice) = &s.notice {
                    ui.label(notice);
                }
                let members: Vec<_> = s.members.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
                to_send = composer::show(ui, &mut s.draft, mode, name.as_deref(), &mut cancel, &mut drop_file, &mut open_poll, &mut picked_file, s.recorder.as_ref().map(|r| r.elapsed_secs()), &mut voice_act, &members, &mut s.mentions);
                if drop_file {
                    s.attachment = None;
                }
                s.poll_open |= open_poll;
                if let Some(path) = picked_file {
                    s.attach(&path);
                }
                match voice_act {
                    Some(composer::VoiceAct::Start) => s.start_recording(),
                    Some(composer::VoiceAct::Send) => s.stop_recording(true),
                    Some(composer::VoiceAct::Cancel) => s.stop_recording(false),
                    None => {}
                }
                // At the cap the note goes out by itself; the timer repaints the elapsed seconds.
                if s.recorder.as_ref().is_some_and(|r| r.is_full()) {
                    s.stop_recording(true);
                }
                if cancel {
                    s.cancel_target();
                }
            }
        });
        if let (Some(body), Some(s)) = (to_send, &mut self.session) {
            s.submit_draft(body);
        }
        if let Some(s) = &mut self.session {
            message_menu(ui.ctx(), s, t);
        }
        if let Some(s) = &mut self.session
            && s.new_open
        {
            newchat::show(ui.ctx(), s, &mut self.new_form, t);
        }
        if let Some(s) = &mut self.session
            && let Some(v) = &mut s.verification
        {
            if s.devices_open {
                devices::show(ui.ctx(), v, &mut s.devices_open, &mut self.device_fields, t);
            }
            devices::dialogs(ui.ctx(), v, &mut self.device_fields);
            devices::popup(ui.ctx(), v);
            devices::new_key(ui.ctx(), v);
        }
        if let Some(s) = &mut self.session
            && s.poll_open
        {
            poll::show(ui.ctx(), s, &mut self.poll_form, t);
        }
        if let Some(s) = &mut self.session
            && s.edit.is_some()
            && let Some(rights) = s.sidebar.selected().map(|r| r.info.edit_rights)
        {
            roomedit::show(ui.ctx(), s, rights);
        }
        if self.accounts_open
            && let Some(dirs) = &self.dirs
        {
            match accounts::show(ui.ctx(), &mut self.accounts_open, dirs, &self.root, t) {
                Some(accounts::Launch::Open(dir)) => self.open_account(dir),
                Some(accounts::Launch::Add) => {
                    let next = matrix_desktop::AccountCatalog::load(dirs).next_root(dirs);
                    self.open_account(next);
                }
                None => {}
            }
        }
        if self.settings_open
            && let Some(new) = settings::show(ui.ctx(), &mut self.settings_open, &self.prefs, self.dirs.as_ref(), t)
        {
            if new.icon != self.prefs.icon
                && let Some(icon) = app_icon(new.icon)
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon))));
            }
            self.prefs = new;
            self.applied = None;
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show(ui, |ui| {
            let invite = self.session.as_ref().and_then(|s| s.sidebar.selected_invite().map(|r| (r.scope.clone(), r.name.clone())));
            if let (Some((scope, name)), Some(s)) = (invite, &mut self.session) {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.);
                    ui.heading(name);
                    ui.add_space(12.);
                    ui.horizontal(|ui| {
                        if ui.button("Accetta").clicked() {
                            s.answer_invite(scope.clone(), true);
                        }
                        if ui.button("Rifiuta").clicked() {
                            s.answer_invite(scope, false);
                        }
                    });
                });
            } else if let Some(s) = &mut self.session
                && let Some(messages) = &s.messages
            {
                let mut acts = timeline::Acts { now_playing: s.audio_now(), ..Default::default() };
                let (first, menu, save, thread) = timeline::show(ui, messages.events(), messages, &mut s.heights, &s.pictures, &s.avatars, &mut acts, s.sidebar.selected().map_or(3, |r| r.info.joined_members), t);
                s.first_visible = first;
                if let Some((id, pos)) = menu {
                    s.open_menu(id, pos);
                }
                if let Some((id, name)) = save {
                    s.save(id, &name);
                }
                if let Some(root) = thread {
                    s.open_thread(root);
                }
                if let Some(id) = acts.play.take() {
                    s.play_audio(id);
                }
                if let Some((id, ix)) = acts.vote {
                    s.vote(id, ix);
                }
            }
        });
    }

    fn sign_in(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 4.).max(24.));
                egui::Frame::NONE
                    .fill(rgb(t.surface))
                    .stroke(Stroke::new(1., rgb(t.border)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(24.)
                    .show(ui, |ui| {
                        ui.set_width(360.);
                        match &mut self.session {
                            Some(session) => sign_in_form(ui, session, t),
                            None => {
                                ui.label(self.start_error.as_deref().unwrap_or("Avvio non riuscito"));
                            }
                        }
                    });
            });
        });
    }
}

/// Right-click menu: only what the engine says the user may do with the message.
fn message_menu(ctx: &egui::Context, s: &mut Session, t: &DesignTokens) {
    let Some(pos) = s.menu else { return };
    let fresh = std::mem::take(&mut s.menu_fresh);
    let perms = s.ops.details().map(|d| d.permissions);
    let mut action: Option<&'static str> = None;
    let area = egui::Area::new(egui::Id::new("message-menu")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(rgb(t.surface)).show(ui, |ui| {
            ui.set_min_width(160.);
            let Some(p) = perms else {
                ui.spinner();
                return;
            };
            if p.react {
                ui.horizontal(|ui| {
                    for (key, name) in [("👍", "up"), ("❤️", "heart"), ("😂", "laugh")] {
                        if ui.button(key).clicked() {
                            action = Some(match name { "up" => "👍", "heart" => "❤️", _ => "😂" });
                        }
                    }
                });
            }
            if p.reply && ui.button("Rispondi").clicked() {
                action = Some("reply");
            }
            if p.edit && ui.button("Modifica").clicked() {
                action = Some("edit");
            }
            let poll = s.ops.selection().and_then(|id| s.messages.as_ref()?.events().iter().find(|e| e.id == *id)?.poll.clone());
            if s.ops.can_end_poll(poll.as_ref()) && ui.button("Chiudi sondaggio").clicked() {
                action = Some("end");
            }
            if p.redact {
                let label = if s.confirm_delete { "Conferma eliminazione" } else { "Elimina" };
                if ui.button(RichText::new(label).color(rgb(t.error))).clicked() {
                    if s.confirm_delete { action = Some("delete") } else { s.confirm_delete = true }
                }
            }
            if !(p.react || p.reply || p.edit || p.redact) {
                ui.label("—");
            }
        });
    });
    match action {
        Some("reply") => s.begin(composer::Mode::Reply),
        Some("edit") => s.begin(composer::Mode::Edit),
        Some("delete") => s.delete(),
        Some("end") => s.end_poll(),
        Some(key) => s.react(key),
        None => {
            let outside = !fresh && ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
            if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                s.close_menu();
            }
        }
    }
}

/// The form for the flow the backend asked for: new profile (server, user, password) or unlock.
fn sign_in_form(ui: &mut egui::Ui, s: &mut Session, t: &DesignTokens) {
    use matrix_engine::signin::SignInMode;
    if s.recovering() {
        recovery_form(ui, s, t);
        return;
    }
    let status = s.status();
    let prompt = match status {
        FormStatus::Prompt(p) => Some(p),
        _ => None,
    };
    // While no prompt is shown (login in flight, start-up) keep the flow of the last prompt and time the wait.
    let (mode_id, since_id) = (egui::Id::new("signin_mode"), egui::Id::new("signin_wait"));
    if let Some(p) = prompt {
        ui.data_mut(|d| {
            d.insert_temp(mode_id, p.mode);
            d.remove::<std::time::Instant>(since_id);
        });
    }
    let mode = prompt.map_or_else(|| ui.data(|d| d.get_temp(mode_id)).unwrap_or(SignInMode::UnlockVault), |p| p.mode);
    let waited = prompt.is_none().then(|| ui.data_mut(|d| d.get_temp_mut_or_insert_with(since_id, std::time::Instant::now).elapsed()));
    let unlock = mode != SignInMode::Password;
    ui.heading(if unlock { "Sblocca Gnobbi" } else { "Accedi a Gnobbi" });
    ui.add_space(12.);
    let mut enter = false;
    let mut field = |ui: &mut egui::Ui, text: &mut String, hint: &str, secret: bool| {
        let r = ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(text).hint_text(hint).password(secret).align(egui::Align2::LEFT_CENTER));
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.);
    };
    if let Some(p) = prompt {
        match (p.mode, s.form.vault) {
            (SignInMode::Password, vault) => {
                field(ui, &mut s.homeserver, "Homeserver HTTPS", false);
                field(ui, &mut s.user, "ID Matrix completo", false);
                field(ui, &mut s.password, "Password Matrix", true);
                if !vault {
                    field(ui, &mut s.phrase, "Frase locale · almeno 16 caratteri", true);
                }
            }
            (SignInMode::Unlock, _) => field(ui, &mut s.phrase, "Frase locale · almeno 16 caratteri", true),
            (SignInMode::UnlockVault, _) => {}
        }
    }
    // The engine gives up on its own only after discovery succeeds: after 20 s say what the form already says for an unreachable server.
    let stalled = waited.is_some_and(|w| w > std::time::Duration::from_secs(20));
    let error = if stalled { FormStatus::Failed(matrix_engine::signin::SignInError::Discovery).error_message() } else { s.form.status().error_message() };
    if let Some(message) = error {
        ui.colored_label(ui.visuals().error_fg_color, message);
        ui.add_space(8.);
    }
    if prompt.is_some() {
        let label = if unlock { "Sblocca" } else { "Accedi" };
        let button = egui::Button::new(egui::RichText::new(label).color(list::rgb(t.on_accent))).fill(list::rgb(t.accent));
        if ui.add_sized([ui.available_width(), 36.], button).clicked() || enter {
            s.submit();
        }
    } else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if unlock { "Sblocco in corso…" } else { "Accesso in corso…" });
        });
    }
}

/// «Cronologia cifrata»: the recovery key restores old encrypted messages; it can be skipped.
fn recovery_form(ui: &mut egui::Ui, s: &mut Session, t: &DesignTokens) {
    ui.heading("Cronologia cifrata");
    ui.add_space(8.);
    ui.label("Senza recupero la cronologia cifrata precedente non sarà leggibile su questo dispositivo.");
    ui.add_space(4.);
    ui.label("Inserisci la chiave di recupero Matrix del tuo account (non è la frase locale del profilo). Il backup può non contenere tutti i messaggi: non tutti i vecchi messaggi sono garantiti.");
    ui.add_space(12.);
    if matches!(s.form.recovery(), Some(matrix_engine::signin::RecoveryState::Pending)) {
        ui.spinner();
        return;
    }
    let r = ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut s.phrase).hint_text("Chiave di recupero Matrix").password(true).align(egui::Align2::LEFT_CENTER));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.);
    if let Some(message) = s.form.error_message() {
        ui.colored_label(ui.visuals().error_fg_color, message);
        ui.add_space(8.);
    }
    let accent = |text: &str| egui::Button::new(egui::RichText::new(text).color(list::rgb(t.on_accent))).fill(list::rgb(t.accent));
    if ui.add_sized([ui.available_width(), 36.], accent("Recupera cronologia cifrata")).clicked() || enter {
        s.recover();
    }
    ui.add_space(6.);
    if ui.add_sized([ui.available_width(), 36.], accent("Continua senza recupero")).clicked() {
        s.defer_recovery();
    }
}

/// First instance listens for later launches; if one is already running (the GPUI app included)
/// the launch is handed to it and this process exits. Socket trouble only costs this behaviour.
#[cfg(target_os = "linux")]
fn single_instance_gate(link: Option<String>) {
    use matrix_desktop::single_instance::{Claim, claim, default_socket_path, outgoing, serve};
    let Some(path) = default_socket_path() else {
        return eprintln!("Gnobbi: single instance off (no private state directory)");
    };
    match claim(&path, &outgoing(link)) {
        Ok(Claim::First(listener)) => serve(listener),
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

#[cfg(windows)]
fn single_instance_gate(_link: Option<String>) {
    use matrix_desktop::windows_link::{Claim, claim};
    match claim("") {
        Ok(Claim::First) => {}
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn single_instance_gate(_link: Option<String>) {}

/// Linux: the bundled Noto Color Emoji (SIL OFL 1.1, see THIRD_PARTY_NOTICES.md) as the last fallback, so the emoji set is the
/// same on every system. macOS and Windows keep their own emoji font (system_fonts).
fn register_emoji_font(ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    {
        static NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../../matrix-desktop/assets/fonts/NotoColorEmoji.ttf");
        ctx.add_font(egui::epaint::text::FontInsert::new(
            "noto-color-emoji",
            egui::FontData::from_static(NOTO_COLOR_EMOJI),
            vec![
                egui::epaint::text::InsertFontFamily { family: egui::FontFamily::Proportional, priority: egui::epaint::text::FontPriority::Lowest },
                egui::epaint::text::InsertFontFamily { family: egui::FontFamily::Monospace, priority: egui::epaint::text::FontPriority::Lowest },
            ],
        ));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = ctx;
}

/// The icon chosen in Settings as the window/Dock icon (Linux Wayland takes it from the installed desktop entry instead).
fn app_icon(choice: matrix_desktop::IconChoice) -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(choice.runtime_png(), image::ImageFormat::Png).ok()?.into_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

/// macOS: closing to the tray hides the whole app (like Cmd+H) instead of ordering the window out, because a window that is not
/// visible gets no frames, so a tray or Dock click would never reach `update`; the Dock click unhides it by itself.
#[cfg(target_os = "macos")]
fn mac_app(hide: bool) {
    use cocoa::{appkit::NSApp, base::{id, nil, YES}};
    use objc::{msg_send, sel, sel_impl};
    // SAFETY: called from `update`, i.e. on the main thread, with a live NSApplication.
    unsafe {
        let app: id = NSApp();
        if hide {
            let _: () = msg_send![app, hide: nil];
        } else {
            let _: () = msg_send![app, unhide: nil];
            let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        }
    }
}

fn main() -> eframe::Result {
    // A `matrix:` link is not an option: taken out first, validated by the engine, carried out once the rooms are there.
    let (args, raw_link) = matrix_desktop::take_matrix_link(std::env::args().skip(1).collect());
    let link = raw_link.clone().and_then(|raw| matrix_engine::matrix_link::parse_matrix_link(&raw).map_err(|_| eprintln!("Gnobbi: ignored matrix link")).ok());
    let dirs = matrix_desktop::app_dirs(Platform::CURRENT, |k| std::env::var(k).ok());
    let default_root = dirs.as_ref().map(|d| d.data.join("profile"));
    // `--signin <folder>` runs another account's profile; the default profile is the first install's.
    let mut trust = None;
    let root = match matrix_desktop::SignInOptions::parse(&args) {
        Ok(Some(o)) => {
            // An explicit CA (a test homeserver); read once, at most 1 MiB, like the GPUI app.
            trust = o.ca_file.as_ref().map(|p| {
                use std::io::Read;
                let mut pem = Vec::new();
                std::fs::File::open(p).and_then(|f| f.take(1 << 20).read_to_end(&mut pem)).unwrap_or_else(|_| {
                    eprintln!("Could not read explicit trust root");
                    std::process::exit(2);
                });
                pem
            });
            o.root
        }
        Ok(None) => default_root.clone().unwrap_or_default(),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // Another profile given by hand is its own window: only the default one hands launches over to a running instance.
    let multi = Some(&root) == default_root.as_ref();
    if multi {
        single_instance_gate(raw_link);
        if let Some(dirs) = &dirs {
            // A pre-catalog install is listed in place; the profile folder does not move.
            let mut catalog = matrix_desktop::AccountCatalog::load(dirs);
            if catalog.migrate_legacy(dirs) {
                let _ = catalog.save(dirs);
            }
        }
    }
    let first = Theme::Classic.tokens();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([first.width, first.height])
            .with_min_inner_size([360., 420.])
            .with_title(TITLE)
            .with_app_id(APP_ID)
            .with_icon(app_icon(dirs.as_ref().map(Preferences::load).unwrap_or_default().icon).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(move |cc| {
        register_emoji_font(&cc.egui_ctx);
        Ok(Box::new(App::new(dirs.as_ref(), root, trust, link, multi)))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visuals_follow_the_tokens_in_every_theme() {
        for theme in Theme::ALL {
            let t = theme.tokens_with(matrix_desktop::Accent::Blue);
            let v = visuals(&t, theme.is_dark());
            assert_eq!(v.panel_fill, rgb(t.timeline));
            assert_eq!(v.window_fill, rgb(t.surface));
        }
    }
}
