#![recursion_limit = "256"]
//! Gnobbi, egui frontend. S1: shell with the saved theme and accent, window identity and the
//! single-instance gate (same profile, never two processes on it).
use matrix_desktop::i18n;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, ViewportBuilder, Visuals};
use matrix_desktop::{
    AppDirs, DesignTokens, FormStatus, Platform, Preferences, Theme,
    app_id::{APP_ID, TITLE},
};
use matrix_engine::RoomSyncStatus;
use std::time::Duration;

mod accounts;
mod avatar;
mod avatar_editor;
mod pick;
mod composer;
mod customize;
mod devices;
mod dir;
mod explore;
#[cfg(target_os = "linux")]
mod desktop_entry;
mod icons;
mod info;
mod join;
mod list;
mod members;
mod metrics;
mod mobile;
mod newchat;
mod poll;
mod rail;
mod rows;
mod roomedit;
mod space;
mod stickers;
mod secret_clip;
mod session;
mod suspend;
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

/// egui visuals from the shared design tokens.
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
const MIN_HEIGHT: f32 = 420.;
/// Below this (a phone held upright) there is one column: the list, or the chat without the rail, or a side panel in place of the chat.
const SINGLE_WIDTH: f32 = 480.;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side { Member, Info, Search, Thread }

/// One column: the side panel drawn in place of the chat, which is also the one «back» closes.
fn side_on_top(info: bool, member: bool, search: bool, thread: bool) -> Option<Side> {
    if info && member { Some(Side::Member) } else if info { Some(Side::Info) } else if search { Some(Side::Search) } else if thread { Some(Side::Thread) } else { None }
}

fn list_max_width(window_width: f32) -> f32 { (window_width - MIN_WIDTH).clamp(240., 600.) }

struct App {
    prefs: Preferences,
    dirs: Option<AppDirs>,
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
    tray: Option<TrayHandle>,
    /// macOS: the menu bar, built on the first frame (winit installs its own after launch) and kept alive here.
    #[cfg(target_os = "macos")]
    /// The macOS menu bar and the language it was built in: it is rebuilt when the language changes (hot switch).
    menubar: Option<(tray_icon::menu::Menu, &'static str)>,
    insets: mobile::Insets,
    quitting: bool,
    window_visible: bool,
    new_form: newchat::Form,
    device_fields: devices::Fields,
    account_form: accounts::Form,
    poll_form: poll::Form,
    confirm: Option<&'static str>,
    /// «Esci» on an account: the confirmation and what follows it.
    sign_out: Option<accounts::SignOut>,
    theme: Theme,
    tokens: DesignTokens,
    /// What `visuals` was last built for: rebuilt only when it changes.
    applied: Option<(Theme, u32, matrix_desktop::TextSize, matrix_desktop::Density, u32)>,
    /// OS text scale (1.0 when unknown), read at start and when the window regains focus.
    system_scale: f32,
    was_focused: bool,
    session: Option<Session>,
    /// Test hook: GNOBBI_EGUI_SHOT=<file.png> saves what the window paints after a few frames and exits.
    shot: Option<(std::path::PathBuf, std::time::Instant)>,
    /// Test hook: GNOBBI_EGUI_LOGIN=<json {homeserver,user,password,phrase?}> signs in by itself (disposable servers only).
    auto_login: Option<serde_json::Value>,
    /// The login hook submits once per account, not on every frame (an error leaves the form on its prompt).
    login_sent: bool,
    shot_sent: bool,
    pending_link: Option<(matrix_domain::MatrixLink, std::time::Instant)>,
    pending_web: Option<matrix_desktop::SafeWebUrl>,
    opening_web: Option<(matrix_desktop::SafeWebUrl, std::sync::mpsc::Receiver<bool>)>,
    failed_web: Option<matrix_desktop::SafeWebUrl>,
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
        matrix_desktop::i18n::apply(prefs.language, None);
        #[cfg(target_os = "ios")]
        if let Some(dirs) = dirs {
            matrix_desktop::push::init(dirs);
        }
        #[cfg(target_os = "linux")]
        if dirs.is_some() {
            desktop_entry::install_at_start(prefs.icon, prefs.desktop_entry_dont_ask);
        }
        let theme = Theme::Classic;
        let (session, start_error) = match dirs.map(|d| Session::start(d, root.clone(), trust.clone())) {
            Some(Ok(s)) => (Some(s), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, Some(i18n::t("Data folder not available").to_owned())),
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
        let mut app = App { prefs, dirs: dirs.cloned(), list_open: false, rail: rail::State { layouts: dirs.map(matrix_desktop::RailLayouts::load).unwrap_or_default(), ..Default::default() }, rail_account: None, root: root.clone(), recorded: false, parked, trust: trust.clone(), wake: None, tray: TrayHandle::start(), #[cfg(target_os = "macos")] menubar: None, insets: mobile::Insets::from_env(), quitting: false, window_visible: true, new_form: Default::default(), device_fields: Default::default(), account_form: Default::default(), poll_form: Default::default(), confirm: None, sign_out: None, theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, system_scale: metrics::system_scale(), was_focused: true, shot: std::env::var_os("GNOBBI_EGUI_SHOT").map(|p| (p.into(), std::time::Instant::now())),
            pending_link: link.map(|l| (l, std::time::Instant::now())), pending_web: None, opening_web: None, failed_web: None, shot_sent: false, hook_sent: false, hook_added: false, auto_login2: std::env::var_os("GNOBBI_EGUI_LOGIN2").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), hook_type: false, hook_alias: false, hook_member: false, alias_input: String::new(), hook_keys: false, hook_menu_at: None,
            auto_login: std::env::var_os("GNOBBI_EGUI_LOGIN").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), session, start_error, login_sent: false };
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
            self.rail.settings = false;
            self.rail.devices = false;
            self.rail.customize = false;
            if let Some(d) = &self.dirs {
                let _ = std::fs::write(d.data.join(LAST_ACCOUNT), self.root.to_string_lossy().as_bytes());
            }
        }
    }

    /// Starts every account again from its saved profile (after the background, which ended the workers).
    fn restart_accounts(&mut self) {
        let Some(d) = self.dirs.clone() else { return };
        if self.session.is_some() && let Ok(s) = Session::start(&d, self.root.clone(), self.trust.clone()) {
            self.session = Some(s);
        }
        for p in &mut self.parked {
            if let Ok(s) = Session::start(&d, p.root.clone(), self.trust.clone()) {
                p.session = s;
            }
        }
    }

    /// «Esci» on an account: confirm, revoke the device on the server, then delete the account from this device.
    fn sign_out_step(&mut self, ctx: &egui::Context) {
        let Some(so) = &mut self.sign_out else { return };
        let dir = so.dir.clone();
        match so.show(ctx) {
            None => {}
            Some(accounts::SignOutStep::Close) => self.sign_out = None,
            Some(accounts::SignOutStep::Start) => {
                let session = if dir == self.root { self.session.as_ref() } else { self.parked.iter().find(|p| p.root == dir).map(|p| &p.session) };
                if let Some(session) = session {
                    let control = session.control();
                    control.logout_and_stop();
                    so.wait_for(control);
                } else {
                    self.sign_out = None;
                }
            }
            // The worker was stopped for the logout: the account starts again from its saved profile.
            Some(accounts::SignOutStep::Restart) => {
                self.sign_out = None;
                if let Some(d) = &self.dirs
                    && let Ok(fresh) = Session::start(d, dir.clone(), self.trust.clone())
                {
                    if dir == self.root {
                        self.session = Some(fresh);
                    } else if let Some(p) = self.parked.iter_mut().find(|p| p.root == dir) {
                        p.session = fresh;
                    }
                }
            }
            Some(accounts::SignOutStep::Remove) => {
                self.sign_out = None;
                self.remove_account(&dir);
            }
        }
    }

    /// Takes the account out of the app and the catalog and deletes its files and vault items; no other account is touched.
    fn remove_account(&mut self, dir: &std::path::Path) {
        let Some(d) = self.dirs.clone() else { return };
        if dir == self.root {
            // The next remaining account comes on screen; with none left the sign-in screen starts on a fresh folder.
            if self.parked.is_empty() {
                let next = matrix_desktop::AccountCatalog::load(&d).next_root(&d);
                self.session = None;
                match Session::start(&d, next.clone(), self.trust.clone()) {
                    Ok(s) => (self.session, self.root, self.recorded) = (Some(s), next, false),
                    Err(e) => self.start_error = Some(e),
                }
            } else {
                let p = self.parked.remove(0);
                self.session = Some(p.session);
                (self.root, self.recorded) = (p.root, p.recorded);
                (self.new_form, self.device_fields, self.account_form, self.poll_form, self.confirm) = Default::default();
                self.rail.settings = false;
                self.rail.devices = false;
                self.rail.customize = false;
                let _ = std::fs::write(d.data.join(LAST_ACCOUNT), self.root.to_string_lossy().as_bytes());
            }
        } else {
            self.parked.retain(|p| p.root != dir);
        }
        let mut catalog = matrix_desktop::AccountCatalog::load(&d);
        let id = catalog.id_for_root(&d, dir);
        accounts::wipe(dir, id.and_then(|id| catalog.cache_dir(&d, id)));
        if let Some(id) = id && catalog.remove(id) {
            let _ = catalog.save(&d);
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
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.system_scale = metrics::system_scale();
        }
        self.was_focused = focused;
        self.theme = self.prefs.effective_theme(system_dark, matrix_desktop::local_hour());
        self.tokens = self.theme.tokens_with(self.prefs.effective_accent());
        let key = (self.theme, self.tokens.accent, self.prefs.text_size, self.prefs.density, self.system_scale.to_bits());
        if self.applied != Some(key) {
            ctx.set_visuals(visuals(&self.tokens, self.theme.is_dark()));
            metrics::apply(ctx, &self.prefs, self.system_scale);
            self.applied = Some(key);
        }
    }
}

