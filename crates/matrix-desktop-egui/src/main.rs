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
#[cfg(target_os = "linux")]
mod desktop_entry;
mod info;
#[allow(dead_code)] // pure state, wired to the join popup by the UI card
mod join;
mod list;
mod members;
mod newchat;
mod poll;
mod rail;
mod roomedit;
mod space;
mod session;
mod settings;
mod timeline;
mod viewer;
use session::Session;

/// Avatar fallback colour (identity, not theme).
pub const AVATAR: u32 = matrix_desktop::AVATAR_FALLBACK;

/// File in the data folder with the profile folder of the account on screen.
const LAST_ACCOUNT: &str = "last_account";

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

/// Below this window width the room list hides: rail 76 + list 320 + a chat of at least ~376.
const NARROW_WIDTH: f32 = 772.;
/// Narrow layout shows rail 76 + chat; the chat needs ≥360.
const MIN_WIDTH: f32 = 436.;

fn list_max_width(window_width: f32) -> f32 { (window_width - MIN_WIDTH).clamp(240., 600.) }

struct App {
    prefs: Preferences,
    dirs: Option<AppDirs>,
    settings_open: bool,
    accounts_open: bool,
    /// Narrow window only: the room list replaces the chat while true.
    list_open: bool,
    rail: rail::State,
    /// Catalog id of the shown account (the rail layout is stored under it), with the profile folder it was read for.
    rail_account: Option<(std::path::PathBuf, u64)>,
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
    account_form: accounts::Form,
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
    pending_web: Option<matrix_desktop::SafeWebUrl>,
    hook_sent: bool,
    hook_added: bool,
    auto_login2: Option<serde_json::Value>,
    hook_type: bool,
    hook_alias: bool,
    hook_member: bool,
    alias_input: String,
    hook_keys: bool,
    hook_menu_at: Option<std::time::Instant>,
    /// Why the backend could not start, shown instead of the form.
    start_error: Option<String>,
}

impl App {
    fn new(dirs: Option<&AppDirs>, root: std::path::PathBuf, trust: Option<Vec<u8>>, link: Option<matrix_domain::MatrixLink>, multi: bool) -> App {
        let prefs = dirs.map(Preferences::load).unwrap_or_else(Preferences::default);
        #[cfg(target_os = "linux")]
        if dirs.is_some() {
            desktop_entry::install_at_start(prefs.icon, prefs.desktop_entry_dont_ask);
        }
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
        // The account on screen at the last quit comes back on screen; gone or never saved: the default profile.
        let last = dirs.and_then(|d| std::fs::read_to_string(d.data.join(LAST_ACCOUNT)).ok()).and_then(|l| parked.iter().position(|p| p.root.to_string_lossy() == l.trim()));
        let mut app = App { prefs, dirs: dirs.cloned(), settings_open: false, accounts_open: false, list_open: false, rail: rail::State { layouts: dirs.map(matrix_desktop::RailLayouts::load).unwrap_or_default(), ..Default::default() }, rail_account: None, root: root.clone(), recorded: false, parked, trust: trust.clone(), wake: None, tray: matrix_desktop::tray::Tray::start(), quitting: false, window_visible: true, new_form: Default::default(), device_fields: Default::default(), account_form: Default::default(), poll_form: Default::default(), confirm: None, theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, shot: std::env::var_os("GNOBBI_EGUI_SHOT").map(|p| (p.into(), std::time::Instant::now())),
            pending_link: link.map(|l| (l, std::time::Instant::now())), pending_web: None, shot_sent: false, hook_sent: false, hook_added: false, auto_login2: std::env::var_os("GNOBBI_EGUI_LOGIN2").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), hook_type: false, hook_alias: false, hook_member: false, alias_input: String::new(), hook_keys: false, hook_menu_at: None,
            auto_login: std::env::var_os("GNOBBI_EGUI_LOGIN").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), session, start_error };
        if let Some(i) = last {
            app.switch_to(i);
        }
        app
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
            self.account_form = Default::default();
            self.poll_form = Default::default();
            self.confirm = None;
            self.settings_open = false;
            if let Some(d) = &self.dirs {
                let _ = std::fs::write(d.data.join(LAST_ACCOUNT), self.root.to_string_lossy().as_bytes());
            }
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
    egui::Frame::NONE.fill(rgb(t.surface))
}

fn list_fill(t: &DesignTokens) -> Color32 {
    let surface = rgb(t.surface);
    Color32::from_rgb(surface.r().saturating_sub(3), surface.g().saturating_sub(3), surface.b().saturating_sub(3))
}