fn chat_background(t: &DesignTokens) -> Color32 {
    if t.timeline == matrix_desktop::NIGHT.timeline { rgb(0x050a07) } else { rgb(t.timeline) }
}

fn bar(t: &DesignTokens) -> egui::Frame {
    egui::Frame::NONE.fill(list_fill(t))
}

fn list_fill(t: &DesignTokens) -> Color32 {
    if t.timeline == matrix_desktop::NIGHT.timeline {
        return rgb(0x0b130e);
    }
    let surface = rgb(t.surface);
    Color32::from_rgb(surface.r().saturating_sub(3), surface.g().saturating_sub(3), surface.b().saturating_sub(3))
}

impl eframe::App for App {
    /// Runs before every frame and also with the window minimized, occluded or hidden (then `ui` does not run):
    /// draining the backend here lets notifications go out with the window out of sight.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Back from the background: the workers were ended (store and locks released); start them again.
        if suspend::take_resumed() {
            self.restart_accounts();
        }
        if suspend::is_suspended() {
            return;
        }
        let Some(session) = &mut self.session else { return };
        // Out of sight nothing is being read: the flags of the last painted frame would still say "focused".
        if ctx.input(|i| i.viewport().minimized == Some(true) || i.viewport().occluded == Some(true)) {
            session.reading.0 = false;
        }
        // Catalog id and the other accounts' names: read once the account is signed in (the catalog file is small).
        if let Some(d) = &self.dirs {
            let mut catalog = None;
            for (s, root) in std::iter::once((&mut *session, &self.root)).chain(self.parked.iter_mut().map(|p| (&mut p.session, &p.root))) {
                if s.catalog_id.is_none() && s.account.is_some() {
                    s.catalog_id = catalog.get_or_insert_with(|| matrix_desktop::AccountCatalog::load(d)).id_for_root(d, root);
                    // The pusher of this account says which store the notification extension must open.
                    if let Some(id) = s.catalog_id && let Ok(dir) = root.strip_prefix(&d.data) {
                        matrix_engine::push::set_account(root.clone(), matrix_engine::push::AccountTag { catalog: id, dir: dir.to_string_lossy().into_owned() });
                    }
                }
            }
            let names: Vec<String> = std::iter::once(&session.account_name).chain(self.parked.iter().map(|p| &p.session.account_name)).cloned().collect();
            let ids: Vec<_> = std::iter::once(session.catalog_id).chain(self.parked.iter().map(|p| p.session.catalog_id)).flatten().collect();
            for (i, s) in std::iter::once(&mut *session).chain(self.parked.iter_mut().map(|p| &mut p.session)).enumerate() {
                s.peers = names.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, n)| n.clone()).collect();
            }
            // A click on the alert of an account that is gone: the window comes forward, one line says why.
            if ids.len() == names.len() && matrix_desktop::notifications::drop_early_click_of_gone_account(&ids) {
                session.notice = Some(i18n::t("Account no longer connected").to_owned());
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
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

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.insets.apply(raw);
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
                    _ if self.rail.accounts && session.profile_caps.is_some_and(|c| c.1) => self.account_form.pick(path.clone(), &self.root, &ctx),
                    _ => session.attach(path, false),
                }
            }
            if session.account.is_some() && !session.recovering() {
                let command = |key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key));
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::E)) && !session.new_open && !session.quick_open && !self.rail.settings {
                    session.open_explore();
                } else if command(egui::Key::K) && !session.new_open && !session.explore.is_some() && !self.rail.settings {
                    session.quick_open = !session.quick_open; session.quick_sel = 0;
                    session.new_term.clear();
                    if session.quick_open { ctx.memory_mut(|m| m.request_focus(egui::Id::unique("quick-search-field"))); }
                } else if !session.quick_open && !session.new_open && session.explore.is_none() && !self.rail.settings {
                    if command(egui::Key::F) {
                        session.focus_search = true;
                    } else if command(egui::Key::Comma) {
                        self.rail.set_settings(true);
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
                    && !session.new_open && session.explore.is_none() && !session.poll_open && session.edit.is_none()
                    && !(self.rail.devices && session.verification.as_ref().is_some_and(devices::modal_open)) && self.pending_web.is_none() && self.sign_out.is_none() && !self.account_form.editing()
                {
                    if self.rail.any_drawer() { (self.rail.edit, self.rail.settings, self.rail.devices, self.rail.accounts, self.rail.customize) = (false, false, false, false, false); }
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
            if self.rail.settings || self.rail.accounts || self.rail.customize || self.pending_web.is_some() || session.new_open || session.explore.is_some() || session.quick_open || session.poll_open || session.edit.is_some() || self.rail.devices || session.viewer.is_some() {
                if let Some(thread) = &mut session.thread { thread.tail_visible = false; }
            }
            // Narrow layout with the chat list on top: the selected room is not on screen.
            let room_hidden = self.list_open && ctx.content_rect().width() < NARROW_WIDTH;
            session.reading = (ctx.input(|i| i.focused) && !room_hidden, session.heights.at_bottom, self.prefs.read_receipts);
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
                && {
                    let hide = session.status_choices.get(messages.scope().room_id.as_str()).hides(self.prefs.show_status);
                    // Not `||`: both projections must run.
                    messages.set_hide_redacted(self.prefs.hide_redacted) | messages.set_hide_status(hide)
                }
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
        let ui = &mut mobile::safe_ui(ui, list_fill(&t));
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
        #[cfg(debug_assertions)]
        if std::env::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("avatar-editor") { self.account_form.demo(&ctx, &t); }
        if let Some(s) = &mut self.session && s.quick_open { quick_search(&ctx, s, &t); }
        if let Some(url) = self.pending_web.clone() {
            let mut choice = None;
            let modal = egui::Modal::new(egui::Id::unique("confirm-web-link")).show(&ctx, |ui| dir::scope(ui, |ui| {
                ui.heading(i18n::t("Open link"));
                ui.label(url.as_str());
                ui.horizontal(|ui| {
                    if ui.button(i18n::t("Cancel")).clicked() { choice = Some(false); }
                    if ui.button(i18n::t("Open")).clicked() { choice = Some(true); }
                });
            }));
            if choice == Some(true) {
                self.opening_web = Some((url.clone(), matrix_desktop::open_url(&url)));
            }
            if choice.is_some() || modal.should_close() {
                self.pending_web = None;
            }
        }
        if let Some((url, rx)) = &self.opening_web {
            match rx.try_recv() {
                Ok(true) => self.opening_web = None,
                Ok(false) => { self.failed_web = Some(url.clone()); self.opening_web = None; }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(_) => self.opening_web = None,
            }
        }
        if let Some(url) = self.failed_web.clone() {
            let mut done = false;
            let modal = egui::Modal::new(egui::Id::unique("failed-web-link")).show(&ctx, |ui| dir::scope(ui, |ui| {
                ui.heading(i18n::t("Could not open the link."));
                ui.label(url.as_str());
                ui.horizontal(|ui| {
                    if ui.button(i18n::t("Copy link")).clicked() { ctx.copy_text(url.as_str().to_owned()); done = true; }
                    if ui.button(i18n::t("Close")).clicked() { done = true; }
                });
            }));
            if done || modal.should_close() { self.failed_web = None; }
        }
    }
}

impl App {
    /// Test hooks (env only): sign in with the given account, then open the view named in GNOBBI_EGUI_VIEW.
    fn test_hooks(&mut self, ctx: &egui::Context) {
        // GNOBBI_EGUI_PICK=file|image|share: opens that system dialog once (from frame 60) and prints the answer; for a simulator run.
        if let Ok(kind) = std::env::var("GNOBBI_EGUI_PICK") {
            static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if ctx.cumulative_pass_nr() >= 60 && !STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                match kind.as_str() {
                    "share" => {
                        let p = pick::save_dir().join("prova.txt");
                        let _ = std::fs::write(&p, b"gnobbi share sheet test");
                        pick::share(&p);
                    }
                    k => pick::start(ctx, "hook", k == "image"),
                }
            }
            if let Some(p) = pick::done("hook") {
                eprintln!("gnobbi: picked {} ({} bytes)", p.display(), std::fs::metadata(&p).map_or(0, |m| m.len()));
            }
        }
        // GNOBBI_EGUI_ADD=1: once the first account is in the catalog, add a second one (signed in by GNOBBI_EGUI_LOGIN2).
        if !self.hook_added
            && self.recorded
            && std::env::var_os("GNOBBI_EGUI_ADD").is_some()
            && let Some(d) = &self.dirs
        {
            self.hook_added = true;
            self.login_sent = false;
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
            && take_once(&mut self.login_sent)
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
            // View «thread»: the drawer of the room's first thread; GNOBBI_EGUI_THREAD_TYPE=<text> is typed into its composer.
            if view == "thread"
                && s.thread.is_none()
                && let Some(matrix_domain::TimelineId::Server(root)) = s.messages.as_ref().and_then(|m| m.events().iter().find(|e| e.thread.is_some()).map(|e| e.id.id.clone()))
            {
                s.open_thread(root);
                if let (Some(th), Ok(text)) = (&mut s.thread, std::env::var("GNOBBI_EGUI_THREAD_TYPE")) { th.draft = text; }
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
            // GNOBBI_EGUI_MEMBER=profile|role: the card of the first other member, or of the last one, who is made «Moderatore» once the rights are in.
            if !self.hook_member
                && let Ok(what @ ("profile" | "role")) = std::env::var("GNOBBI_EGUI_MEMBER").as_deref()
                && let Some(user) = s.members.as_ref().and_then(|(_, l)| {
                    let mut others = l.iter().filter(|m| m.user_id != s.user);
                    if what == "role" { others.next_back() } else { others.next() }.map(|m| m.user_id.clone())
                })
            {
                if s.profile.is_none() {
                    s.open_profile(user);
                    self.hook_member = what == "profile";
                } else if s.rights.is_some() {
                    self.hook_member = true;
                    s.moderate(user, matrix_engine::operations::Moderation::SetPower(50));
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
            // Views «join-<state>» / «qjoin-<state>» (state: go, slow, missing, offline, unreachable, generic): «Unisciti» / the quick search with a join seeded in that state; nothing is sent.
            if let Some((quick, kind)) = view.strip_prefix("join-").map(|k| (false, k)).or(view.strip_prefix("qjoin-").map(|k| (true, k))) {
                use matrix_engine::{join::JoinPhase, operations::OperationError};
                let demo = "#grande:matrix.org";
                let mut join = join::JoinState::default();
                join.start();
                match kind {
                    "slow" => join.apply(JoinPhase::Slow, true),
                    "missing" => join.apply(JoinPhase::Refused(OperationError::Unavailable), true),
                    "offline" => join.apply(JoinPhase::Unconfirmed, false),
                    "unreachable" => join.apply(JoinPhase::Unconfirmed, true),
                    "generic" => join.apply(JoinPhase::Refused(OperationError::Forbidden), true),
                    _ => {}
                }
                s.joins.insert(demo.to_owned(), join);
                if quick {
                    s.quick_open = true;
                    s.new_term = "grande".to_owned();
                    let hit = matrix_engine::operations::DirectoryRoom { room_id: demo.to_owned(), alias: None, name: "Stanza grande".to_owned(), topic: None, members: 40000, avatar_url: None, world_readable: false, guest_can_join: false, join_rule: "public".to_owned(), origin: "matrix.org".to_owned() };
                    s.public_rooms = matrix_engine::search::SearchState::Results { term: s.new_term.clone(), hits: vec![hit], more: matrix_engine::search::More::None };
                } else if !s.new_open {
                    s.new_open = true;
                    self.new_form.prefill_join(demo);
                }
            }
            match view.as_str() {
                "info" => s.info_open = true,
                "search" => s.search_open = true,
                "devices" => self.rail.set_devices(true),
                "new" => s.new_open = true,
                // Test hook (runs every frame, so only while closed): «Esplora stanze pubbliche», with a term when written «explore-<term>».
                v if v.starts_with("explore") && s.explore.is_none() => {
                    s.open_explore();
                    if let (Some(e), Some(term)) = (&mut s.explore, v.strip_prefix("explore-")) { e.set_term(term); }
                }
                "space" | "space-public" => {
                    s.new_open = true;
                    self.new_form.open_space(view == "space-public");
                }
                "poll" => s.poll_open = true,
                // «reply»: the reply banner over the composer; «message-menu»: the menu of the last message.
                "reply" | "message-menu" if s.target.is_none() && s.menu.is_none() => {
                    if let Some(id) = s.messages.as_ref().and_then(|m| m.events().last().map(|e| e.id.clone())) {
                        s.open_menu(id, ctx.content_rect().center());
                        if view == "reply" { s.begin(composer::Mode::Reply); }
                    }
                }
                "settings" => self.rail.set_settings(true),
                "accounts" => self.rail.set_accounts(true),
                "customize" => self.rail.set_customize(true),
                _ => {}
            }
        }
    }

    /// Tray clicks, the menu and icon state, and what closing the window does (hide when the tray can be reached).
    #[cfg(target_os = "ios")]
    fn tray(&mut self, _ctx: &egui::Context) {} // iOS: no tray, no close-to-tray

    #[cfg(not(target_os = "ios"))]
    fn tray(&mut self, ctx: &egui::Context) {
        use matrix_desktop::tray_menu::{CloseAction, TrayAction, close_action, tray_menu};
        let show = |ctx: &egui::Context| {
            #[cfg(target_os = "macos")]
            mac_app(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        };
        #[cfg(target_os = "macos")]
        if self.menubar.as_ref().is_none_or(|(_, lang)| *lang != matrix_desktop::i18n::current()) {
            self.menubar = Some((matrix_desktop::tray::mac_menubar(), matrix_desktop::i18n::current()));
        }
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
                    TrayAction::Settings => {
                        show(ctx);
                        self.window_visible = true;
                        self.rail.set_settings(true);
                    }
                    TrayAction::Edit(edit) => match edit_input(edit) {
                        EditInput::Request(cmd) => ctx.send_viewport_cmd(cmd),
                        EditInput::Key(key, modifiers) => ctx.input_mut(|i| {
                            i.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
                        }),
                    },
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
                    let name = crate::list::clean_single_line(&if unread > 0 { format!("{label} · {unread}") } else { label });
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
            Some(RoomSyncStatus::Ready) => i18n::t("Connected"),
            Some(RoomSyncStatus::AuthenticationFailed) => i18n::t("Sign-in failed"),
            Some(RoomSyncStatus::SyncFailed) => i18n::t("Sync interrupted"),
            _ => i18n::t("Connecting…"),
        };
        // The shown account sits on top of the spaces rail: a click lists the others, each with a dot below it (filled = unread).
        let mut pick_account = None;
        let width = ui.ctx().content_rect().width();
        let narrow = width < NARROW_WIDTH;
        let single = width < SINGLE_WIDTH;
        if !narrow {
            self.list_open = false;
        }
        // Test hook: GNOBBI_EGUI_VIEW=list shows the list of a narrow window without a click.
        if narrow && std::env::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("list") {
            self.list_open = true;
        }
        // Left-edge swipe = the back button.
        if single && !self.list_open && mobile::back_swipe(ui.ctx()) {
            self.list_open = true;
        }
        let mut side = None;
        if let Some(s) = &mut self.session {
            let room = s.sidebar.selected().map(|r| r.scope.clone());
            // A room just created or joined is shown, not left behind the list (shortcut: a link or the quick search to a room already in the list leaves the list open; give `Session` an "opened" flag if that is ever met on a phone).
            let (pending, started) = (egui::Id::new("pending-room"), egui::Id::new("shell-started"));
            let waiting = s.pending_select.is_some();
            if !waiting && ui.data(|d| d.get_temp(pending)) == Some(true) {
                self.list_open = false;
            }
            // Started as one column (a phone): the list comes first, the room reopened from the last run stays behind it.
            if single && ui.data(|d| d.get_temp::<bool>(started)).is_none() {
                self.list_open = true;
            }
            ui.data_mut(|d| { d.insert_temp(pending, waiting); d.insert_temp(started, true); });
            if single {
                side = side_on_top(s.info_open && room.is_some(), s.profile.is_some(), s.search_open, s.thread.is_some());
                // «Back» closes the side panel on top before it shows the list.
                if self.list_open && room.is_some() && let Some(top) = side {
                    match top {
                        Side::Member => s.profile = None,
                        Side::Info => s.info_open = false,
                        Side::Search => s.search_open = false,
                        Side::Thread => s.thread = None,
                    }
                    self.list_open = false;
                    side = side_on_top(s.info_open, s.profile.is_some(), s.search_open, s.thread.is_some());
                }
                // Without a room there is nothing to show but the list.
                self.list_open |= room.is_none();
            }
        }
        let covered = side.is_some();
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
            rail::edit_window(ui.ctx(), session, &mut self.new_form, &mut self.rail, account, self.dirs.as_ref(), t);
            if !single || self.list_open {
                if single { mobile::rail_background(ui, chat_background(t)); }
                dir::panel_start("rail").exact_size(rail::WIDTH).resizable(false).show_separator_line(false).frame(egui::Frame::NONE.fill(chat_background(t))).show(ui, |ui| {
                    ui.add_space(8.);
                    // Five dots per row fit the rail; more accounts wrap onto further rows, each centred.
                    let dots = others.len().div_ceil(5) as f32 * 10.;
                    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(rail::WIDTH, 52. + dots), if others.is_empty() { egui::Sense::hover() } else { egui::Sense::click() });
                    let resp = crate::metrics::clickable(resp);
                    let name = session.account_name.as_str();
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                    if !others.is_empty() && resp.hovered() {
                        ui.painter().rect_filled(rect.shrink2(egui::Vec2::new(6., 2.)), 10., rgb(t.input));
                    }
                    let c = rect.center_top() + egui::Vec2::new(0., 26.);
                    let initial: String = name.trim_start_matches('@').chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned());
                    avatar::paint(ui, c, 36., rgb(AVATAR), &session.own_avatar(), &initial, 16., rgb(t.on_accent));
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
                        egui::Popup::menu(&resp).align(if dir::rtl() { egui::RectAlign::LEFT_START } else { egui::RectAlign::RIGHT_START }).width(280.).frame(egui::Frame::popup(ui.style()).inner_margin(8).corner_radius(10).stroke(egui::Stroke::NONE).fill(rgb(t.surface))).show(|ui| crate::dir::popup(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.;
                            // The id line tells apart accounts with the same name; a migrated one without id shows its homeserver.
                            let catalog = self.dirs.as_ref().map(matrix_desktop::AccountCatalog::load);
                            let line = |root: &std::path::Path| -> String {
                                let (Some(c), Some(d)) = (&catalog, &self.dirs) else { return String::new() };
                                c.accounts().iter().find(|e| c.data_dir(d, e.id).as_deref() == Some(root)).and_then(|e| e.user_id.clone().or_else(|| e.homeserver.clone())).unwrap_or_default()
                            };
                            let shown_row = (name.to_owned(), line(&self.root), session.own_avatar());
                            let rows: Vec<_> = self.parked.iter().map(|p| (p.session.account_name.clone(), line(&p.root), p.session.own_avatar())).collect();
                            accounts::switch_row(ui, &shown_row.0, &shown_row.1, &shown_row.2, true, 0, t);
                            for (i, ((other, id, pic), (_, n))) in rows.iter().zip(&others).enumerate() {
                                if accounts::switch_row(ui, other, id, pic, false, *n, t).clicked() {
                                    pick_account = Some(i);
                                    ui.close();
                                }
                            }
                        }));
                    }
                    resp.on_hover_text(name);
                    let placed = session.space_placed.take();
                    rail::show(ui, &mut session.sidebar, &session.avatars, &mut self.rail, account, placed, t);
                });
            }
            if !narrow || self.list_open {
                // Narrow: the list takes the whole width left of the rail and the chat is not drawn; a tap on a row, the open one too, goes to the chat.
                let list_fill = list_fill(t);
                if narrow {
                    dir::panel_start("list-narrow").exact_size(ui.available_width()).resizable(false).show_separator_line(false).frame(bar(t).fill(list_fill)).show(ui, |ui| dir::scope(ui, |ui| {
                        let mut new_chat = false;
                        // One column: the chat header has no room for the app's own windows, they sit in one menu here.
                        let (mut settings, mut accounts, mut devices) = (false, false, false);
                        if list::rooms(ui, session, &mut new_chat, t, &mut |ui| {
                            if !single { return; }
                            egui::Popup::menu(&icons::button(ui, icons::Icon::List, false, i18n::t("Menu"))).show(|ui| crate::dir::popup(ui, |ui| {
                                settings = ui.button(i18n::t("Settings")).clicked();
                                accounts = ui.button(i18n::t("Account")).clicked();
                                devices = ui.button(i18n::t("Devices")).clicked();
                                if settings || accounts || devices { ui.close(); }
                            }));
                        }) {
                            self.list_open = false;
                        }
                        session.new_open |= new_chat;
                        if settings { self.rail.set_settings(true); }
                        if accounts { self.rail.set_accounts(true); }
                        if devices { self.rail.set_devices(true); }
                    }));
                } else {
                    let max = list_max_width(ui.ctx().content_rect().width());
                    let resize_id = ui.scope_id().with_salt(egui::IdSalt::new("list")).with("__resize");
                    let hovered = ui.style().visuals.widgets.hovered.fg_stroke;
                    let active = ui.style().visuals.widgets.active.fg_stroke;
                    let highlight = Stroke::NONE;
                    ui.style_mut().visuals.widgets.hovered.fg_stroke = highlight;
                    ui.style_mut().visuals.widgets.active.fg_stroke = highlight;
                    let response = dir::panel_start("list")
                        .default_size(self.prefs.list_width.map_or(t.sidebar, |w| w as f32))
                        .size_range(240. ..=max).resizable(true).show_separator_line(false)
                        .frame(bar(t).fill(list_fill)).show(ui, |ui| {
                            ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                            ui.style_mut().visuals.widgets.active.fg_stroke = active;
                            let mut new_chat = false;
                            dir::scope(ui, |ui| list::rooms(ui, session, &mut new_chat, t, &mut |_| {}));
                            session.new_open |= new_chat;
                        });
                    ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                    ui.style_mut().visuals.widgets.active.fg_stroke = active;
                    if let Some(handle) = ui.read_response(resize_id)
                        && handle.drag_stopped()
                    {
                        let width = handle.interact_pointer_pos().map_or(response.response.rect.width(), |p| if dir::rtl() { response.response.rect.right() - p.x } else { p.x - response.response.rect.left() }).clamp(240., max).round() as u32;
                        self.prefs.list_width = Some(width);
                        if let Some(dirs) = &self.dirs
                            && let Err(error) = Preferences::update(dirs, |p| p.list_width = Some(width))
                        {
                            eprintln!("Gnobbi: list_width {:?}", error.kind());
                        }
                    }
                }
                if narrow {
                    self.overlays(ui, t);
                    return;
                }
            }
        }
        if let Some(i) = pick_account {
            self.switch_to(i);
        }
        let has_room = self.session.as_ref().is_some_and(|s| s.messages.is_some());
        let (info_on, search_on) = self.session.as_ref().map_or((false, false), |s| (s.info_open, s.search_open));
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        let badge = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| (r.initials.clone(), r.color)));
        let head_pic = self.session.as_ref().map(|s| avatar::lookup(&s.avatars, s.sidebar.selected().and_then(|r| r.info.avatar.as_deref())));
        egui::Panel::top("header").exact_size(t.header).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(if single { 4. } else { 16. });
                // One column: the list is the previous screen, so the button reads «back».
                if narrow && icons::button(ui, if single { icons::Icon::CaretLeft } else { icons::Icon::List }, false, if single { i18n::t("Back") } else { i18n::t("Chat list") }).clicked() {
                    self.list_open = true;
                }
                if let Some((initials, color)) = &badge {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(32.), egui::Sense::hover());
                    avatar::paint(ui, rect.center(), 32., rgb(*color), head_pic.as_ref().unwrap_or(&avatar::Avatar::Missing), initials, 13., rgb(t.avatar_text));
                    ui.add_space(4.);
                }
                ui.with_layout(dir::row_rev(egui::Align::Center), |ui| {
                    ui.add_space(if single { 4. } else { 12. });
                    if has_room && icons::button(ui, icons::Icon::Info, info_on, i18n::t("Info")).clicked()
                        && let Some(s) = &mut self.session
                    {
                        s.info_open = !s.info_open;
                        s.search_open = false;
                    }
                    if has_room && icons::button(ui, icons::Icon::MagnifyingGlass, search_on, i18n::t("Search in room")).clicked()
                        && let Some(s) = &mut self.session
                    {
                        s.search_open = !s.search_open;
                        s.info_open = false;
                    }
                    // Last, so the title gets what the buttons leave and is cut with an ellipsis instead of running under them.
                    ui.with_layout(dir::row(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(crate::list::clean_single_line(title.as_deref().unwrap_or(state))).truncate());
                    });
                });
            });
        }));
        if let Some(s) = &self.session
            && s.info_open
            && (!single || side == Some(Side::Info))
            && let Some(room) = s.sidebar.selected()
        {
            let members = s.members.as_ref().map(|(_, m)| m.as_slice());
            let mut out = info::Out::default();
            let mut alias_input = std::mem::take(&mut self.alias_input);
            let mut alias_action = None;
            let room_id = room.scope.room_id.as_str().to_owned();
            let mut status = s.status_choices.get(&room_id);
            let (status_was, hidden_status) = (status, s.messages.as_ref().filter(|m| m.scope() == &room.scope).map_or(0, |m| m.hidden_status_count()));
            let mut view = info::AliasView { data: s.aliases.as_ref(), error: s.alias_error, input: &mut alias_input, action: None };
            dir::panel_end("info").exact_size(if single { ui.available_width() } else { 300. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| out = info::show(ui, room, members, s.admin.as_ref().filter(|a| a.scope == room.scope), &s.user, &mut view, (&mut status, hidden_status), &s.avatars, t)));
            alias_action = view.action.take();
            self.alias_input = alias_input;
            if status != status_was && let Some(s) = &mut self.session {
                s.status_choices.set(&room_id, status);
            }
            if out.open_link.is_some() {
                self.pending_web = out.open_link.take();
            }
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
            let (rights, failed, busy) = (s.rights.clone(), s.moderation_failed, s.moderating);
            let me = id == s.user;
            let can_verify = s.verification.as_ref().is_some_and(|v| v.can_begin());
            dir::panel_end("member").exact_size(if single { ui.available_width() } else { 300. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| flip = info::card(ui, &id, profile.as_ref(), rights.as_ref(), failed, busy, s.new_failed, me, can_verify, &mut action, &mut self.confirm, &mut moderation, &mut close, t)));
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
            && (!single || side == Some(Side::Search))
        {
            let state = s.search.clone();
            let mut selected = None;
            let mut more = false;
            dir::panel_end("search").exact_size(if single { ui.available_width() } else { 300. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| selected = info::search(ui, &mut s.search_term, &state, &mut more, t)));
            if more { s.more_messages(); }
            if selected.is_some() {
                s.search_jump = selected;
                s.notice = None;
            }
        }
        if let Some(s) = &mut self.session
            && (!single || side == Some(Side::Thread))
            && let Some(mut th) = s.thread.take()
        {
            let (mut close, mut out) = (false, info::ThreadOut::default());
            let mut acts = timeline::Acts { now_playing: s.audio_now(), ..Default::default() };
            let members = s.sidebar.selected().map_or(3, |r| r.info.joined_members);
            let recording = s.recording(true);
            let w = self.prefs.thread_width.map_or(320., |w| w as f32);
            let hovered = ui.style().visuals.widgets.hovered.fg_stroke;
            let active = ui.style().visuals.widgets.active.fg_stroke;
            ui.style_mut().visuals.widgets.hovered.fg_stroke = Stroke::NONE;
            ui.style_mut().visuals.widgets.active.fg_stroke = Stroke::NONE;
            // Its own id in one column: the full width must not become the remembered width of the resizable panel.
            let panel = if single { dir::panel_end("thread-single").exact_size(ui.available_width()).resizable(false) } else { dir::panel_end("thread").default_size(w).size_range(260. ..=640.).resizable(true) };
            let shown = panel.show_separator_line(false).frame(bar(t)).show(ui, |ui| {
                ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                ui.style_mut().visuals.widgets.active.fg_stroke = active;
                out = info::thread(ui, &mut th, &s.pictures, &s.avatars, members, recording, &mut acts, &mut close, t);
            });
            ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
            ui.style_mut().visuals.widgets.active.fg_stroke = active;
            // Saved once the drag is over, not on every frame of it.
            let now = shown.response.rect.width().round() as u32;
            if !single && !ui.input(|i| i.pointer.any_down()) && self.prefs.thread_width.unwrap_or(320) != now && let Some(dirs) = &self.dirs {
                self.prefs.thread_width = Some(now);
                if let Err(e) = Preferences::update(dirs, |p| p.thread_width = Some(now)) { eprintln!("thread width not saved: {e}"); }
            }
            if !close {
                s.thread = Some(th);
            } else if recording.is_some() {
                s.stop_recording(false);
            }
            if let Some(image) = out.pasted { s.attach_image(&image, true); }
            if let Some(path) = out.picked { s.attach(&path, true); }
            match out.voice {
                Some(composer::VoiceAct::Start) => s.start_recording(true, ui.ctx()),
                Some(composer::VoiceAct::Send) => s.stop_recording(true),
                Some(composer::VoiceAct::Cancel) => s.stop_recording(false),
                None => {}
            }
            if let Some(body) = out.send { s.send_thread(body); }
            if let Some((id, name)) = out.save { s.save(id, &name); }
            if let Some((id, video, name)) = acts.picture.take() { s.open_viewer(id, video, name); }
            if let Some(id) = acts.play.take() { s.play_audio(id); }
            if acts.open_link.is_some() { self.pending_web = acts.open_link; }
        }
        let mut to_send = None;
        composer::panel(bar(t)).show(ui, |ui| dir::scope(ui, |ui| {
            if let Some(s) = &mut self.session
                && s.messages.is_some()
                && !covered
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
                let recording = s.recording(false);
                to_send = composer::show(ui, "", &mut s.draft, mode, name.as_deref(), &mut cancel, &mut drop_file, &mut open_poll, &mut picked_file, &mut pasted_image, recording, &mut voice_act, &members, &mut s.mentions, Some((&mut s.stickers, &s.avatars)), t);
                if let Some(image) = pasted_image {
                    s.attach_image(&image, false);
                }
                if drop_file {
                    s.attachment = None;
                }
                s.poll_open |= open_poll;
                if let Some(sticker) = s.stickers.chosen.take() {
                    s.send_sticker(sticker);
                }
                if let Some(path) = picked_file {
                    s.attach(&path, false);
                }
                match voice_act {
                    Some(composer::VoiceAct::Start) => s.start_recording(false, ui.ctx()),
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
        }));
        if let (Some(body), Some(s)) = (to_send, &mut self.session) {
            s.submit_draft(body);
        }
        self.overlays(ui, t);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(chat_background(t))).show(ui, |ui| dir::scope(ui, |ui| {
            // The click that changes room lands before `drive_history` swaps the messages: drawing the old room under the new room's scroll state moved it to the bottom and saved that as its reading position.
            // A side panel has taken the whole width: nothing is laid out in the sliver left.
            if covered {
                return;
            }
            let stale = self.session.as_ref().is_some_and(|s| s.messages.as_ref().is_some_and(|m| s.sidebar.selected().is_some_and(|r| r.scope != *m.scope())));
            if stale {
                ui.ctx().request_repaint();
            }
            let invite = self.session.as_ref().and_then(|s| s.sidebar.selected_invite().map(|r| (r.scope.clone(), r.name.clone())));
            if let (Some((scope, name)), Some(s)) = (invite, &mut self.session) {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.);
                    ui.heading(crate::list::clean_single_line(&name));
                    ui.add_space(12.);
                    ui.horizontal(|ui| {
                        if ui.button(i18n::t("Accept")).clicked() {
                            s.answer_invite(scope.clone(), true);
                        }
                        if ui.button(i18n::t("Decline")).clicked() {
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
                let (first, menu, save, thread) = ui.push_id(room.map(|r| r.scope.room_id.as_str()), |ui| timeline::show(ui, room.map_or(0, |r| r.unread), messages.events(), messages, &mut s.heights, &s.pictures, &s.avatars, &mut acts, members, &readers, t, jump, None)).inner;
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
        }));
    }

    /// The windows and drawers of the session (settings, account, devices, new chat…). Drawn in every layout: the one-column list returns before the chat.
    fn overlays(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        if let Some(s) = &mut self.session {
            message_menu(ui.ctx(), s, t);
        }
        if let Some(s) = &mut self.session
            && s.new_open
        {
            newchat::show(ui.ctx(), s, &mut self.new_form, t);
        }
        if let Some(s) = &mut self.session {
            explore::show(ui.ctx(), s, t);
        }
        if let Some(s) = &mut self.session {
            list::space_picker(ui.ctx(), s);
            space::window(ui.ctx(), s, t);
            space::exit_window(ui.ctx(), s, t);
            space::delete_window(ui.ctx(), s, t);
            members::windows(ui.ctx(), s, t);
        }
        secret_clip::tick(ui.ctx());
        let account = self.session.as_ref().and_then(|s| s.account);
        if let Some(s) = &mut self.session
            && let Some(v) = &mut s.verification
        {
            // Debug builds only (screenshots): a fixed list of other devices, every frame so the engine's own answer cannot replace it.
            #[cfg(debug_assertions)]
            if let (Some(account_id), true) = (account, std::env::var_os("GNOBBI_EGUI_FAKE_DEVICES").is_some()) {
                let d = |id: &str, name: Option<&str>, verified, ago_min: u64, ip: &str| matrix_engine::verification::VerificationDevice {
                    user_id: String::new(),
                    device_id: id.into(),
                    verified,
                    display_name: name.map(Into::into),
                    last_seen_ts: Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |n| n.as_millis() as u64) - ago_min * 60_000),
                    last_seen_ip: Some(ip.into()),
                };
                v.apply(matrix_engine::verification::VerificationEvent::Devices {
                    account_id,
                    devices: vec![
                        d("QPLRTXWKAB", Some("Gnobbi (macOS)"), true, 3, "192.0.2.17"),
                        d("HJ7DKXQ2ZM", Some("Element iOS"), false, 60 * 26, "203.0.113.14"),
                        d("W4N8CCEYTU", Some("Firefox su Linux"), false, 60 * 24 * 9, "2001:db8:85a3::8a2e:370:7334"),
                        d("ZZ91ABCDEF", None, false, 60 * 24 * 40, "203.0.113.9"),
                        d("LAPTOP001", Some("Gnobbi Linux"), true, 30, "198.51.100.21"),
                        d("PHONE0001", Some("Android"), true, 90, "198.51.100.22"),
                        d("BROWSER01", Some("Safari"), true, 120, "198.51.100.23"),
                        d("TABLET001", Some("iPad"), true, 240, "198.51.100.24"),
                        d("DESKTOP01", Some("Windows"), true, 480, "198.51.100.25"),
                    ],
                });
            }
            if std::mem::take(&mut self.rail.refresh_devices) {
                v.refresh();
            }
            devices::show(ui.ctx(), v, &mut self.rail, &mut self.device_fields, t);
            devices::dialogs(ui.ctx(), v, &mut self.device_fields);
            devices::popup(ui.ctx(), v);
            devices::new_key(ui.ctx(), v);
            // Test hook: GNOBBI_EGUI_NO_OFFER keeps the recovery-key offer out of a screenshot.
            let suppressed = account.is_some_and(|a| self.prefs.no_recovery_offer_accounts.contains(&a.as_u64())) || std::env::var_os("GNOBBI_EGUI_NO_OFFER").is_some();
            match devices::offer(ui.ctx(), v, suppressed, &mut self.device_fields) {
                Some(devices::Offer::Create) => v.accept_offer(),
                Some(devices::Offer::Recover(key)) => { v.recover_keys(key); }
                Some(devices::Offer::NotNow) => v.decline_offer(),
                Some(devices::Offer::Never) => {
                    v.decline_offer();
                    if let (Some(d), Some(a)) = (&self.dirs, account)
                        && let Ok(p) = matrix_desktop::Preferences::suppress_recovery_offer(d, a.as_u64())
                    {
                        self.prefs.no_recovery_offer_accounts = p.no_recovery_offer_accounts;
                    }
                }
                None => {}
            }
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
            roomedit::show(ui.ctx(), s, rights, if is_space { i18n::t("Edit space") } else { i18n::t("Edit room") });
        }
        if let Some(dirs) = &self.dirs {
            let running: Vec<_> = self.parked.iter().map(|p| accounts::Running { dir: p.root.clone(), picture: p.session.own_avatar(), unread: p.session.sidebar.navigation_entries().first().map_or(0, |e| e.unread) }).collect();
            match accounts::show(ui.ctx(), &mut self.rail, dirs, &self.root, self.session.as_mut(), &running, &mut self.account_form, t) {
                Some(accounts::Launch::Open(dir)) => self.open_account(dir),
                Some(accounts::Launch::Add) => {
                    let next = matrix_desktop::AccountCatalog::load(dirs).next_root(dirs);
                    self.open_account(next);
                }
                Some(accounts::Launch::SignOut(dir)) => {
                    let catalog = matrix_desktop::AccountCatalog::load(dirs);
                    let entry = catalog.id_for_root(dirs, &dir).and_then(|id| catalog.get(id));
                    let id = entry.and_then(|e| e.user_id.as_deref().or(e.homeserver.as_deref())).unwrap_or_default();
                    let name = entry.map(|e| e.label.as_str()).filter(|l| !l.is_empty()).unwrap_or(id);
                    self.sign_out = Some(accounts::SignOut::new(dir, name, id));
                }
                None => {}
            }
        }
        self.sign_out_step(ui.ctx());
        let new = settings::show(ui.ctx(), &mut self.rail, &self.prefs, self.dirs.as_ref(), t)
            .or_else(|| customize::show(ui.ctx(), &mut self.rail, &self.prefs, self.system_scale, self.dirs.as_ref(), t));
        if let Some(new) = new {
            if new.icon != self.prefs.icon
                && let Some(icon) = app_icon(new.icon)
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon))));
            }
            if new.language != self.prefs.language {
                matrix_desktop::i18n::apply(new.language, Some(ui.ctx()));
            }
            self.prefs = new;
            self.applied = None;
        }
    }

    fn sign_in(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show(ui, |ui| {
            let room = ui.available_height();
            // Scrolls when the keyboard (it shrinks the content rect) or a large text size leaves too little room.
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let m = crate::metrics::get();
                let width = (ui.available_width() - 2. * m.gap * 3.).clamp(200., 400.);
                let height_id = egui::Id::new("signin_height");
                let last = ui.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(0.);
                ui.add_space(((room - last) / 2.).max(m.gap * 3.));
                let top = ui.cursor().top();
                ui.vertical_centered(|ui| {
                    ui.allocate_ui_with_layout(egui::vec2(width, 0.), dir::col(), |ui| match &mut self.session {
                        Some(session) => sign_in_form(ui, session, t),
                        None => {
                            ui.label(self.start_error.as_deref().unwrap_or(i18n::t("Start failed")));
                        }
                    });
                });
                ui.add_space(m.gap * 3.);
                let used = ui.cursor().top() - top;
                ui.data_mut(|d| d.insert_temp(height_id, used));
            });
        });
    }
}

/// Local conversations and server users use the same search models as the GPUI switcher.
fn quick_search(ctx: &egui::Context, s: &mut Session, t: &DesignTokens) {
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
    let joins: Vec<_> = public.iter().map(|h| s.joins.get(&h.room_id).map(|j| (j.is_running(), j.line(), j.error().is_some()))).collect();
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
    let modal = egui::Modal::new(egui::Id::unique("quick-search")).show(ctx, |ui| dir::scope(ui, |ui| {
        ui.set_min_width(360.);
        ui.heading(i18n::t("Search"));
        if ui.add(crate::metrics::line(&mut s.new_term).id(egui::Id::unique("quick-search-field")).hint_text(i18n::t("Search"))).changed() {
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
            if !hits.is_empty() { ui.label(i18n::t("Conversations")); }
            for (i, hit) in hits.iter().enumerate() { row(ui, i, hit.name.clone()); }
            drop(row);
            if !users.is_empty() { ui.label(i18n::t("Users")); }
            let duplicates = crate::rows::duplicate_names(users.iter().map(|hit| hit.display_name.as_deref().unwrap_or(&hit.user_id)));
            for (i, (hit, duplicate)) in users.iter().zip(duplicates).enumerate() {
                let name = hit.display_name.as_deref().unwrap_or(&hit.user_id);
                let index = hits.len() + i;
                let r = ui.push_id(&hit.user_id, |ui| crate::rows::person_row(ui, name, &hit.user_id, duplicate, index == sel, t)).inner;
                if index == sel && moved { r.scroll_to_me(None); }
                if r.clicked() { picked = Some(index); }
            }
            let mut row = |ui: &mut egui::Ui, i: usize, text: String| {
                let r = ui.selectable_label(i == sel, text);
                if i == sel && moved { r.scroll_to_me(None); }
                if r.clicked() { picked = Some(i); }
            };
            if !public.is_empty() { ui.label(i18n::t("Public rooms")); }
            for (i, hit) in public.iter().enumerate() {
                // The join's spinner and its one-line state sit on the room's own row.
                ui.horizontal(|ui| {
                    row(ui, hits.len() + users.len() + i, hit.name.clone());
                    let Some((running, line, failed)) = joins[i] else { return };
                    if running { ui.spinner(); }
                    if let Some(line) = line {
                        ui.label(egui::RichText::new(line).color(if failed { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() }));
                    }
                });
            }
        });
    }));
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
                // The search stays open on the row until the join's real outcome.
                None => { s.join_public(hit); return; }
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
        egui::Frame::popup(ui.style()).fill(rgb(t.surface)).show(ui, |ui| dir::popup(ui, |ui| {
            ui.set_min_width(160.);
            let Some(p) = perms else {
                ui.spinner();
                return;
            };
            if p.react {
                ui.collapsing(i18n::t("React"), |ui| {
                    egui::Grid::new("reaction-emoji-grid").show(ui, |ui| {
                        for (i, (name, emoji)) in matrix_desktop::EMOJI_SET.iter().enumerate() {
                            if ui.button(*emoji).on_hover_text(matrix_desktop::i18n::t(name)).clicked() {
                                action = Some(*emoji);
                            }
                            if i % 8 == 7 { ui.end_row(); }
                        }
                    });
                });
            }
            if p.reply && ui.button(i18n::t("Reply")).clicked() {
                action = Some("reply");
            }
            if p.reply && ui.button(i18n::t("Reply in thread")).clicked() {
                action = Some("thread");
            }
            if p.edit && ui.button(i18n::t("Edit")).clicked() {
                action = Some("edit");
            }
            let poll = s.ops.selection().and_then(|id| s.messages.as_ref()?.events().iter().find(|e| e.id == *id)?.poll.clone());
            if s.ops.can_end_poll(poll.as_ref()) {
                let label = if s.confirm_end_poll { i18n::t("Confirm closing") } else { i18n::t("Close poll") };
                if ui.button(label).clicked() {
                    if s.confirm_end_poll { action = Some("end"); } else { s.confirm_end_poll = true; }
                }
                if s.confirm_end_poll && ui.button(i18n::t("Cancel")).clicked() { s.confirm_end_poll = false; }
            }
            if p.redact {
                let label = if s.confirm_delete { i18n::t("Confirm deletion") } else { i18n::t("Delete") };
                if ui.button(RichText::new(label).color(rgb(t.error))).clicked() {
                    if s.confirm_delete { action = Some("delete") } else { s.confirm_delete = true }
                }
            }
            if !(p.react || p.reply || p.edit || p.redact) {
                ui.label("—");
            }
        }));
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
    let m = crate::metrics::get();
    let (width, height) = (ui.available_width(), crate::metrics::button());
    let radius = CornerRadius::same(m.radius.max(12.) as u8);
    {
        // The logo: the accent square with the app's chat glyph, then the name.
        let side = (height * 1.5).max(64.);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same((side / 4.) as u8), rgb(t.accent));
            icons::paint(ui.painter(), rect.center(), icons::Icon::ChatCircleDots, true, side * 0.58, rgb(t.on_accent));
            ui.add_space(m.gap * 1.5);
            ui.label(RichText::new("Gnobbi").size(m.title * 1.5).strong().color(rgb(t.text)));
        });
        ui.add_space(m.gap * 3.);
    }
    // Fields: fill from the theme, no outline, accent ring while focused.
    let v = ui.visuals_mut();
    v.extreme_bg_color = rgb(t.input);
    v.selection.stroke = Stroke::new(2., rgb(t.accent));
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = radius;
    }
    let mut enter = false;
    let mut field = |ui: &mut egui::Ui, text: &mut String, label: &str, secret: bool, raw: bool| {
        ui.label(RichText::new(label).size(m.secondary).color(rgb(t.secondary)));
        ui.add_space(m.gap / 2.);
        let eye_id = egui::Id::new(("signin-eye", label));
        let shown = secret && ui.data(|d| d.get_temp::<bool>(eye_id)).unwrap_or(false);
        let (near, far) = (14, if secret { height as i8 } else { 14 });
        let margin = if dir::rtl() { egui::Margin { left: far, right: near, top: 0, bottom: 0 } } else { egui::Margin { left: near, right: far, top: 0, bottom: 0 } };
        let r = ui.add_sized([width, height], crate::metrics::line(text).password(secret && !shown).raw_input(raw).margin(margin));
        if secret {
            let rect = dir::flip(r.rect, egui::Rect::from_center_size(egui::pos2(r.rect.right() - height / 2., r.rect.center().y), egui::vec2(height, height)));
            let (icon, name) = if shown { (icons::Icon::EyeSlash, i18n::t("Hide password")) } else { (icons::Icon::Eye, i18n::t("Show password")) };
            if icons::button_at(ui, rect, eye_id.with("button"), icon, false, m.icon, name, None, true, 1.).clicked() {
                ui.data_mut(|d| d.insert_temp(eye_id, !shown));
            }
        }
        // Long press (touch) or right click: the only way to paste on a phone without a keyboard shortcut.
        r.context_menu(|ui| {
            if ui.button(i18n::t_ctx("menu", "Paste")).clicked() {
                ui.memory_mut(|mem| mem.request_focus(r.id));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
        });
        if r.gained_focus() {
            r.scroll_to_me(None);
        }
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(m.gap * 1.5);
    };
    if let Some(p) = prompt {
        match (p.mode, s.form.vault) {
            (SignInMode::Password, vault) => {
                field(ui, &mut s.homeserver, i18n::t("Server"), false, true);
                field(ui, &mut s.user, i18n::t("Matrix ID"), false, true);
                field(ui, &mut s.password, i18n::t("Password"), true, false);
                if !vault {
                    field(ui, &mut s.phrase, i18n::t("Local passphrase"), true, false);
                }
            }
            (SignInMode::Unlock, _) => field(ui, &mut s.phrase, i18n::t("Local passphrase"), true, false),
            (SignInMode::UnlockVault, _) => {}
        }
    }
    // The engine gives up on its own only after discovery succeeds: after 20 s say what the form already says for an unreachable server.
    let stalled = waited.is_some_and(|w| w > std::time::Duration::from_secs(20));
    let error = if stalled { FormStatus::Failed(matrix_engine::signin::SignInError::Discovery).error_message() } else { s.form.status().error_message() };
    if let Some(message) = error {
        ui.label(RichText::new(message).color(rgb(t.error)));
        ui.add_space(m.gap * 1.5);
    }
    let busy = prompt.is_none();
    let label = match (unlock, busy) {
        (false, false) => i18n::t("Sign in"),
        (true, false) => i18n::t("Unlock"),
        (false, true) => i18n::t("Signing in…"),
        (true, true) => i18n::t("Unlocking…"),
    };
    let fill = if busy { rgb(t.accent).gamma_multiply(0.6) } else { rgb(t.accent) };
    let button = egui::Button::new(RichText::new(label).strong().color(rgb(t.on_accent))).fill(fill).corner_radius(radius);
    let r = ui.add_sized([width, height + 4.], button);
    if busy {
        // Loading: a spinner at the left edge of the (inert) button.
        let rect = dir::flip(r.rect, egui::Rect::from_center_size(egui::pos2(r.rect.left() + height / 2. + 4., r.rect.center().y), egui::Vec2::splat(m.icon)));
        egui::Spinner::new().color(rgb(t.on_accent)).paint_at(ui, rect);
    } else if r.clicked() || enter {
        s.submit();
    }
}