impl eframe::App for App {
    /// Runs before every frame and also with the window minimized, occluded or hidden (then `ui` does not run):
    /// draining the backend here lets notifications go out with the window out of sight.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(session) = &mut self.session else { return };
        // Out of sight nothing is being read: the flags of the last painted frame would still say "focused".
        if ctx.input(|i| i.viewport().minimized == Some(true) || i.viewport().occluded == Some(true)) {
            session.reading.0 = false;
        }
        session.poll(ctx, self.prefs.notifications, self.prefs.message_preview);
        // The other accounts keep syncing: unread counts and notifications go on, read markers do not.
        let mut wake = None;
        for (i, p) in self.parked.iter_mut().enumerate() {
            p.session.reading = (false, false, self.prefs.read_receipts);
            p.session.poll(ctx, self.prefs.notifications, self.prefs.message_preview);
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
        // The backend answers off-thread and cannot wake the UI: poll it ten times a second.
        // shortcut: a fixed tick, replace by a repaint signal from the workers when idle power matters.
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn on_exit(&mut self) {
        if let Some(s) = &mut self.session {
            s.remember_scroll();
        }
    }

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
                // With the account window open a dropped picture is the profile photo (kept for «Salva»), otherwise an attachment.
                match &mut session.verification {
                    _ if session.edit.is_some() => {
                        if let Some(e) = &mut session.edit {
                            e.avatar = Some(path.clone());
                        }
                    }
                    _ if self.accounts_open && session.profile_caps.is_some_and(|c| c.1) => self.account_form.pick(path.clone(), &ctx),
                    _ => session.attach(path),
                }
            }
            if session.account.is_some() && !session.recovering() {
                let command = |key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key));
                if command(egui::Key::K) && !session.new_open && !self.settings_open {
                    session.quick_open = !session.quick_open; session.quick_sel = 0;
                    session.new_term.clear();
                    if session.quick_open { ctx.memory_mut(|m| m.request_focus(egui::Id::unique("quick-search-field"))); }
                } else if !session.quick_open && !session.new_open && !self.settings_open {
                    if command(egui::Key::F) {
                        session.focus_search = true;
                    } else if command(egui::Key::Comma) {
                        self.settings_open = true;
                    } else if command(egui::Key::N) {
                        session.new_open = true;
                    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::I)) {
                        session.info_open = session.sidebar.selected().is_some() && !session.info_open;
                        session.search_open = false;
                    } else {
                        let previous = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab) || i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp));
                        let next = !previous && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab) || i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown));
                        if next || previous {
                            session.sidebar.navigate(if previous { matrix_desktop::Navigation::Previous } else { matrix_desktop::Navigation::Next });
                        }
                        let menu = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10) || i.consume_key(egui::Modifiers::NONE, egui::Key::Menu));
                        if menu && let Some(id) = session.messages.as_ref().and_then(|m| m.events().last().map(|e| e.id.clone())) {
                            session.open_menu(id, ctx.content_rect().center());
                        }
                    }
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape))
                    && !session.new_open && !session.poll_open && session.edit.is_none()
                    && !session.devices_open && !self.settings_open && !self.accounts_open && self.pending_web.is_none()
                {
                    if self.rail.edit { self.rail.edit = false; }
                    else if session.quick_open { session.quick_open = false; }
                    else if let Some(a) = session.admin.as_mut().filter(|a| !a.busy && (a.dialog.is_some() || a.inviting)) { (a.dialog, a.inviting, a.failed) = (None, false, false); }
                    else if session.space_view.is_some() { session.space_view = None; }
                    else if session.menu.is_some() { session.close_menu(); }
                    else if session.thread.is_some() { session.thread = None; }
                    else if session.profile.is_some() { session.profile = None; }
                    else if session.info_open { session.info_open = false; }
                    else if session.search_open { session.search_open = false; }
                }
            }
            if self.settings_open || self.accounts_open || self.pending_web.is_some() || session.new_open || session.quick_open || session.poll_open || session.edit.is_some() || session.devices_open || session.viewer.is_some() {
                if let Some(thread) = &mut session.thread { thread.tail_visible = false; }
            }
            session.reading = (ctx.input(|i| i.focused), session.heights.at_bottom, self.prefs.read_receipts);
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
                    // A changed name goes into the saved catalog too.
                    self.recorded = false;
                }
            }
            if let Some(messages) = &mut session.messages
                && messages.set_hide_redacted(self.prefs.hide_redacted)
            {
                // A room opened just now is projected for the first time here (the default is «show»): its saved reading position must survive.
                let restore = session.heights.restore.take();
                session.heights = Default::default();
                session.heights.restore = restore;
            }
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
        // macOS: the content runs under the transparent titlebar; paint that strip like the bars below it.
        #[cfg(target_os = "macos")]
        egui::Panel::top("titlebar").exact_size(28.).resizable(false).show_separator_line(false).frame(egui::Frame::NONE.fill(rgb(t.surface))).show(ui, |ui| {
            // The strip covers the native titlebar, so the clicks land here: hand the drag and the double-click zoom back to the window.
            let bar = ui.interact(ui.max_rect(), ui.id().with("drag"), egui::Sense::click_and_drag());
            if bar.double_clicked() {
                let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            } else if bar.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        });
        if self.session.as_ref().is_some_and(|s| s.account.is_some() && !s.recovering()) {
            self.shell(ui, &t);
        } else {
            self.sign_in(ui, &t);
        }
        if let Some(s) = &mut self.session { s.show_viewer(&ctx); }
        if let Some(s) = &mut self.session && s.quick_open { quick_search(&ctx, s); }
        if let Some(url) = self.pending_web.clone() {
            let mut choice = None;
            let modal = egui::Modal::new(egui::Id::unique("confirm-web-link")).show(&ctx, |ui| {
                ui.heading("Apri link");
                ui.label(url.as_str());
                ui.horizontal(|ui| {
                    if ui.button("Annulla").clicked() { choice = Some(false); }
                    if ui.button("Apri").clicked() { choice = Some(true); }
                });
            });
            if choice == Some(true) {
                ctx.open_url(egui::OpenUrl::new_tab(url.as_str()));
            }
            if choice.is_some() || modal.should_close() {
                self.pending_web = None;
            }
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
        if self.auto_login.is_some() && s.recovering() && s.form.recovery_key().is_none() && std::env::var_os("GNOBBI_EGUI_KEEP_RECOVERY").is_none() {
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
            // GNOBBI_EGUI_ROOM=<name>: that room instead of the first.
            let wanted = std::env::var("GNOBBI_EGUI_ROOM").ok();
            let pick = |s: &Session| {
                let rooms = s.sidebar.rooms();
                let by_name = wanted.as_deref().and_then(|w| s.sidebar.visible_indices().iter().copied().find(|&i| rooms[i].name == w));
                by_name.or_else(|| s.sidebar.visible_indices().first().copied())
            };
            let target = wanted.as_deref().and_then(|w| s.sidebar.rooms().iter().find(|r| r.name == w).map(|r| r.scope.clone()));
            if (s.sidebar.selected().is_none() || target.as_ref().is_some_and(|t| s.sidebar.selected().map(|r| &r.scope) != Some(t)))
                && let Some(first) = pick(s)
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
            // Views «sp-menu», «sp-info», «sp-members», «sp-add»: a space (the one named in GNOBBI_EGUI_ROOM, else the first) is picked in the rail, then its menu or window is shown.
            if let Some(page) = view.strip_prefix("sp-") {
                let first = s.sidebar.spaces().find(|r| wanted.as_deref().is_none_or(|w| r.name == w)).map(|r| r.scope.clone());
                if s.sidebar.active_space().is_none() && first.is_some() {
                    s.sidebar.set_space(first);
                }
                // «sp-exit»: the confirmation window once the space may be left; GNOBBI_EGUI_CONFIRM=forget|keep confirms it, once.
                if page == "exit" && !self.hook_member && let Some(scope) = s.sidebar.active_space().cloned() {
                    match &s.space_facts {
                        None => s.space_facts_for(scope),
                        Some((_, Some(c))) if space::leave_block(s.sidebar.rooms(), &scope, c).is_none() => {
                            self.hook_member = true;
                            match std::env::var("GNOBBI_EGUI_CONFIRM") {
                                Ok(how) => s.leave_space(scope, how == "forget"),
                                Err(_) => s.space_exit = Some((scope, false)),
                            }
                        }
                        _ => {}
                    }
                }
                if !matches!(page, "menu" | "exit") && s.space_view.is_none() && let Some(scope) = s.sidebar.active_space().cloned() {
                    s.open_space(scope, page == "members");
                }
                if page == "add" && let Some(v) = &mut s.space_view && matches!(v.children, Some(Ok(_))) && v.adding.is_none() {
                    v.adding = Some(None);
                }
            }
            // GNOBBI_EGUI_MEMBER=kick|ban|invite|unban: that action on the first other (or first banned) member, once the rights and the lists are in.
            // With GNOBBI_EGUI_CONFIRM the «Espelli»/«Blocca» window is confirmed too (reason from GNOBBI_EGUI_REASON).
            if !self.hook_member
                && let Ok(what) = std::env::var("GNOBBI_EGUI_MEMBER")
                && let Some(a) = s.admin.as_ref().filter(|a| a.perms.can_kick || a.perms.can_invite)
            {
                let list = s.space_view.as_ref().and_then(|v| v.members.as_ref()).or(s.members.as_ref().map(|(_, m)| m));
                let act = match (what.as_str(), list.and_then(|l| l.iter().find(|m| m.user_id != s.user)), a.banned.as_ref().ok().and_then(|b| b.first())) {
                    ("invite", _, _) => Some(members::Act::Invite),
                    ("kick" | "ban", Some(m), _) => Some(members::Act::Ask { user_id: m.user_id.clone(), name: m.name.clone(), ban: what == "ban" }),
                    ("unban", _, Some(b)) => Some(members::Act::Unban(b.user_id.clone())),
                    _ => None,
                };
                if let Some(act) = act {
                    self.hook_member = true;
                    s.member_act(act);
                    // GNOBBI_EGUI_TERM=<text>: typed in the «Invita» search.
                    if what == "invite" { s.new_term = std::env::var("GNOBBI_EGUI_TERM").unwrap_or_default(); }
                    if let Some(a) = &mut s.admin
                        && let Some(d) = &mut a.dialog
                    {
                        d.reason = std::env::var("GNOBBI_EGUI_REASON").unwrap_or_default();
                        if std::env::var_os("GNOBBI_EGUI_CONFIRM").is_some() {
                            a.busy = true;
                            use matrix_engine::operations::Moderation;
                            let (scope, user, action) = (a.scope.clone(), d.user_id.clone(), if d.ban { Moderation::BanWith(d.reason.clone()) } else { Moderation::KickWith(d.reason.clone()) });
                            s.moderate_in(scope, user, action);
                        }
                    }
                }
            }
            match view.as_str() {
                "info" => s.info_open = true,
                "search" => s.search_open = true,
                "devices" => s.devices_open = true,
                "new" => s.new_open = true,
                "space" | "space-public" => {
                    s.new_open = true;
                    self.new_form.open_space(view == "space-public");
                }
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
        // winit cannot unmap a Wayland window (Visible(false) is a no-op), so there a close request must quit.
        let reachable = self.tray.as_ref().is_some_and(|t| t.host_present()) && !wayland_session();
        // Ctrl/Cmd+Q is a real quit (not hide-to-tray), also while a TextEdit has the focus; Super+Q reaches us as a close request.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let actions = self.tray.as_ref().map(|tray| tray.actions()).unwrap_or_default();
        if !actions.is_empty() {
            for action in actions {
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
                    TrayAction::SwitchAccount(id) => {
                        if let Some(dirs) = &self.dirs {
                            let catalog = matrix_desktop::AccountCatalog::load(dirs);
                            if let Some(path) = catalog.data_dir(dirs, id) {
                                if let Some(i) = self.parked.iter().position(|p| p.root == path) { self.switch_to(i); }
                                show(ctx);
                                self.window_visible = true;
                            }
                        }
                    }
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
            let mut rows = Vec::new();
            if let Some(dirs) = &self.dirs {
                let catalog = matrix_desktop::AccountCatalog::load(dirs);
                for entry in catalog.accounts() {
                    let Some(path) = catalog.data_dir(dirs, entry.id) else { continue };
                    let current = path == self.root;
                    let session = if current { self.session.as_ref() } else { self.parked.iter().find(|p| p.root == path).map(|p| &p.session) };
                    let Some(session) = session else { continue };
                    let unread = session.sidebar.navigation_entries().first().map_or(0, |e| e.unread);
                    // The server in brackets tells apart accounts with the same name (Marco, 19/11); the unread count follows after a dot.
                    let server = entry.user_id.as_deref().and_then(|u| u.split_once(':')).map(|(_, s)| s);
                    let label = match server { Some(s) => format!("{} ({s})", session.account_name), None => session.account_name.clone() };
                    let name = if unread > 0 { format!("{label} · {unread}") } else { label };
                    rows.push(matrix_desktop::AccountRow { id: entry.id, initials: matrix_desktop::initials(&session.account_name), name, user_id: session.user.clone(), avatar: None, current, unread, mentions: 0 });
                }
            }
            let unread = rows.iter().fold(0u64, |n, row| n.saturating_add(row.unread));
            let items = tray_menu(self.window_visible, self.prefs.notifications, &rows);
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
        // The shown account sits on top of the spaces rail: a click lists the others, each with a dot below it (filled = unread).
        let mut pick_account = None;
        let narrow = ui.ctx().content_rect().width() < NARROW_WIDTH;
        if !narrow {
            self.list_open = false;
        }
        let unread = |s: &Session| s.sidebar.navigation_entries().first().map_or(0, |e| e.unread);
        let others: Vec<(String, u64)> = self.parked.iter().map(|p| (p.session.account_name.clone(), unread(&p.session))).collect();
        // The catalog lists the account once its login is recorded; until then the layout is the default one, kept under 0.
        if self.recorded && self.rail_account.as_ref().is_none_or(|(root, _)| *root != self.root)
            && let Some(d) = &self.dirs
        {
            self.rail_account = matrix_desktop::AccountCatalog::load(d).id_for_root(d, &self.root).map(|id| (self.root.clone(), id.as_u64()));
        }
        let account = self.rail_account.as_ref().filter(|(root, _)| *root == self.root).map_or(0, |(_, id)| *id);
        if let Some(session) = &mut self.session {
            rail::edit_window(ui.ctx(), &mut session.sidebar, &mut self.rail, account, self.dirs.as_ref(), t);
            egui::Panel::left("rail").exact_size(rail::WIDTH).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| {
                ui.add_space(8.);
                // Five dots per row fit the rail; more accounts wrap onto further rows, each centred.
                let dots = others.len().div_ceil(5) as f32 * 10.;
                let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(rail::WIDTH, 52. + dots), if others.is_empty() { egui::Sense::hover() } else { egui::Sense::click() });
                let name = session.account_name.as_str();
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                if !others.is_empty() && resp.hovered() {
                    ui.painter().rect_filled(rect.shrink2(egui::Vec2::new(6., 2.)), 10., rgb(t.input));
                }
                let c = rect.center_top() + egui::Vec2::new(0., 26.);
                let initial: String = name.trim_start_matches('@').chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned());
                ui.painter().circle_filled(c, 18., rgb(AVATAR));
                ui.painter().text(c, egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(16.), rgb(t.on_accent));
                if let Some(tex) = session.own_picture() {
                    let r = egui::Rect::from_center_size(c, egui::Vec2::splat(36.));
                    egui::Image::new(&tex).fit_to_exact_size(r.size()).corner_radius(egui::CornerRadius::same(18)).paint_at(ui, r);
                }
                for (i, (_, n)) in others.iter().enumerate() {
                    let in_row = (others.len() - i / 5 * 5).min(5);
                    let d = egui::Pos2::new(c.x + ((i % 5) as f32 - (in_row - 1) as f32 / 2.) * 10., rect.top() + 55. + (i / 5) as f32 * 10.);
                    if *n > 0 {
                        ui.painter().circle_filled(d, 3., rgb(t.accent));
                    } else {
                        ui.painter().circle_stroke(d, 2.5, egui::Stroke::new(1., rgb(t.secondary)));
                    }
                }
                if !others.is_empty() {
                    // Test hook: GNOBBI_EGUI_VIEW=account-menu shows the list without a click.
                    if std::env::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("account-menu") {
                        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
                    }
                    egui::Popup::menu(&resp).align(egui::RectAlign::RIGHT_START).show(|ui| {
                        for (i, (other, n)) in others.iter().enumerate() {
                            let count = if *n > 0 { n.to_string() } else { String::new() };
                            if ui.add(egui::Button::new(other.as_str()).right_text(egui::RichText::new(count).color(rgb(t.accent)))).clicked() {
                                pick_account = Some(i);
                                ui.close();
                            }
                        }
                    });
                }
                resp.on_hover_text(name);
                rail::show(ui, &mut session.sidebar, &session.avatars, &mut self.rail, account, t);
            });
            if !narrow || self.list_open {
                let before = session.sidebar.selected().map(|r| r.scope.clone());
                // Narrow: the list takes the whole width left of the rail and the chat is not drawn (shortcut: re-clicking the open room does not close it).
                let list_fill = list_fill(t);
                if narrow {
                    egui::Panel::left("list-narrow").exact_size(ui.available_width()).resizable(false).show_separator_line(false).frame(bar(t).fill(list_fill)).show(ui, |ui| {
                        let mut new_chat = false;
                        list::rooms(ui, session, &mut new_chat, t);
                        session.new_open |= new_chat;
                    });
                } else {
                    let max = list_max_width(ui.ctx().content_rect().width());
                    let resize_id = ui.scope_id().with_salt(egui::IdSalt::new("list")).with("__resize");
                    let hovered = ui.style().visuals.widgets.hovered.fg_stroke;
                    let active = ui.style().visuals.widgets.active.fg_stroke;
                    let highlight = Stroke::NONE;
                    ui.style_mut().visuals.widgets.hovered.fg_stroke = highlight;
                    ui.style_mut().visuals.widgets.active.fg_stroke = highlight;
                    let response = egui::Panel::left("list")
                        .default_size(self.prefs.list_width.map_or(t.sidebar, |w| w as f32))
                        .size_range(240. ..=max).resizable(true).show_separator_line(false)
                        .frame(bar(t).fill(list_fill)).show(ui, |ui| {
                            ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                            ui.style_mut().visuals.widgets.active.fg_stroke = active;
                            let mut new_chat = false;
                            list::rooms(ui, session, &mut new_chat, t);
                            session.new_open |= new_chat;
                        });
                    ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                    ui.style_mut().visuals.widgets.active.fg_stroke = active;
                    if let Some(handle) = ui.read_response(resize_id)
                        && handle.drag_stopped()
                    {
                        let width = handle.interact_pointer_pos().map_or(response.response.rect.width(), |p| p.x - response.response.rect.left()).clamp(240., max).round() as u32;
                        self.prefs.list_width = Some(width);
                        if let Some(dirs) = &self.dirs
                            && let Err(error) = Preferences::update(dirs, |p| p.list_width = Some(width))
                        {
                            eprintln!("Gnobbi: list_width {:?}", error.kind());
                        }
                    }
                }
                if narrow && session.sidebar.selected().map(|r| r.scope.clone()) != before {
                    self.list_open = false;
                }
                if narrow {
                    return;
                }
            }
        }
        if let Some(i) = pick_account {
            self.switch_to(i);
        }
        let has_room = self.session.as_ref().is_some_and(|s| s.messages.is_some());
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        let badge = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| (r.initials.clone(), r.color)));
        let head_pic = self.session.as_ref().and_then(|s| s.sidebar.selected().and_then(|r| r.info.avatar.as_deref()).and_then(|a| s.avatars.get(a)?.clone()));
        egui::Panel::top("header").exact_size(t.header).show_separator_line(false).frame(bar(t)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(16.);
                if narrow && ui.button("☰").clicked() {
                    self.list_open = true;
                }
                if let Some((initials, color)) = &badge {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(32.), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 16., rgb(*color));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, egui::FontId::proportional(13.), rgb(t.avatar_text));
                    if let Some(tex) = &head_pic {
                        egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(egui::CornerRadius::same(16)).paint_at(ui, rect);
                    }
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
            egui::Panel::right("info").exact_size(300.).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| out = info::show(ui, room, members, s.admin.as_ref().filter(|a| a.scope == room.scope), &s.user, &mut view, t));
            alias_action = view.action.take();
            self.alias_input = alias_input;
            if let (Some(action), Some(s)) = (alias_action, &mut self.session) {
                s.alias_action(action);
            }
            if let (Some(act), Some(s)) = (out.member, &mut self.session) {
                s.member_act(act);
            }
            if out.edit && let Some(s) = &mut self.session {
                s.open_edit();
            }
        }
        if let Some(s) = &mut self.session
            && s.info_open
            && let Some((id, profile)) = s.profile.clone()
        {
            let (mut close, mut flip, mut moderation, mut action) = (false, None, None, None);
            let (rights, failed) = (s.rights.clone(), s.moderation_failed);
            let me = id == s.user;
            let can_verify = s.verification.as_ref().is_some_and(|v| v.can_begin());
            egui::Panel::right("member").exact_size(300.).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| flip = info::card(ui, &id, profile.as_ref(), rights.as_ref(), failed, s.new_failed, me, can_verify, &mut action, &mut self.confirm, &mut moderation, &mut close, t));
            match action {
                Some(info::ProfileAction::Message) => s.start_dm(id.clone()),
                Some(info::ProfileAction::Mention) => {
                    if let Some(p) = &profile {
                        s.mention_profile(id.clone(), &p.name);
                        let composer = egui::Id::unique("composer-text");
                        let mut state = egui::TextEdit::load_state(ui.ctx(), composer).unwrap_or_default();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(s.draft.chars().count()))));
                        state.store(ui.ctx(), composer);
                        ui.ctx().memory_mut(|m| m.request_focus(composer));
                        s.info_open = false;
                        close = true;
                    }
                }
                Some(info::ProfileAction::Verify) => { if let Some(v) = &mut s.verification { v.begin_user(id.clone()); } }
                Some(info::ProfileAction::ReadReceipt) => {
                    if let Some(event) = rights.as_ref().and_then(|r| r.read_receipt.as_ref()) {
                        s.search_jump = Some((matrix_domain::EventId::new(event), false));
                        s.notice = None;
                        s.info_open = false;
                        close = true;
                    }
                }
                None => {}
            }
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
            let mut selected = None;
            let mut more = false;
            egui::Panel::right("search").exact_size(300.).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| selected = info::search(ui, &mut s.search_term, &state, &mut more, t));
            if more { s.more_messages(); }
            if selected.is_some() {
                s.search_jump = selected;
                s.notice = None;
            }
        }
        if let Some(s) = &mut self.session
            && let Some(mut th) = s.thread.take()
        {
            let (mut close, mut body) = (false, None);
            let w = self.prefs.thread_width.map_or(320., |w| w as f32);
            let hovered = ui.style().visuals.widgets.hovered.fg_stroke;
            let active = ui.style().visuals.widgets.active.fg_stroke;
            ui.style_mut().visuals.widgets.hovered.fg_stroke = Stroke::NONE;
            ui.style_mut().visuals.widgets.active.fg_stroke = Stroke::NONE;
            let shown = egui::Panel::right("thread").default_size(w).size_range(260. ..=640.).resizable(true).show_separator_line(false).frame(bar(t)).show(ui, |ui| {
                ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                ui.style_mut().visuals.widgets.active.fg_stroke = active;
                body = info::thread(ui, &mut th, &mut close, t);
            });
            ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
            ui.style_mut().visuals.widgets.active.fg_stroke = active;
            // Saved once the drag is over, not on every frame of it.
            let now = shown.response.rect.width().round() as u32;
            if !ui.input(|i| i.pointer.any_down()) && self.prefs.thread_width.unwrap_or(320) != now && let Some(dirs) = &self.dirs {
                self.prefs.thread_width = Some(now);
                if let Err(e) = Preferences::update(dirs, |p| p.thread_width = Some(now)) { eprintln!("thread width not saved: {e}"); }
            }
            if !close {
                s.thread = Some(th);
            }
            if let Some(body) = body {
                s.send_thread(body);
            }
        }
        let mut to_send = None;
        composer::panel(bar(t)).show(ui, |ui| {
            if let Some(s) = &mut self.session
                && s.messages.is_some()
            {
                let mode = s.target.as_ref().map_or(composer::Mode::Send, |(_, m)| *m);
                let mut cancel = false;
                let mut drop_file = false;
                let mut open_poll = false;
                let mut picked_file = None;
                let mut pasted_image = None;
                let mut voice_act = None;
                let name = s.attachment.as_ref().map(|(n, _)| n.clone());
                if let Some(notice) = &s.notice {
                    ui.label(notice);
                }
                let members: Vec<_> = s.members.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
                to_send = composer::show(ui, "", &mut s.draft, mode, name.as_deref(), &mut cancel, &mut drop_file, &mut open_poll, &mut picked_file, &mut pasted_image, s.recorder.as_ref().map(|r| r.elapsed_secs()), &mut voice_act, &members, &mut s.mentions);
                if let Some(image) = pasted_image {
                    s.attach_image(&image);
                }
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
        if let Some(s) = &mut self.session {
            list::space_picker(ui.ctx(), s);
            space::window(ui.ctx(), s, t);
            space::exit_window(ui.ctx(), s, t);
            space::delete_window(ui.ctx(), s, t);
            members::windows(ui.ctx(), s, t);
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
            && let Some((rights, is_space)) = s.edit.as_ref().and_then(|e| s.sidebar.rooms().iter().find(|r| r.scope == e.scope)).map(|r| (r.info.edit_rights, r.info.is_space))
        {
            roomedit::show(ui.ctx(), s, rights, if is_space { "Modifica spazio" } else { "Modifica stanza" });
        }
        if self.accounts_open
            && let Some(dirs) = &self.dirs
        {
            match accounts::show(ui.ctx(), &mut self.accounts_open, dirs, &self.root, self.session.as_mut(), &mut self.account_form, t) {
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
            // The click that changes room lands before `drive_history` swaps the messages: drawing the old room under the new room's scroll state moved it to the bottom and saved that as its reading position.
            let stale = self.session.as_ref().is_some_and(|s| s.messages.as_ref().is_some_and(|m| s.sidebar.selected().is_some_and(|r| r.scope != *m.scope())));
            if stale {
                ui.ctx().request_repaint();
            }
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
                && !stale
            {
                s.flash = s.flash.take().filter(|(_, at)| at.elapsed() < std::time::Duration::from_millis(1500));
                if s.flash.is_some() { ui.ctx().request_repaint_after(std::time::Duration::from_millis(100)); }
                let mut acts = timeline::Acts { now_playing: s.audio_now(), flash: s.flash.as_ref().map(|(id, _)| id.clone()), ..Default::default() };
                let jump = s.search_jump.as_ref().and_then(|(id, _)| messages.events().iter().position(|e| e.id.id == matrix_domain::TimelineId::Server(id.clone())));
                let room = s.sidebar.selected();
                let members = room.map_or(3, |r| r.info.joined_members);
                let readers = room.map(|r| messages.readers_by_row(&r.info.public_read_receipts)).unwrap_or_default();
                // Scroll state per room: one shared id carried the old room's offset into the new one (jump on switch).
                let (first, menu, save, thread) = ui.push_id(room.map(|r| r.scope.room_id.as_str()), |ui| timeline::show(ui, room.map_or(0, |r| r.unread), messages.events(), messages, &mut s.heights, &s.pictures, &s.avatars, &mut acts, members, &readers, t, jump)).inner;
                if jump.is_some() && let Some((root, open_thread)) = s.search_jump.take() {
                    if open_thread { s.open_thread(root); } else { s.flash = Some((root, std::time::Instant::now())); }
                }
                // Not in memory: the same loop as a search hit loads older pages until it shows up.
                if let Some(id) = acts.reply_jump.take() { s.search_jump = Some((id, false)); }
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
                if let Some((id, video, name)) = acts.picture.take() { s.open_viewer(id, video, name); }
                if let Some(id) = acts.play.take() {
                    s.play_audio(id);
                }
                if let Some((id, ix)) = acts.vote {
                    s.vote(id, ix);
                }
                if acts.open_link.is_some() {
                    self.pending_web = acts.open_link;
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

/// Local conversations and server users use the same search models as the GPUI switcher.
fn quick_search(ctx: &egui::Context, s: &mut Session) {
    use matrix_desktop::quick_search::{Entry, Index};
    use matrix_engine::operations::SearchState;
    let rooms = s.sidebar.rooms();
    let count = rooms.len() as u64;
    let index = Index::new(rooms.iter().enumerate().filter(|(_, r)| !r.info.is_space && !r.info.invited).map(|(i, r)| Entry::from_room(r, count - i as u64)));
    let hits = quick_hits(&index, &s.new_term);
    let typed = !s.new_term.trim().is_empty();
    let users: Vec<_> = match &s.users {
        SearchState::Results { hits, .. } if typed => hits.iter().take(5).cloned().collect(),
        _ => Vec::new(),
    };
    let public: Vec<_> = match &s.public_rooms {
        matrix_engine::search::SearchState::Results { hits, .. } if typed => hits.iter().take(matrix_desktop::quick_search::ROOMS_LIMIT).cloned().collect(),
        _ => Vec::new(),
    };
    let total = hits.len() + users.len() + public.len();
    // Up/Down/Enter are taken before the field sees them; Esc closes through the modal.
    let (down, up, enter) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
    if total > 0 {
        if down { s.quick_sel = (s.quick_sel + 1) % total; }
        if up { s.quick_sel = (s.quick_sel + total - 1) % total; }
    }
    s.quick_sel = s.quick_sel.min(total.saturating_sub(1));
    let sel = s.quick_sel;
    let moved = down || up;
    let mut picked = if enter && total > 0 { Some(sel) } else { None };
    let modal = egui::Modal::new(egui::Id::unique("quick-search")).show(ctx, |ui| {
        ui.set_min_width(360.);
        ui.heading("Cerca");
        if ui.add(egui::TextEdit::singleline(&mut s.new_term).id(egui::Id::unique("quick-search-field")).hint_text("Cerca")).changed() {
            s.users = SearchState::Idle;
            s.public_rooms = matrix_engine::search::SearchState::Idle;
            s.quick_sel = 0;
        }
        egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
            let mut row = |ui: &mut egui::Ui, i: usize, text: String| {
                let r = ui.selectable_label(i == sel, text);
                if i == sel && moved { r.scroll_to_me(None); }
                if r.clicked() { picked = Some(i); }
            };
            if !hits.is_empty() { ui.label("Conversazioni"); }
            for (i, hit) in hits.iter().enumerate() { row(ui, i, hit.name.clone()); }
            if !users.is_empty() { ui.label("Utenti"); }
            for (i, hit) in users.iter().enumerate() {
                let name = hit.display_name.as_deref().unwrap_or(&hit.user_id);
                row(ui, hits.len() + i, format!("{name}  {}", hit.user_id));
            }
            if !public.is_empty() { ui.label("Stanze pubbliche"); }
            for (i, hit) in public.iter().enumerate() { row(ui, hits.len() + users.len() + i, hit.name.clone()); }
        });
    });
    if let Some(i) = picked {
        if let Some(hit) = hits.get(i) {
            let scope = s.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == hit.id).map(|r| r.scope.clone());
            if let Some(scope) = scope { s.sidebar.select(&scope); }
        } else if let Some(hit) = users.get(i - hits.len()) {
            s.start_dm(hit.user_id.clone());
        } else if let Some(hit) = public.get(i - hits.len() - users.len()) {
            let joined = s.sidebar.rooms().iter().find(|r| !r.info.invited && r.scope.room_id.as_str() == hit.room_id).map(|r| r.scope.clone());
            match joined {
                Some(scope) => { s.sidebar.select(&scope); }
                None => s.join_public(hit),
            }
        }
        s.quick_open = false;
    }
    if modal.should_close() { s.quick_open = false; }
}

fn quick_hits(index: &matrix_desktop::quick_search::Index, term: &str) -> Vec<matrix_desktop::quick_search::Hit> {
    index.search(term, 20)
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
                ui.collapsing("Reagisci", |ui| {
                    egui::Grid::new("reaction-emoji-grid").show(ui, |ui| {
                        for (i, (name, emoji)) in matrix_desktop::EMOJI_SET.iter().enumerate() {
                            if ui.button(*emoji).on_hover_text(*name).clicked() {
                                action = Some(*emoji);
                            }
                            if i % 8 == 7 { ui.end_row(); }
                        }
                    });
                });
            }
            if p.reply && ui.button("Rispondi").clicked() {
                action = Some("reply");
            }
            if p.reply && ui.button("Rispondi nel thread").clicked() {
                action = Some("thread");
            }
            if p.edit && ui.button("Modifica").clicked() {
                action = Some("edit");
            }
            let poll = s.ops.selection().and_then(|id| s.messages.as_ref()?.events().iter().find(|e| e.id == *id)?.poll.clone());
            if s.ops.can_end_poll(poll.as_ref()) {
                let label = if s.confirm_end_poll { "Conferma chiusura" } else { "Chiudi sondaggio" };
                if ui.button(label).clicked() {
                    if s.confirm_end_poll { action = Some("end"); } else { s.confirm_end_poll = true; }
                }
                if s.confirm_end_poll && ui.button("Annulla").clicked() { s.confirm_end_poll = false; }
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
        Some("thread") => {
            let root = s.ops.selection().and_then(|id| match &id.id {
                matrix_domain::TimelineId::Server(id) => Some(id.clone()),
                _ => None,
            });
            s.close_menu();
            if let Some(root) = root { s.open_thread(root); }
        }
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
    let accent = |text: &str| egui::Button::new(egui::RichText::new(text).color(list::rgb(t.on_accent))).fill(list::rgb(t.accent));
    if let Some(key) = s.form.recovery_key() {
        ui.label("Chiave di recupero Matrix");
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let copy = ui.button("⧉");
                copy.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Copia chiave"));
                if copy.clicked() { ui.ctx().copy_text(key.to_owned()); }
                ui.add(egui::Label::new(egui::RichText::new(key).monospace()).selectable(true));
            });
        });
        ui.add_space(8.);
        if ui.add_sized([ui.available_width(), 36.], accent("Ho salvato la chiave: continua")).clicked() {
            s.defer_recovery();
        }
        return;
    }
    if matches!(s.form.recovery(), Some(matrix_engine::signin::RecoveryState::Pending))
        || s.form.setup_phase() == Some(matrix_desktop::SetupPhase::Pending)
    {
        ui.spinner();
        return;
    }
    if s.form.recovery() != Some(matrix_engine::signin::RecoveryState::Unsupported) {
        let r = ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut s.phrase).hint_text("Chiave di recupero Matrix").password(true).align(egui::Align2::LEFT_CENTER));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.);
        if ui.add_sized([ui.available_width(), 36.], accent("Recupera cronologia cifrata")).clicked() || enter {
            s.recover();
        }
    } else if s.form.setup_offered() {
        let label = if s.form.setup_phase().is_some() { "Riprova a creare la chiave" } else { "Crea chiave di recupero" };
        if ui.add_sized([ui.available_width(), 36.], accent(label)).clicked() {
            s.setup_recovery();
        }
    }
    if (s.form.setup_phase().is_some() || matches!(s.form.recovery(), Some(matrix_engine::signin::RecoveryState::Failed(_))))
        && let Some(message) = s.form.error_message()
    {
        ui.colored_label(ui.visuals().error_fg_color, message);
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
    matrix_desktop::notifications::init();
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
            .with_min_inner_size([MIN_WIDTH, 420.])
            .with_title(TITLE)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
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
    fn list_width_bounds_and_persistence() {
        assert_eq!(list_max_width(772.), 336.);
        assert_eq!(list_max_width(1600.), 600.);
        let root = std::env::temp_dir().join(format!("gnobbi-list-width-{}", std::process::id()));
        let dirs = AppDirs { config: root.join("config"), data: root.join("data"), cache: root.join("cache") };
        Preferences::update(&dirs, |p| { p.list_width = Some(410); p.debug = true; }).unwrap();
        assert_eq!(Preferences::load(&dirs).list_width, Some(410));
        assert!(Preferences::load(&dirs).debug);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quick_search_uses_shared_normalized_index() {
        use matrix_desktop::quick_search::{Entry, Index};
        let index = Index::new([Entry { id: "!room:example.org".into(), name: "Caffè".into(), ..Default::default() }]);
        assert!(quick_hits(&index, "").is_empty());
        assert_eq!(quick_hits(&index, "caffe")[0].id, "!room:example.org");
        assert!(quick_hits(&index, "other").is_empty());
    }

    #[test]
    fn list_background_stays_distinct_from_rail_and_timeline() {
        for theme in Theme::ALL {
            let t = theme.tokens_with(matrix_desktop::Accent::Blue);
            let list = list_fill(&t);
            let rail = rgb(t.surface);
            let timeline = rgb(t.timeline);
            assert!(list.r() < rail.r() && list.g() < rail.g() && list.b() < rail.b());
            assert!(list.r().abs_diff(timeline.r()) >= 3 || list.g().abs_diff(timeline.g()) >= 3 || list.b().abs_diff(timeline.b()) >= 3, "{theme:?}: list {list:?}, timeline {timeline:?}");
        }
    }

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

#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) && std::env::var_os("WINIT_UNIX_BACKEND").is_none_or(|v| v != "x11")
}
#[cfg(not(target_os = "linux"))]
fn wayland_session() -> bool { false }