/// «Cronologia cifrata»: the recovery key restores old encrypted messages; it can be skipped.
fn recovery_form(ui: &mut egui::Ui, s: &mut Session, t: &DesignTokens) {
    ui.heading(i18n::t("Encrypted history"));
    ui.add_space(8.);
    let accent = |text: &str| egui::Button::new(egui::RichText::new(text).color(list::rgb(t.on_accent))).fill(list::rgb(t.accent));
    if let Some(key) = s.form.recovery_key() {
        ui.label(i18n::t("Matrix recovery key"));
        let kept_id = egui::Id::new("sign-in-key-kept");
        let mut kept = ui.data(|d| d.get_temp::<bool>(kept_id)).unwrap_or(false);
        devices::key_view(ui, key, &mut kept);
        ui.checkbox(&mut kept, i18n::t("I kept it elsewhere"));
        ui.data_mut(|d| d.insert_temp(kept_id, kept));
        ui.add_space(8.);
        if ui.add_enabled(kept, egui::Button::new(egui::RichText::new(i18n::t("I saved the key: continue")).color(list::rgb(t.on_accent))).fill(list::rgb(t.accent)).min_size(egui::vec2(ui.available_width(), crate::metrics::button()))).clicked() {
            ui.data_mut(|d| d.remove::<bool>(kept_id));
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
        let r = ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut s.phrase).hint_text(i18n::t("Matrix recovery key")).password(true));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.);
        if ui.add_sized([ui.available_width(), crate::metrics::button()], accent(i18n::t("Recover encrypted history"))).clicked() || enter {
            s.recover();
        }
    } else if s.form.setup_offered() {
        let label = if s.form.setup_phase().is_some() { i18n::t("Try creating the key again") } else { i18n::t("Create recovery key") };
        if ui.add_sized([ui.available_width(), crate::metrics::button()], accent(label)).clicked() {
            s.setup_recovery();
        }
    }
    if (s.form.setup_phase().is_some() || matches!(s.form.recovery(), Some(matrix_engine::signin::RecoveryState::Failed(_))))
        && let Some(message) = s.form.error_message()
    {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
    ui.add_space(6.);
    if ui.add_sized([ui.available_width(), crate::metrics::button()], accent(i18n::t("Continue without recovery"))).clicked() {
        s.defer_recovery();
    }
}

/// First instance listens for later launches; if one is already running
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
#[cfg(not(target_os = "ios"))]
use matrix_desktop::tray::Tray as TrayHandle;
/// iOS stub: the tray module needs tray-icon, which does not exist there.
#[cfg(target_os = "ios")]
struct TrayHandle;
#[cfg(target_os = "ios")]
impl TrayHandle { fn start() -> Option<Self> { None } }

fn register_emoji_font(ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    {
        static NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../../../assets/fonts/NotoColorEmoji.ttf");
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

/// What a macOS Edit menu entry sends to egui: copy/cut/paste take the fork's clipboard path, the rest is the key egui itself binds.
enum EditInput {
    Request(egui::ViewportCommand),
    Key(egui::Key, egui::Modifiers),
}

fn edit_input(edit: matrix_desktop::tray_menu::Edit) -> EditInput {
    use matrix_desktop::tray_menu::Edit;
    let command = egui::Modifiers::COMMAND;
    match edit {
        Edit::Cut => EditInput::Request(egui::ViewportCommand::RequestCut),
        Edit::Copy => EditInput::Request(egui::ViewportCommand::RequestCopy),
        Edit::Paste => EditInput::Request(egui::ViewportCommand::RequestPaste),
        Edit::Undo => EditInput::Key(egui::Key::Z, command),
        Edit::Redo => EditInput::Key(egui::Key::Z, command | egui::Modifiers::SHIFT),
        Edit::SelectAll => EditInput::Key(egui::Key::A, command),
    }
}

fn take_once(done: &mut bool) -> bool {
    !std::mem::replace(done, true)
}

fn main() -> eframe::Result {
    // iOS: stderr goes nowhere, so `log` goes to os_log (Console / `log stream`) and stderr is piped into it line by line.
    #[cfg(target_os = "ios")]
    {
        use std::io::BufRead;
        use std::os::fd::FromRawFd;
        let _ = oslog::OsLogger::new("app.gnobbi").level_filter(log::LevelFilter::Info).init();
        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
            unsafe { libc::dup2(fds[1], 2); libc::dup2(fds[1], 1); }
            let rd = unsafe { std::fs::File::from_raw_fd(fds[0]) };
            std::thread::spawn(move || std::io::BufReader::new(rd).lines().map_while(Result::ok).for_each(|l| log::info!("{l}")));
        }
        if std::env::var_os("RUST_LOG").is_some() {
            let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).try_init();
        }
    }
    matrix_desktop::notifications::init();
    // A `matrix:` link is not an option: taken out first, validated by the engine, carried out once the rooms are there.
    let (args, raw_link) = matrix_desktop::take_matrix_link(std::env::args().skip(1).collect());
    let link = raw_link.clone().and_then(|raw| matrix_engine::matrix_link::parse_matrix_link(&raw).map_err(|_| eprintln!("Gnobbi: ignored matrix link")).ok());
    #[allow(unused_mut)]
    let mut dirs = matrix_desktop::app_dirs(Platform::CURRENT, |k| std::env::var(k).ok());
    // iOS: the stores move to the App Group container; if that fails, keep working from where they were.
    #[cfg(all(target_os = "ios", not(target_abi = "sim")))]
    if let Some(d) = &mut dirs
        && d.data != d.config
        && matrix_desktop::app_group::migrate(&d.config, &d.data).is_err()
    {
        eprintln!("Gnobbi: store migration failed");
        d.data = d.config.clone();
    }
    let default_root = dirs.as_ref().map(|d| d.data.join("profile"));
    // `--signin <folder>` runs another account's profile; the default profile is the first install's.
    let mut trust = None;
    let root = match matrix_desktop::SignInOptions::parse(&args) {
        Ok(Some(o)) => {
            // An explicit CA (a test homeserver); read once, at most 1 MiB.
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
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([first.width, first.height])
            .with_min_inner_size([MIN_WIDTH, MIN_HEIGHT])
            .with_title(TITLE)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_app_id(APP_ID)
            .with_icon(app_icon(dirs.as_ref().map(Preferences::load).unwrap_or_default().icon).unwrap_or_default()),
        ..Default::default()
    };
    // iOS: the Metal adapter (the simulator's at least) caps max_inter_stage_shader_variables at 15, below wgpu's default 16 that egui-wgpu requests.
    #[cfg(target_os = "ios")]
    {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.device_descriptor = std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor { label: Some("gnobbi"), required_limits: adapter.limits(), ..Default::default() });
        options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    let result = eframe::run_native(TITLE, options, Box::new(move |cc| {
        register_emoji_font(&cc.egui_ctx);
        icons::register(&cc.egui_ctx);
        #[cfg(target_os = "ios")]
        {
            mobile::observe_keyboard(&cc.egui_ctx);
            mobile::observe_lifecycle(&cc.egui_ctx);
        }
        #[cfg(target_os = "ios")]
        pick::purge_temp();
        Ok(Box::new(App::new(dirs.as_ref(), root, trust, link, multi)))
    }));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_entries_map_to_egui_input() {
        use matrix_desktop::tray_menu::Edit;
        assert!(matches!(edit_input(Edit::Copy), EditInput::Request(egui::ViewportCommand::RequestCopy)));
        assert!(matches!(edit_input(Edit::Paste), EditInput::Request(egui::ViewportCommand::RequestPaste)));
        assert!(matches!(edit_input(Edit::Redo), EditInput::Key(egui::Key::Z, m) if m.shift && m.command));
        assert!(matches!(edit_input(Edit::SelectAll), EditInput::Key(egui::Key::A, m) if m.command && !m.shift));
    }

    #[test]
    fn login_hook_fires_once_over_many_frames() {
        let mut sent = false;
        assert_eq!((0..50).filter(|_| take_once(&mut sent)).count(), 1);
    }

    #[test]
    fn login_hook_fires_again_only_after_the_second_account_resets_it() {
        let mut sent = false;
        assert!(take_once(&mut sent));
        assert!(!take_once(&mut sent), "an error leaves the form on its prompt: no retry");
        sent = false; // what GNOBBI_EGUI_ADD does when it opens the second account
        assert!(take_once(&mut sent));
        assert!(!take_once(&mut sent));
    }

    #[test]
    fn one_column_shows_one_side_panel_at_a_time() {
        assert_eq!(side_on_top(true, true, false, true), Some(Side::Member));
        assert_eq!(side_on_top(true, false, false, true), Some(Side::Info));
        assert_eq!(side_on_top(false, true, true, true), Some(Side::Search));
        assert_eq!(side_on_top(false, true, false, true), Some(Side::Thread));
        assert_eq!(side_on_top(false, true, false, false), None);
    }

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
            let (expected_a, expected_b) = match theme {
                Theme::Classic => (0xe9f0eb, 0xfcfcfc),
                Theme::Day => (0xf4f6f5, 0xfcfcfc),
                Theme::Dark => (0x101b14, 0x142018),
                Theme::Night => (0x050a07, 0x0b130e),
            };
            let t = theme.tokens_with(matrix_desktop::Accent::Blue);
            let timeline = chat_background(&t);
            let rail = egui::Frame::NONE.fill(chat_background(&t)).fill;
            let list = list_fill(&t);
            assert_eq!(timeline, rgb(expected_a), "{theme:?}");
            assert_eq!(list, rgb(expected_b), "{theme:?}");
            assert_eq!(rail, timeline, "{theme:?}");
            assert_eq!(bar(&t).fill, list, "{theme:?}");
            assert_ne!(timeline, list, "{theme:?}");
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
