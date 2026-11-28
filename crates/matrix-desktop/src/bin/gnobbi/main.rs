#![recursion_limit = "256"]
//! Gnobbi, egui frontend. S1: shell with the saved theme and accent, window identity and the
//! single-instance gate (same profile, never two processes on it).
use list::rgb;
use matrix_desktop::i18n;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, ViewportBuilder, Visuals};
use matrix_desktop::{
    AppDirs, DesignTokens, FormStatus, Platform, Preferences, Theme,
    app_id::{APP_ID, TITLE},
};
use matrix_engine::RoomSyncStatus;
use std::time::Duration;

mod accounts;
mod animation;
mod attach;
mod avatar;
mod avatar_editor;
mod pick;
mod composer;
mod mention_popup;
mod customize;
mod devices;
mod dir;
mod explore;
#[cfg(target_os = "linux")]
mod desktop_entry;
mod highlight;
mod icons;
mod info;
mod join;
mod layout;
mod list;
mod loading;
mod members;
mod media_cards;
#[cfg(test)]
mod members_bench;
mod message_hits;
mod metrics;
mod mobile;
mod newchat;
mod poll;
mod rail;
mod reactions;
mod rows;
mod roomedit;
mod space;
mod stickers;
mod secret_clip;
mod session;
mod shared;
mod suspend;
mod settings;
mod status_pill;
mod timeline;
mod touch_list;
mod touch_menu;
mod viewer;
use session::Session;

/// Avatar fallback colour (identity, not theme).
pub const AVATAR: u32 = matrix_desktop::AVATAR_FALLBACK;

/// File in the data folder with the profile folder of the account on screen.
const LAST_ACCOUNT: &str = "last_account";

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
    v.menu_corner_radius = CornerRadius::same(10);
    v
}

/// An account that is not on screen: its profile folder and its live session.
struct Parked<S = Session> {
    root: std::path::PathBuf,
    session: S,
    recorded: bool,
}

/// Narrow layout shows rail 76 + chat; the chat needs ≥360.
const MIN_WIDTH: f32 = 436.;
const MIN_HEIGHT: f32 = 420.;

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
    /// The slide between the chat and the list in one column.
    nav: mobile::Nav,
    rail: rail::State,
    /// Catalog id of the shown account (the rail layout is stored under it), with the profile folder it was read for.
    rail_account: Option<(std::path::PathBuf, u64)>,
    /// Profile folder this window runs on, and whether its login is already in the catalog.
    root: std::path::PathBuf,
    recorded: bool,
    /// The other accounts of the catalog, running at the same time as the shown one (own worker, own notifications).
    parked: Vec<Parked>,
    /// The merged chat list of «Non lette» / «Tutte le chat» over every account, and the room a click on another account's row chose.
    merged: matrix_desktop::UnifiedList,
    merged_open: Option<(usize, matrix_domain::RoomScope)>,
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
    applied: Option<(Theme, u32, matrix_desktop::TextSize, matrix_desktop::Density, u32, u32)>,
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
    hook_hit: bool,
    alias_input: String,
    hook_keys: bool,
    hook_menu_at: Option<std::time::Instant>,
    /// Test hook «viewer»: when the viewer was opened, and whether the step was made.
    hook_viewer: Option<(std::time::Instant, bool)>,
    /// Why the backend could not start, shown instead of the form.
    start_error: Option<String>,
    startup: Option<std::collections::VecDeque<std::path::PathBuf>>,
    startup_notice: Option<String>,
    account_notice: Option<String>,
    reopening: Option<(Option<matrix_engine::WorkerControl>, bool)>,
    remembered_root: Option<std::path::PathBuf>,
    signing_in_again: bool,
    recovery_switch: bool,
    /// Add-account mode: the account that was on screen when the sign-in form opened (it keeps running, parked).
    add_from: Option<std::path::PathBuf>,
    /// Cancelled new profiles: removed once their worker has ended and let go of the store.
    discarding: Vec<(matrix_engine::WorkerControl, std::path::PathBuf)>,
}

impl App {
    fn new(dirs: Option<&AppDirs>, root: std::path::PathBuf, trust: Option<Vec<u8>>, link: Option<matrix_domain::MatrixLink>, multi: bool) -> App {
        let prefs = dirs.map(Preferences::load).unwrap_or_else(Preferences::default);
        matrix_desktop::i18n::apply(prefs.language, None);
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if let Some(dirs) = dirs {
            matrix_desktop::push::init(dirs);
        }
        #[cfg(target_os = "linux")]
        if dirs.is_some() {
            desktop_entry::install_at_start(prefs.icon, prefs.desktop_entry_dont_ask);
        }
        #[cfg(target_os = "macos")]
        mac_dock_icon(prefs.icon);
        let theme = Theme::Classic;
        let candidates = if multi {
            dirs.map(|d| {
                let catalog = matrix_desktop::AccountCatalog::load(d);
                let roots = catalog.accounts().iter().filter_map(|e| catalog.data_dir(d, e.id)).collect();
                let last = std::fs::read_to_string(d.data.join(LAST_ACCOUNT)).ok();
                startup_roots(root.clone(), roots, last.as_deref().map(str::trim))
            }).unwrap_or_else(|| vec![root.clone()])
        } else { vec![root.clone()] };
        let root = candidates[0].clone();
        let startup = multi.then(|| candidates.into_iter().skip(1).collect());
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
        App { prefs, dirs: dirs.cloned(), list_open: false, nav: mobile::Nav::from_env(), rail: rail::State { layouts: dirs.map(matrix_desktop::RailLayouts::load).unwrap_or_default(), ..Default::default() }, rail_account: None, root: root.clone(), recorded: false, parked, merged: Default::default(), merged_open: None, trust: trust.clone(), wake: None, tray: TrayHandle::start(), #[cfg(target_os = "macos")] menubar: None, insets: mobile::Insets::from_env(), quitting: false, window_visible: true, new_form: Default::default(), device_fields: Default::default(), account_form: Default::default(), poll_form: Default::default(), confirm: None, sign_out: None, theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, system_scale: metrics::system_scale(), was_focused: true, shot: matrix_desktop::hooks::var_os("GNOBBI_EGUI_SHOT").map(|p| (p.into(), std::time::Instant::now())),
            pending_link: link.map(|l| (l, std::time::Instant::now())), pending_web: None, opening_web: None, failed_web: None, shot_sent: false, hook_sent: false, hook_added: false, auto_login2: matrix_desktop::hooks::var_os("GNOBBI_EGUI_LOGIN2").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), hook_type: false, hook_alias: false, hook_member: false, hook_hit: false, alias_input: String::new(), hook_keys: false, hook_menu_at: None, hook_viewer: None,
            auto_login: matrix_desktop::hooks::var_os("GNOBBI_EGUI_LOGIN").and_then(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok()), session, start_error, login_sent: false, startup, startup_notice: None, account_notice: None, reopening: None, remembered_root: None, signing_in_again: false, recovery_switch: false, add_from: None, discarding: Vec::new() }
    }

    /// Brings parked account `i` on screen; the shown one takes its place in the list.
    fn switch_to(&mut self, i: usize) {
        if i >= self.parked.len() { return; }
        let target = self.dirs.as_ref().and_then(|d| matrix_desktop::AccountCatalog::load(d).id_for_root(d, &self.parked[i].root)).map(|id| id.as_u64());
        eprintln!("Gnobbi: switch request catalog={target:?}");
        self.wake = None;
        self.add_from = None;
        let p = self.parked.remove(i);
        if let Some(session) = self.session.take() {
            self.parked.push(Parked { root: self.root.clone(), session, recorded: self.recorded });
        }
        (self.session, self.root, self.recorded) = (Some(p.session), p.root, p.recorded);
        self.start_error = None;
        self.account_notice = None;
        self.signing_in_again = false;
        self.recovery_switch = false;
        self.remembered_root = None;
        self.reset_account_ui();
        self.remember_current();
    }

    /// A row of another account in the merged list: that account comes forward with the same view and search, and the room opens in it.
    fn open_merged_room(&mut self, parked: usize, scope: matrix_domain::RoomScope) {
        let (unread_view, query) = self.session.as_ref().map(|s| (s.sidebar.unread_view(), s.query.clone())).unwrap_or_default();
        self.switch_to(parked);
        let Some(s) = &mut self.session else { return };
        if s.sidebar.active_space().is_some() {
            s.sidebar.set_space(None);
        }
        s.sidebar.set_unread_view(unread_view);
        if s.query != query {
            s.sidebar.set_query(&query);
            s.query = query;
        }
        if !s.sidebar.select(&scope) {
            s.sidebar.select_outside_view(&scope);
        }
    }

    fn reset_account_ui(&mut self) {
        (self.new_form, self.device_fields, self.account_form, self.poll_form, self.confirm) = Default::default();
        self.rail.settings = false;
        self.rail.accounts = false;
        self.rail.devices = false;
        self.rail.customize = false;
        self.rail_account = None;
    }

    fn catalog_entry(&self) -> Option<matrix_desktop::AccountEntry> {
        let d = self.dirs.as_ref()?;
        let c = matrix_desktop::AccountCatalog::load(d);
        c.accounts().iter().find(|e| c.data_dir(d, e.id).as_deref() == Some(&self.root)).cloned()
    }

    fn open_failed(&self) -> bool {
        self.reopening.is_none() && (self.start_error.is_some() || (self.catalog_entry().is_some()
            && self.session.as_ref().is_some_and(|s| s.account.is_none()
                && if self.signing_in_again { matches!(s.status(), FormStatus::Failed(_)) } else { restore_failed(s.status()) })))
    }

    fn remember_current(&mut self) {
        let opened = self.session.as_ref().is_some_and(|s| s.account.is_some());
        if opened && self.remembered_root.as_ref() != Some(&self.root)
            && let Some(d) = &self.dirs
            && remember_open_account(&d.data.join(LAST_ACCOUNT), &self.root, opened).is_ok()
        {
            let id = self.catalog_entry().map(|e| e.id.as_u64());
            eprintln!("Gnobbi: switch result catalog={id:?} result=Ok");
            self.remembered_root = Some(self.root.clone());
        }
    }

    /// Wait for the actual authentication outcome, not just the worker's creation.
    fn selection_step(&mut self) {
        let opened = self.session.as_ref().is_some_and(|s| s.account.is_some());
        let failed = !opened && self.open_failed();
        match startup_step(&mut self.startup, opened, failed) {
            StartupAction::Ready => {
                self.start_error = None;
                if let Some(line) = self.startup_notice.take() {
                    let id = self.catalog_entry().map(|e| e.id.as_u64());
                    eprintln!("Gnobbi: fallback chosen catalog={id:?} result=Ok");
                    self.account_notice = Some(line);
                }
                self.remember_current();
            }
            StartupAction::Open(next) => {
                if self.startup_notice.is_none() {
                    let identity = self.catalog_entry().map(|e| e.user_id.unwrap_or(e.label)).unwrap_or_default();
                    self.startup_notice = Some(i18n::t("Could not open {account}. Switched to another account.").replace("{account}", &identity));
                }
                self.open_account(next);
            }
            StartupAction::Exhausted => self.startup_notice = None,
            StartupAction::Wait => {}
        }
    }

    /// Starts every account again from its saved profile (after the background, which ended the workers).
    fn restart_accounts(&mut self) {
        let Some(d) = self.dirs.clone() else { return };
        if let Some(session) = &mut self.session {
            if let Err(error) = session.reconnect(&d, self.root.clone(), self.trust.clone()) {
                eprintln!("Gnobbi: reconnect failed: {error}");
            }
        }
        for p in &mut self.parked {
            if let Err(error) = p.session.reconnect(&d, p.root.clone(), self.trust.clone()) {
                eprintln!("Gnobbi: reconnect failed: {error}");
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
                    so.local_only();
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
                        self.start_error = None;
                        self.signing_in_again = false;
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
        self.add_from = None;
        if dir == self.root {
            self.start_error = None;
            self.startup = None;
            self.signing_in_again = false;
            self.remembered_root = None;
            self.account_notice = None;
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
                self.remembered_root = None;
                self.remember_current();
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
        if crate::suspend::is_suspended() {
            eprintln!("Gnobbi: account switch request result=suspended");
            return;
        }
        let Some(d) = &self.dirs else {
            eprintln!("Gnobbi: account switch request result=no-data-directory");
            return;
        };
        match Session::start(d, dir.clone(), self.trust.clone()) {
            Ok(session) => {
                self.parked.push(Parked { root: dir, session, recorded: false });
                let last = self.parked.len() - 1;
                self.switch_to(last);
            }
            Err(error) => {
                if let Some(session) = self.session.take() {
                    self.parked.push(Parked { root: self.root.clone(), session, recorded: self.recorded });
                }
                self.root = dir;
                self.recorded = true;
                self.start_error = Some(error);
                self.signing_in_again = false;
                self.remembered_root = None;
                self.reset_account_ui();
            }
        }
    }

    /// Leaves the add-account sign-in form: the previous account comes back on screen (it never stopped), the half-created
    /// profile ends its worker and is wiped by `logic` once that has let go of the store. Nothing of the previous account is written.
    fn cancel_add(&mut self) {
        let Some(previous) = self.add_from.take() else { return };
        let Some((new, new_root)) = cancel_add(&mut self.session, &mut self.root, &mut self.recorded, &mut self.parked, &previous) else { return };
        let control = new.control();
        control.stop();
        self.discarding.push((control, new_root));
        self.start_error = None;
        self.account_notice = None;
        self.signing_in_again = false;
        self.remembered_root = Some(self.root.clone()); // LAST_ACCOUNT still names it: no write
        self.reset_account_ui();
    }

    /// Theme and accent for now (OS appearance, saved choice, automatic night); re-read every minute.
    fn refresh(&mut self, ctx: &egui::Context) {
        let system_dark = ctx.system_theme().is_some_and(|t| t == egui::Theme::Dark);
        let focused = ctx.input(|i| i.focused);
        if cfg!(target_os = "ios") || (focused && !self.was_focused) {
            self.system_scale = metrics::system_scale();
        }
        self.was_focused = focused;
        self.theme = self.prefs.effective_theme(system_dark, matrix_desktop::local_hour());
        self.tokens = self.theme.tokens_with(self.prefs.effective_accent());
        let key = (self.theme, self.tokens.accent, self.prefs.text_size, self.prefs.density, self.system_scale.to_bits(), (ctx.content_rect().width() * ctx.zoom_factor()).round() as u32); // the width: a rotation changes the zoom the screen allows
        if self.applied != Some(key) {
            ctx.set_visuals(visuals(&self.tokens, self.theme.is_dark()));
            #[cfg(target_os = "android")]
            mobile::set_bar_icons(!self.theme.is_dark());
            metrics::apply(ctx, &self.prefs, self.system_scale);
            if mobile::touch() { touch_list::apply_touch(ctx); }
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
            // The window focus does not follow the app lifecycle on iOS: do not wait for it to rewrite the badge.
            matrix_desktop::notifications::reset_badge();
            self.restart_accounts();
        }
        if suspend::is_suspended() {
            matrix_desktop::notifications::set_visible(None);
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        if let Some(d) = &self.dirs {
            self.discarding.retain(|(control, dir)| {
                let done = control.is_done();
                if done { discard_new_profile(d, dir); }
                !done
            });
        }
        if self.reopening.as_ref().is_some_and(|(control, _)| control.as_ref().is_none_or(|c| c.is_done())) {
            let (_, again) = self.reopening.take().unwrap();
            self.reopen_account(again);
        }
        let Some(session) = &mut self.session else {
            self.selection_step();
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
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
        #[cfg(any(target_os = "ios", target_os = "android"))]
        if matrix_desktop::push::take_granted() && self.dirs.as_ref().is_some_and(matrix_desktop::push::grant_default) {
            self.prefs.notifications = true;
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        matrix_desktop::push::set_enabled(self.prefs.notifications);
        session.poll(ctx, self.prefs.notifications, self.prefs.message_preview);
        matrix_desktop::notifications::set_visible(session.reading.0.then(|| session.sidebar.selected().map(|r| (session.catalog_id, r.scope.clone()))).flatten());
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
        self.selection_step();
        // The backend answers off-thread and cannot wake the UI: poll it ten times a second.
        // shortcut: a fixed tick, replace by a repaint signal from the workers when idle power matters.
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // A keyboard left open across a rotation stays in the old orientation: close it (and forget its height).
        if self.insets.apply(raw, ctx.pixels_per_point()) && mobile::touch() {
            ctx.memory_mut(|m| if let Some(id) = m.focused() { m.surrender_focus(id) });
            mobile::set_keyboard(0.);
        }
    }

    fn on_exit(&mut self) {
        if let Some(s) = &mut self.session {
            s.remember_scroll();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // First frame drawn: a freshly installed build is confirmed (else the next starts would roll it back).
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            static CONFIRM: std::sync::Once = std::sync::Once::new();
            CONFIRM.call_once(|| { std::thread::spawn(matrix_desktop::update::startup::confirm_update); });
        }
        let ctx = ui.ctx().clone();
        self.refresh(&ctx);
        ctx.request_repaint_after(Duration::from_secs(60));
        if let Some((path, frame)) = &mut self.shot {
            let secs: u64 = matrix_desktop::hooks::var("GNOBBI_EGUI_SHOT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
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
        // Test hook: GNOBBI_EGUI_EMOJI_PROBE=1 draws thumbs-up, ZWJ family, flag, keycap and toned hand (label + text field) over everything, for GNOBBI_EGUI_SHOT.
        if matrix_desktop::hooks::var_os("GNOBBI_EGUI_EMOJI_PROBE").is_some() {
            let mut samples = ["\u{1F44D}", "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", "\u{1F1EE}\u{1F1F9}", "1\u{FE0F}\u{20E3}", "\u{1F44B}\u{1F3FD}"].map(String::from);
            egui::Area::new(egui::Id::new("emoji-probe")).fixed_pos(egui::pos2(0., 80.)).order(egui::Order::Debug).show(&ctx, |ui| {
                egui::Frame::new().fill(egui::Color32::WHITE).show(ui, |ui| {
                    for s in &mut samples {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(s.as_str()).size(48.).color(egui::Color32::BLACK));
                            ui.add(egui::TextEdit::singleline(s).font(egui::FontId::proportional(48.)).desired_width(120.));
                        });
                    }
                });
            });
        }
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        // Files over the window: the chat says it takes them (Telegram does the same).
        if self.session.as_ref().is_some_and(|s| s.messages.is_some()) && ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint"));
            let painter = ctx.layer_painter(layer);
            painter.rect_filled(screen, 0., egui::Color32::from_black_alpha(140));
        }
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
                    _ => session.attach_all(&dropped, false),
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
                    else if session.members_page { session.members_page = false; }
                    else if session.info_open {
                        if let Some(scope) = session.shared.as_ref().map(|s| s.scope.clone()) { session.close_shared(scope); } else { session.info_open = false; }
                    }
                    else if session.search_open { session.search_open = false; }
                }
            }
            if self.rail.settings || self.rail.accounts || self.rail.customize || self.pending_web.is_some() || session.new_open || session.explore.is_some() || session.quick_open || session.poll_open || session.edit.is_some() || self.rail.devices || session.viewer.is_some() {
                if let Some(thread) = &mut session.thread { thread.tail_visible = false; }
            }
            // Narrow layout with the chat list on top: the selected room is not on screen.
            let room_hidden = self.list_open && layout::compact(&ctx);
            session.reading = (ctx.input(|i| i.focused) && !room_hidden, session.heights.at_bottom, self.prefs.read_receipts);
            if !self.recorded
                && let (Some(v), Some(dirs)) = (&session.verification, &self.dirs)
                && let Some((id, name)) = v.profile()
            {
                let _ = matrix_desktop::AccountCatalog::update(dirs, |c| {
                    c.record_login(dirs, &self.root, id, name.as_deref().unwrap_or(id))
                });
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
        self.open_pending_link();
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
        if let Some(line) = &self.account_notice {
            egui::Panel::top("account-selection-notice").frame(bar(&t)).show(ui, |ui| {
                ui.add(egui::Label::new(line).wrap());
            });
        }
        if self.open_failed() {
            self.account_recovery(ui, &t);
        } else if self.session.as_ref().is_some_and(|s| s.account.is_some() && !s.recovering()) {
            self.add_from = None;
            self.shell(ui, &t);
            if self.nav.wants_repaint(self.list_open) {
                ctx.request_repaint();
            }
        } else {
            let back = signin_back(self.signing_in_again, self.add_from.as_ref().is_some_and(|p| self.parked.iter().any(|q| &q.root == p)), self.session.as_ref().is_some_and(|s| s.account.is_some()));
            let mut go = false;
            if back.is_some() {
                egui::Panel::top("account-sign-in-back").frame(bar(&t)).show(ui, |ui| {
                    go = icons::button(ui, icons::Icon::CaretLeft, false, i18n::t("Back")).clicked();
                });
                // Esc on desktop, the left-edge swipe on a phone.
                go |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    || (mobile::touch() && mobile::back_drag(&ctx) == Some(mobile::BackDrag::Released(true)));
            }
            match back {
                Some(SigninBack::Recovery) if go => self.signing_in_again = false,
                Some(SigninBack::Previous) if go => self.cancel_add(),
                _ => {}
            }
            if !go { self.sign_in(ui, &t); }
        }
        if self.open_failed() { self.sign_out_step(&ctx); }
        if let Some(s) = &mut self.session { s.show_viewer(&ctx); }
        if let Some(s) = &mut self.session {
            #[cfg(debug_assertions)]
            stickers::fixture(&ctx, &mut s.stickers, &mut s.avatars);
            stickers::windows(&ctx, &mut s.stickers, &s.avatars, &t);
        }
        #[cfg(debug_assertions)]
        stickers::demo(&ctx, &t);
        #[cfg(debug_assertions)]
        mention_popup::demo(&ctx, &t);
        #[cfg(debug_assertions)]
        if matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("avatar-editor") { self.account_form.demo(&ctx, &t); }
        #[cfg(debug_assertions)]
        if let Ok(view) = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW") && view.starts_with("loading-") { loading::demo(&ctx, &view, chat_background(&t), list_fill(&t), &t); }
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
    /// Opens the `matrix:`/matrix.to link given at launch once the first account has its rooms (product code, not a test hook).
    fn open_pending_link(&mut self) {
        let Some(s) = &mut self.session else { return };
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
    }

    /// Test hooks (env only): sign in with the given account, then open the view named in GNOBBI_EGUI_VIEW.
    fn test_hooks(&mut self, ctx: &egui::Context) {
        // GNOBBI_EGUI_PICK=file|image|photos|files|share: opens that system dialog once (from frame 60; photos and files take several) and prints the answer; for a simulator run.
        if let Ok(kind) = matrix_desktop::hooks::var("GNOBBI_EGUI_PICK") {
            static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if ctx.cumulative_pass_nr() < 60 { ctx.request_repaint(); } // an idle login screen never reaches frame 60 by itself
            if ctx.cumulative_pass_nr() >= 60 && !STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                match kind.as_str() {
                    "share" => {
                        let p = pick::save_dir().join("prova.txt");
                        let _ = std::fs::write(&p, b"gnobbi share sheet test");
                        pick::share(&p);
                    }
                    k @ ("photos" | "files") => pick::start_many(ctx, "hook", k == "photos"),
                    k => pick::start(ctx, "hook", k == "image"),
                }
            }
            for p in pick::done_many("hook") {
                eprintln!("gnobbi: picked {} ({} bytes)", p.display(), std::fs::metadata(&p).map_or(0, |m| m.len()));
            }
        }
        // GNOBBI_EGUI_ADD=1: once the first account is in the catalog, add a second one (signed in by GNOBBI_EGUI_LOGIN2).
        if !self.hook_added
            && self.recorded
            && matrix_desktop::hooks::var_os("GNOBBI_EGUI_ADD").is_some()
            && let Some(d) = &self.dirs
        {
            self.hook_added = true;
            self.login_sent = false;
            let next = matrix_desktop::AccountCatalog::load(d).next_root(d);
            self.open_account(next);
        }
        let Some(s) = &mut self.session else { return };
        // The test accounts have nothing to recover: skip the step so the other views can be reached.
        if self.auto_login.is_some() && s.recovering() && s.form.recovery_key().is_none() && matrix_desktop::hooks::var_os("GNOBBI_EGUI_KEEP_RECOVERY").is_none() {
            s.defer_recovery();
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
        if s.account.is_some() && matrix_desktop::hooks::var_os("GNOBBI_EGUI_VIEW").is_some() {
            let view = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
            // The first room with something to read, once the list has arrived.
            // GNOBBI_EGUI_ROOM=<name>: that room instead of the first.
            let wanted = matrix_desktop::hooks::var("GNOBBI_EGUI_ROOM").ok();
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
                s.opened = true; // one column: the room, not the list behind it
            }
            // GNOBBI_EGUI_EXPORT=<passphrase> / GNOBBI_EGUI_NEWKEY=1: the two key actions of the Devices window, once.
            if !self.hook_keys
                && let Some(v) = &mut s.verification
                && v.own().is_some()
            {
                if let Ok(pass) = matrix_desktop::hooks::var("GNOBBI_EGUI_EXPORT") {
                    let path = matrix_desktop::hooks::var_os("GNOBBI_EGUI_EXPORT_PATH").map(std::path::PathBuf::from).unwrap_or_default();
                    devices::start_export(v, path, &zeroize::Zeroizing::new(pass));
                    self.hook_keys = true;
                } else if matrix_desktop::hooks::var_os("GNOBBI_EGUI_NEWKEY").is_some() {
                    v.keys = Some(matrix_desktop::key_export::KeysPage::new(matrix_desktop::key_export::KeysKind::Reset));
                    v.reset_recovery(None);
                    self.hook_keys = true;
                }
            }
            // GNOBBI_EGUI_ALIAS=<local>: adds that address to the open room once its addresses are loaded.
            if !self.hook_alias
                && s.aliases.is_some()
                && let Ok(local) = matrix_desktop::hooks::var("GNOBBI_EGUI_ALIAS")
            {
                s.alias_action(matrix_engine::alias::AliasAction::Add { local, make_canonical: None });
                self.hook_alias = true;
            }
            // GNOBBI_EGUI_TYPE=<text>: puts the text in the composer and focuses it (to see the «@» list).
            if !self.hook_type
                && s.messages.is_some()
                && s.members.is_some()
                && let Ok(text) = matrix_desktop::hooks::var("GNOBBI_EGUI_TYPE")
            {
                s.draft = text;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
                self.hook_type = true;
            }
            // GNOBBI_EGUI_EMOJI=hover|click|sticker: keeps the small popup shown (hover) or opens the docked drawer on the Emoji (click) or Sticker tab; for screenshots on a seat without a pointer.
            if s.messages.is_some() && let Ok(mode) = matrix_desktop::hooks::var("GNOBBI_EGUI_EMOJI") {
                static OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if mode == "hover" {
                    let now = ctx.input(|i| i.time);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("emoji-hover"), (true, now)));
                } else if !OPENED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    ctx.data_mut(|d| {
                        d.insert_temp(egui::Id::new(stickers::TAB_ID), mode == "sticker");
                        d.insert_temp(egui::Id::new(composer::DOCK_ID), true);
                    });
                }
            }
            // GNOBBI_EGUI_ATTACH=<path>[:<path>...]: attaches those files to the open room's composer once (the same call a drop makes).
            // GNOBBI_EGUI_ATTACH_SHEET=1: opens the phone attach sheet once.
            if s.messages.is_some() && matrix_desktop::hooks::var_os("GNOBBI_EGUI_ATTACH_SHEET").is_some() {
                static SHEET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !SHEET.swap(true, std::sync::atomic::Ordering::Relaxed) { ctx.data_mut(|d| d.insert_temp(egui::Id::new("attach-sheet-open"), true)); }
            }
            // With GNOBBI_EGUI_ROOM the hooks wait for that room: the list may open another one first.
            let in_room = target.as_ref().is_none_or(|t| s.sidebar.selected().map(|r| &r.scope) == Some(t));
            if s.messages.is_some() && in_room && let Ok(path) = matrix_desktop::hooks::var("GNOBBI_EGUI_ATTACH") {
                static ATTACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !ATTACHED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    let files: Vec<_> = path.split(':').map(std::path::PathBuf::from).collect();
                    s.attach_all(&files, false);
                }
            }
            // A headless seat never focuses the window, and egui drops the text focus with it: say it is focused.
            if self.hook_type && matrix_desktop::hooks::var_os("GNOBBI_EGUI_TYPE").is_some() {
                ctx.input_mut(|i| i.events.push(egui::Event::WindowFocused(true)));
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
            }
            // GNOBBI_EGUI_SEND=<text>: sends it once into the open room (disposable servers only).
            if !self.hook_sent
                && in_room
                && s.messages.as_ref().is_some_and(|m| !m.events().is_empty())
                && (matrix_desktop::hooks::var_os("GNOBBI_EGUI_ATTACH").is_none() || s.attachment.is_some()) // the files are read in the background: wait for the card
                && let Ok(text) = matrix_desktop::hooks::var("GNOBBI_EGUI_SEND")
            {
                s.send(text);
                self.hook_sent = true;
                self.hook_menu_at = Some(std::time::Instant::now());
            }
            // View «viewer»: the viewer on the room's first picture once it is loaded; GNOBBI_EGUI_VIEWER_STEP=<n> then steps n pictures on, 4 s later.
            // GNOBBI_EGUI_UPLOAD_FIXTURE=1: one upload card at 40 %.
            if view == "viewer" && s.viewer.is_none() && self.hook_viewer.is_none() && let Some(item) = s.viewer_start() {
                s.open_viewer(item.id, item.video, item.name);
                self.hook_viewer = Some((std::time::Instant::now(), false));
            }
            if let Some((at, false)) = self.hook_viewer && at.elapsed().as_secs() >= 4 && let Some(n) = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEWER_STEP").ok().and_then(|n| n.parse().ok()) {
                s.step_viewer(n);
                self.hook_viewer = Some((at, true));
            }
            if matrix_desktop::hooks::var_os("GNOBBI_EGUI_UPLOAD_FIXTURE").is_some() && s.uploads.is_empty() {
                s.uploads.push(session::Upload { id: matrix_engine::sending::fresh_transaction_id(), name: "vacanze-2026.mov".into(), sent: 12 << 20, total: 30 << 20, room: None });
            }
            // View «thread»: the drawer of the room's first thread; GNOBBI_EGUI_THREAD_TYPE=<text> is typed into its composer.
            if view == "thread"
                && s.thread.is_none()
                && let Some(matrix_domain::TimelineId::Server(root)) = s.messages.as_ref().and_then(|m| m.events().iter().find(|e| e.thread.is_some()).map(|e| e.id.id.clone()))
            {
                s.open_thread(root);
                if let (Some(th), Ok(text)) = (&mut s.thread, matrix_desktop::hooks::var("GNOBBI_EGUI_THREAD_TYPE")) { th.draft = text; }
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
                            match matrix_desktop::hooks::var("GNOBBI_EGUI_CONFIRM") {
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
                && let Ok(what @ ("profile" | "role")) = matrix_desktop::hooks::var("GNOBBI_EGUI_MEMBER").as_deref()
                && let Some(user) = s.members.as_ref().and_then(|l| {
                    let mut others = l.rows().iter().filter(|m| m.user_id != s.user);
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
                && let Ok(what) = matrix_desktop::hooks::var("GNOBBI_EGUI_MEMBER")
                && let Some(a) = s.admin.as_ref().filter(|a| a.perms.can_kick || a.perms.can_invite)
            {
                let list = s.space_view.as_ref().and_then(|v| v.members.as_ref()).or(s.members.as_ref()).map(|l| l.rows());
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
                    if what == "invite" { s.new_term = matrix_desktop::hooks::var("GNOBBI_EGUI_TERM").unwrap_or_default(); }
                    if let Some(a) = &mut s.admin
                        && let Some(d) = &mut a.dialog
                    {
                        d.reason = matrix_desktop::hooks::var("GNOBBI_EGUI_REASON").unwrap_or_default();
                        if matrix_desktop::hooks::var_os("GNOBBI_EGUI_CONFIRM").is_some() {
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
                // Test hook: the card of the first other member opened as a click on a sender in the chat does.
                "chat-profile" if s.profile.is_none() && let Some(user) = s.members.as_ref().and_then(|l| l.rows().iter().find(|m| m.user_id != s.user).map(|m| m.user_id.clone())) => s.open_profile_from_chat(user),
                "search" => s.search_open = true,
                // Test hook: «Media, files and links» (GNOBBI_EGUI_SHARED_TAB=files|links picks the tab) 30 s after the hook's own send, once it is in the room.
                "shared" if s.shared.is_none() && self.hook_sent && self.hook_menu_at.is_some_and(|t| t.elapsed().as_secs() >= 30) => {
                    s.info_open = true;
                    s.open_shared(match matrix_desktop::hooks::var("GNOBBI_EGUI_SHARED_TAB").as_deref() { Ok("files") => matrix_engine::operations::SharedTab::Files, Ok("links") => matrix_engine::operations::SharedTab::Links, _ => matrix_engine::operations::SharedTab::Media });
                }
                // Test hook: GNOBBI_EGUI_TERM=<text> typed in the room list's field after GNOBBI_EGUI_TERM_AFTER seconds, so the index has filled (the messages of every room are searched for real).
                "list-search" | "list-search-open" if s.query.is_empty() && self.shot.as_ref().is_some_and(|(_, t0)| t0.elapsed().as_secs() >= matrix_desktop::hooks::var("GNOBBI_EGUI_TERM_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(0)) => {
                    s.query = matrix_desktop::hooks::var("GNOBBI_EGUI_TERM").unwrap_or_default();
                    s.sidebar.set_query(&s.query);
                }
                // The same, then the first message found is chosen once (as a tap would): its room opens on that message.
                "list-search-open" if !self.hook_hit => {
                    if let matrix_engine::operations::MessageSearchState::Results { hits, .. } = &s.all_search
                        && let Some(hit) = hits.first().cloned()
                    {
                        self.hook_hit = s.open_message_hit(&hit);
                    }
                }
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
                // «Send feedback» as if tapped; once more after a failure, as a retry by the user.
                "feedback" if s.sync == Some(RoomSyncStatus::Ready) => {
                    static TAPS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
                    let taps = TAPS.load(std::sync::atomic::Ordering::Relaxed);
                    if taps == 0 || (taps == 1 && s.room_error.is_some()) {
                        TAPS.store(taps + 1, std::sync::atomic::Ordering::Relaxed);
                        s.send_feedback();
                    }
                }
                "accounts" => self.rail.set_accounts(true),
                "customize" => self.rail.set_customize(true),
                _ => {}
            }
        }
    }

    /// Tray clicks, the menu and icon state, and what closing the window does (hide when the tray can be reached).
    #[cfg(any(target_os = "ios", target_os = "android"))]
    fn tray(&mut self, ctx: &egui::Context) {
        // mobile: no tray, no close-to-tray; the icon badge is the unread total of every account.
        let total = self.session.iter().chain(self.parked.iter().map(|p| &p.session)).fold(0u64, |n, s| n.saturating_add(s.unread_total()));
        matrix_desktop::notifications::set_badge(total, ctx.input(|i| i.focused));
    }

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
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
        // One layout switch: Compact = one column (list, or chat, or a side panel); the old «narrow» middle tier is gone.
        let single = layout::compact(ui.ctx());
        let narrow = single;
        if !narrow {
            self.list_open = false;
        }
        // Test hook: GNOBBI_EGUI_VIEW=list shows the list of a narrow window without a click.
        if narrow && matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("list") {
            self.list_open = true;
        }
        let drag = self.back_swipe(ui, single);
        let side = self.side_on_top_now(ui, single);
        let covered = side.is_some();
        // One column with a room: the chat slides over the list; while it is on the way both pages are drawn, each in its shifted rect.
        let sliding = single && !self.rail.any_drawer() && self.session.as_ref().is_some_and(|s| s.sidebar.selected().is_some());
        // shortcut: a side panel (info, member, search, thread) is part of the chat page, so the page does not follow the finger while one is on top (the swipe closes it on release); give the panels their own page to animate them.
        let held = match drag { Some(mobile::BackDrag::Moving(p)) if !covered => Some(p), _ => None };
        let both = self.nav.step(sliding, self.list_open, held, std::time::Instant::now(), mobile::reduce_motion());
        let screen_width = ui.ctx().viewport_rect().width();
        let (mut list_dx, chat_dx) = if both { mobile::nav_offsets(self.nav.progress, screen_width, dir::rtl()) } else { (0., 0.) };
        // A drawer page on its way over the list: the list trails under it like under the chat.
        if single && self.list_open && !both {
            list_dx = mobile::nav_offsets(1. - rail::pages_open(ui.ctx()).1, screen_width, dir::rtl()).0;
        }
        let show_list = self.list_open || both;
        let (pick_account, list_only) = self.rail_and_list(ui, t, (narrow, single), both, list_dx, show_list);
        if let Some((i, scope)) = self.merged_open.take() {
            self.open_merged_room(i, scope);
        }
        let (pick, list_only) = account_switch_effect(pick_account, &self.root, list_only);
        if let Some(root) = pick {
            self.rail.one_account = true;
            self.open_account(root);
        }
        if list_only {
            self.overlays(ui, t);
            return;
        }
        let mut over = both.then(|| {
            mobile::nav_cover(ui, self.nav.progress, chat_dx, list_fill(t));
            mobile::nav_layer(ui, chat_dx)
        });
        let ui: &mut egui::Ui = match &mut over { Some(layer) => layer, None => ui };
        self.header(ui, t, state, narrow, single);
        self.info_panel(ui, t, single, side);
        self.member_panel(ui, t, single);
        self.search_panel(ui, t, single, side);
        self.thread_panel(ui, t, single, side);
        self.composer_bar(ui, t, covered);
        self.overlays(ui, t);
        self.timeline_panel(ui, t, covered);
        // On the way the pages take no taps or scrolls: a shield on top of both swallows them (the finger that drags is already down).
        if both && !self.nav.dragging {
            ui.interact(ui.ctx().viewport_rect(), egui::Id::new("nav-shield"), egui::Sense::click_and_drag());
        }
    }

    /// Left-edge swipe = the back button; the chat follows the finger.
    fn back_swipe(&mut self, ui: &mut egui::Ui, single: bool) -> Option<mobile::BackDrag> {
        let drag = if single && !self.list_open && !self.rail.any_drawer() { mobile::back_drag(ui.ctx()) } else { None };
        // The touch menu is open: the swipe closes it and goes no further.
        let drag = match &mut self.session {
            Some(s) if drag.is_some() && s.lift.is_some() => {
                if s.menu.is_some() { s.close_menu(); }
                mobile::cancel_back_drag(ui.ctx());
                None
            }
            _ => drag,
        };
        if let Some(mobile::BackDrag::Released(back)) = drag {
            self.list_open = back;
        }
        drag
    }

    /// The side panel on top (one column only), after «Back» has closed what it had to.
    fn side_on_top_now(&mut self, ui: &mut egui::Ui, single: bool) -> Option<Side> {
        let mut side = None;
        if let Some(s) = &mut self.session {
            let room = s.sidebar.selected().map(|r| r.scope.clone());
            // A room just created, joined or opened from an alert is shown, not left behind the list.
            let started = egui::Id::new("shell-started");
            if std::mem::take(&mut s.opened) {
                self.list_open = false;
            }
            // The open chat was left: back to the list, not into another chat.
            if std::mem::take(&mut s.left_open) {
                self.list_open = true;
            }
            // Started as one column (a phone): the list comes first, the room reopened from the last run stays behind it.
            if single && ui.data(|d| d.get_temp::<bool>(started)).is_none() {
                self.list_open = true;
            }
            ui.data_mut(|d| d.insert_temp(started, true));
            #[cfg(debug_assertions)]
            if room.is_some() && matrix_desktop::hooks::var_os("GNOBBI_DEBUG_LONGPRESS_MSG").is_some() {
                self.list_open = false; // test hook: the long-pressed message needs the chat on screen
            }
            s.settle_chat_profile();
            if single {
                side = side_on_top((s.info_open || s.chat_profile.is_some()) && room.is_some(), s.profile.is_some(), s.search_open, s.thread.is_some());
                // «Back» closes the side panel on top before it shows the list.
                if self.list_open && room.is_some() && let Some(top) = side {
                    match top {
                        Side::Member => s.profile = None,
                        Side::Info => if s.members_page { s.members_page = false } else if let Some(scope) = s.shared.as_ref().map(|x| x.scope.clone()) { s.close_shared(scope); } else { s.info_open = false },
                        Side::Search => s.search_open = false,
                        Side::Thread => s.thread = None,
                    }
                    self.list_open = false;
                    s.settle_chat_profile();
                    side = side_on_top(s.info_open || s.chat_profile.is_some(), s.profile.is_some(), s.search_open, s.thread.is_some());
                }
                // Without a room there is nothing to show but the list.
                self.list_open |= room.is_none();
            }
        }
        side
    }

    /// The spaces rail and the room list left of the chat: the account picked in the rail menu and whether only the list is on screen.
    fn rail_and_list(&mut self, ui: &mut egui::Ui, t: &DesignTokens, (narrow, single): (bool, bool), both: bool, list_dx: f32, show_list: bool) -> (Option<std::path::PathBuf>, bool) {
        // The shown account sits on top of the spaces rail: a click lists the others, each with a dot below it (filled = unread).
        let mut pick_account = None;
        let mut list_only = false;
        let mut others = Vec::new();
        let (active_id, active_label, account) = self.account_rows(&mut others);
        // The badges of «Non lette» and «Tutte le chat» count every account, the parked ones too (cached per sidebar, no walk of the rooms).
        // With «Lista unica» off they count the shown account only, as the list does.
        let totals = matrix_desktop::badge_totals(self.prefs.unified_list, self.session.iter().chain(self.parked.iter().map(|p| &p.session)).map(|s| s.sidebar.counts()));
        if let Some(session) = &mut self.session {
            if std::mem::take(&mut self.rail.feedback) {
                session.send_feedback();
                (self.rail.edit, self.rail.settings, self.rail.devices, self.rail.accounts, self.rail.customize) = (false, false, false, false, false);
            }
            rail::edit_window(ui.ctx(), session, &mut self.new_form, &mut self.rail, account, self.dirs.as_ref(), t);
            let mut under = (both || list_dx != 0.).then(|| mobile::nav_layer(ui, list_dx));
            let ui: &mut egui::Ui = match &mut under { Some(layer) => layer, None => &mut *ui };
            if !single || show_list {
                if single { mobile::rail_background(ui, chat_background(t), list_dx); }
                pick_account = rail_panel(ui, session, &mut self.rail, account, totals, &others, (&active_label, &active_id), t);
            }
            // More than one running account and no account or space picked in the rail: the list holds the rooms of all of them.
            let merged = matrix_desktop::unified_applies(self.prefs.unified_list, self.rail.one_account, self.parked.len(), session.sidebar.active_space().is_some());
            let unified = merged.then(|| list::Unified { parked: &mut self.parked, cache: &mut self.merged, open: &mut self.merged_open });
            if !narrow || show_list {
                // Narrow: the list takes the whole width left of the rail and the chat is not drawn; a tap on a row, the open one too, goes to the chat.
                let list_fill = list_fill(t);
                if narrow {
                    narrow_list(ui, session, &mut self.list_open, list_fill, unified, t);
                } else {
                    wide_list(ui, session, &mut self.prefs, self.dirs.as_ref(), list_fill, unified, t);
                }
                if narrow && !both {
                    list_only = true;
                }
            }
        }
        (pick_account, list_only)
    }

    /// The other accounts (parked or in the catalog) and the shown one, for the rail.
    fn account_rows(&mut self, others: &mut Vec<(std::path::PathBuf, String, String, avatar::Avatar, Option<u64>)>) -> (String, String, u64) {
        let unread = |s: &Session| s.sidebar.navigation_entries().first().map_or(0, |e| e.unread);
        let catalog = self.dirs.as_ref().map(matrix_desktop::AccountCatalog::load);
        let entry_for = |root: &std::path::Path| catalog.as_ref().zip(self.dirs.as_ref()).and_then(|(c, d)| c.accounts().iter().find(|e| c.data_dir(d, e.id).as_deref() == Some(root)));
        *others = self.parked.iter().map(|p| {
            let entry = entry_for(&p.root);
            let id = entry.and_then(|e| e.user_id.as_deref()).unwrap_or(&p.session.user);
            let name = account_row_label(Some(&p.session.account_name), entry.map(|e| e.label.as_str()), Some(id));
            (p.root.clone(), name.to_owned(), id.to_owned(), p.session.own_avatar(), Some(unread(&p.session)))
        }).collect();
        if let (Some(c), Some(d)) = (&catalog, &self.dirs) {
            for entry in c.accounts() {
                if let Some(root) = c.data_dir(d, entry.id)
                    && root != self.root && !others.iter().any(|row| row.0 == root)
                {
                    let id = entry.user_id.as_deref().unwrap_or("");
                    let name = account_row_label(None, Some(&entry.label), Some(id));
                    others.push((root, name.to_owned(), id.to_owned(), avatar::Avatar::Missing, None));
                }
            }
        }
        let active_entry = entry_for(&self.root);
        let active_id = active_entry.and_then(|e| e.user_id.as_deref()).filter(|id| !id.trim().is_empty()).or_else(|| self.session.as_ref().map(|s| s.user.as_str())).unwrap_or_default().to_owned();
        let active_label = account_row_label(self.session.as_ref().map(|s| s.account_name.as_str()), active_entry.map(|e| e.label.as_str()), Some(&active_id)).to_owned();
        // The catalog lists the account once its login is recorded; until then the layout is the default one, kept under 0.
        if self.recorded && self.rail_account.as_ref().is_none_or(|(root, _)| *root != self.root)
            && let Some(d) = &self.dirs
        {
            self.rail_account = matrix_desktop::AccountCatalog::load(d).id_for_root(d, &self.root).map(|id| (self.root.clone(), id.as_u64()));
        }
        let account = self.rail_account.as_ref().filter(|(root, _)| *root == self.root).map_or(0, |(_, id)| *id);
        (active_id, active_label, account)
    }

    /// The header bar of the chat.
    fn header(&mut self, ui: &mut egui::Ui, t: &DesignTokens, state: &str, narrow: bool, single: bool) {
        let has_room = self.session.as_ref().is_some_and(|s| s.messages.is_some());
        let (info_on, search_on) = self.session.as_ref().map_or((false, false), |s| (s.info_open, s.search_open));
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        let badge = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| (r.initials.clone(), r.color)));
        let head_pic = self.session.as_ref().map(|s| avatar::lookup(&s.avatars, s.sidebar.selected().and_then(|r| r.info.avatar.as_deref())));
        // A phone turned on its side with the keyboard up: the header would take what is left of the height (the composer needs it).
        let type_in_landscape = mobile::touch() && mobile::keyboard() > 0. && ui.ctx().content_rect().aspect_ratio() > 1.;
        if !type_in_landscape {
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
                    if has_room && let Some(s) = &mut self.session { list::header_menu(ui, s, t); }
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
        if single && let Some(error) = self.session.as_ref().and_then(|s| s.room_error.as_deref()) {
            egui::Panel::top("room-error").show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| { ui.colored_label(rgb(t.error), error); }));
        }
        }
    }

    /// The room info panel (right).
    fn info_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, single: bool, side: Option<Side>) {
        if let Some(s) = &mut self.session {
            if s.info_open && s.sidebar.selected().is_some() {
                s.refresh_member_order();
            } else {
                (s.member_order_fresh, s.members_page) = (false, false);
                s.member_query.clear();
            }
        }
        if self.session.as_ref().is_some_and(|s| s.info_open && s.shared.is_some() && (!single || side == Some(Side::Info)) && s.sidebar.selected().is_some()) {
            self.shared_panel(ui, t, single);
            return;
        }
        if let Some(s) = &mut self.session
            && s.info_open
            && (!single || side == Some(Side::Info))
            && let Some(room) = s.sidebar.selected()
        {
            let mut page_back = false;
            let mut out = info::Out::default();
            let mut alias_input = std::mem::take(&mut self.alias_input);
            let mut alias_action = None;
            let room_id = room.scope.room_id.as_str().to_owned();
            let mut status = s.status_choices.get(&room_id);
            let (status_was, hidden_status) = (status, s.messages.as_ref().filter(|m| m.scope() == &room.scope).map_or(0, |m| m.hidden_status_count()));
            let mut view = info::AliasView { data: s.aliases.as_ref(), error: s.alias_error, input: &mut alias_input, action: None };
            dir::panel_end("info").exact_size(if single { ui.available_width() } else { 300. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| {
                let admin = s.admin.as_ref().filter(|a| a.scope == room.scope);
                if s.members_page && let Some(list) = s.members.as_mut() {
                    let p = members::page(ui, list, &mut s.member_query, None, admin, &s.user, &s.avatars, t);
                    (out.member, page_back) = (p.act, p.back);
                } else {
                    out = info::show(ui, room, s.members.as_ref(), s.members_failed, admin, &s.user, &mut view, (&mut status, hidden_status), &s.avatars, t);
                }
            }));
            alias_action = view.action.take();
            self.alias_input = alias_input;
            if let Some(s) = &mut self.session {
                if out.retry_members { s.retry_members(); }
                s.members_page = (s.members_page && !page_back) || out.members_page;
            }
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
            if out.shared && let Some(s) = &mut self.session {
                s.open_shared(matrix_engine::operations::SharedTab::Media);
            }
        }
    }

    /// «Media, files and links» in place of the info panel.
    fn shared_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, single: bool) {
        let Some(s) = &mut self.session else { return };
        let Some(mut st) = s.shared.take() else { return };
        let (transfer, scope, tab) = (s.transfer(), st.scope.clone(), st.tab);
        let mut out = shared::Out::default();
        let cols = shared::columns(single);
        dir::panel_end("shared").exact_size(if single { ui.available_width() } else { 420. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| out = shared::show(ui, &mut st, &s.pictures, transfer, cols, t)));
        s.shared = Some(st);
        if out.back { s.close_shared(scope); return; }
        if let Some(tab) = out.tab.filter(|n| *n != tab) { s.open_shared(tab); }
        if let Some((id, video, name)) = out.picture { s.open_shared_viewer(id, video, name); }
        if let Some((id, name)) = out.open { s.open_file(id, name); }
        if let Some((id, name)) = out.save { s.save(id, &name); }
        if out.cancel { s.cancel_transfer(); }
        if out.more { s.more_shared(); }
        if out.link.is_some() { self.pending_web = out.link; }
    }

    /// The member card panel (right).
    fn member_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, single: bool) {
        if let Some(s) = &mut self.session
            && s.card_open()
            && let Some((id, profile)) = s.profile.clone()
        {
            let (mut close, mut flip, mut moderation, mut action) = (false, None, None, None);
            let (rights, failed, busy) = (s.rights.clone(), s.moderation_failed, s.moderating);
            let me = id == s.user;
            let can_verify = s.verification.as_ref().is_some_and(|v| v.can_begin());
            let picture = s.profile_picture(profile.as_ref().and_then(|p| p.avatar.as_deref()));
            dir::panel_end("member").exact_size(if single { ui.available_width() } else { 300. }).resizable(false).show_separator_line(false).frame(bar(t)).show(ui, |ui| dir::scope(ui, |ui| flip = info::card(ui, &id, profile.as_ref(), rights.as_ref(), failed, busy, s.new_failed, me, can_verify, &picture, &mut action, &mut self.confirm, &mut moderation, &mut close, t)));
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
                        (s.info_open, s.chat_profile) = (false, None);
                        close = true;
                    }
                }
                Some(info::ProfileAction::Picture) => {
                    if let Some(p) = &profile && let Some(mxc) = &p.avatar { s.open_avatar_viewer(mxc, p.name.clone()); }
                }
                Some(info::ProfileAction::Verify) => { if let Some(v) = &mut s.verification { v.begin_user(id.clone()); } }
                Some(info::ProfileAction::ReadReceipt) => {
                    if let Some(event) = rights.as_ref().and_then(|r| r.read_receipt.as_ref()) {
                        s.search_jump = Some((matrix_domain::EventId::new(event), false));
                        s.jump_term = None;
                        s.notice = None;
                        (s.info_open, s.chat_profile) = (false, None);
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
    }

    /// The in-room search panel (right).
    fn search_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, single: bool, side: Option<Side>) {
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
                s.jump_term = Some(s.search_term.trim().to_owned());
                s.notice = None;
            }
        }
    }

    /// The thread panel (right).
    fn thread_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, single: bool, side: Option<Side>) {
        if let Some(s) = &mut self.session
            && (!single || side == Some(Side::Thread))
            && let Some(mut th) = s.thread.take()
        {
            let (mut close, mut out) = (false, info::ThreadOut::default());
            let mut acts = timeline::Acts { now_playing: s.audio_now(), wishes: s.ops.wishes().to_vec(), transfer: s.transfer(), saved: s.saved_ids(), ..Default::default() };
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
            if let Some(image) = out.pasted { s.attach_image(image, true); }
            s.attach_all(&out.picked, true);
            s.attach_acts(&out.attach, true);
            match out.voice {
                Some(composer::VoiceAct::Start) => s.start_recording(true, ui.ctx()),
                Some(composer::VoiceAct::Send) => s.stop_recording(true),
                Some(composer::VoiceAct::Cancel) => s.stop_recording(false),
                None => {}
            }
            if let Some(body) = out.send { s.send_thread(body); }
            if let Some((id, name)) = out.save { s.save(id, &name); }
            if let Some((id, video, name)) = acts.picture.take() { s.open_viewer(id, video, name); }
            if let Some((id, name)) = acts.open_file.take() { s.open_file(id, name); }
            if let Some(id) = acts.reveal_file.take() { s.reveal_file(&id); }
            if std::mem::take(&mut acts.cancel_transfer) { s.cancel_transfer(); }
            if let Some(user) = acts.profile.take() { s.open_profile_from_chat(user); }
            if let Some(id) = acts.play.take() { s.play_audio(id); }
            if let Some((id, frac, phase)) = acts.scrub.take() { s.scrub_audio(&id, frac, phase); }
            s.reaction_acts(std::mem::take(&mut acts.react), ui.ctx(), t);
            if acts.open_link.is_some() { self.pending_web = acts.open_link; }
        }
    }

    /// The composer at the bottom of the chat.
    fn composer_bar(&mut self, ui: &mut egui::Ui, t: &DesignTokens, covered: bool) {
        let mut to_send = None;
        // The drawer's open flag lives in the context while drawing and is saved to the preferences when it changes.
        let dock_id = egui::Id::new(composer::DOCK_ID);
        let dock = ui.data(|d| d.get_temp::<bool>(dock_id)).unwrap_or(self.prefs.emoji_dock);
        // Same for the drawer's tab (shared with the narrow popup): seeded from the preferences once, saved when it changes.
        let tab_id = egui::Id::new(stickers::TAB_ID);
        let tab = ui.data(|d| d.get_temp::<bool>(tab_id)).unwrap_or(self.prefs.emoji_sticker_tab);
        ui.data_mut(|d| { d.insert_temp(dock_id, dock); d.insert_temp(tab_id, tab); });
        composer::panel(bar(t)).show(ui, |ui| dir::scope(ui, |ui| {
            if let Some(s) = &mut self.session
                && s.messages.is_some()
                && !covered
            {
                let mode = s.target.as_ref().map_or(composer::Mode::Send, |(_, m)| *m);
                let mut cancel = false;
                let mut attach_acts = attach::Acts::default();
                let mut open_poll = false;
                let mut picked_files = Vec::new();
                let mut pasted_image = None;
                let mut voice_act = None;
                if let Some(remaining) = status_pill::expire_line(&mut s.notice, &mut s.notice_shown, i18n::t("Saved"), std::time::Instant::now()) {
                    if let Some(notice) = &s.notice {
                        // Above everything (the phone viewer covers the chat) and inside the safe area, never at the screen edge.
                        let screen = ui.ctx().content_rect();
                        egui::Area::new(egui::Id::new("notice-toast")).order(egui::Order::Foreground).interactable(false).pivot(egui::Align2::CENTER_BOTTOM)
                            .fixed_pos(egui::pos2(screen.center().x, screen.bottom() - 88.)).show(ui.ctx(), |ui| {
                                ui.set_max_width(screen.width() - 32.);
                                egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(notice));
                            });
                    }
                    ui.ctx().request_repaint_after(remaining);
                }
                let members = s.members.as_ref().map_or(&[][..], |l| l.rows());
                let recording = s.recording(false);
                let mut stop = None;
                for u in s.room_uploads() {
                    if ui.push_id(("upload", u.id.as_str()), |ui| media_cards::upload_card(ui, &u.name, u.sent, u.total, t)).inner { stop = Some(u.id.clone()); }
                }
                to_send = composer::show(ui, "", &mut s.draft, mode, s.attachment.as_mut(), &mut cancel, &mut attach_acts, &mut open_poll, &mut picked_files, &mut pasted_image, recording, &mut voice_act, &members, &mut s.mentions, Some((&mut s.stickers, &s.avatars)), t);
                if let Some(id) = stop { s.cancel_upload(&id); } // after the composer: `members` borrows the session until then
                if let Some(image) = pasted_image {
                    s.attach_image(image, false);
                }
                s.attach_acts(&attach_acts, false);
                s.poll_open |= open_poll;
                if let Some(sticker) = s.stickers.chosen.take() {
                    s.send_sticker(sticker);
                }
                s.attach_all(&picked_files, false);
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
        let now = ui.data(|d| d.get_temp::<bool>(dock_id)).unwrap_or(dock);
        if now != self.prefs.emoji_dock && let Some(dirs) = &self.dirs {
            self.prefs.emoji_dock = now;
            if let Err(e) = Preferences::update(dirs, |p| p.emoji_dock = now) { eprintln!("emoji drawer state not saved: {e}"); }
            ui.ctx().request_repaint();
        }
        let tab_now = ui.data(|d| d.get_temp::<bool>(tab_id)).unwrap_or(tab);
        if tab_now != self.prefs.emoji_sticker_tab && let Some(dirs) = &self.dirs {
            self.prefs.emoji_sticker_tab = tab_now;
            if let Err(e) = Preferences::update(dirs, |p| p.emoji_sticker_tab = tab_now) { eprintln!("emoji drawer tab not saved: {e}"); }
        }
        if let (Some(body), Some(s)) = (to_send, &mut self.session) {
            s.submit_draft(body);
        }
    }

    /// The timeline (or the invite) in the central panel.
    fn timeline_panel(&mut self, ui: &mut egui::Ui, t: &DesignTokens, covered: bool) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(chat_background(t))).show(ui, |ui| dir::scope(ui, |ui| {
            // The click that changes room lands before `drive_history` swaps the messages: drawing the old room under the new room's scroll state moved it to the bottom and saved that as its reading position.
            // A side panel has taken the whole width: nothing is laid out in the sliver left.
            if covered {
                return;
            }
            let timeline_view = ui.available_rect_before_wrap();
            let scroll = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| timeline_view.contains(p)) && (i.smooth_scroll_delta.y != 0. || (i.pointer.primary_down() && i.pointer.delta().y.abs() > 1.)));
            if scroll && let Some(s) = &mut self.session { s.cancel_unread_search(); }
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
                let (now, still) = (std::time::Instant::now(), mobile::reduce_motion());
                s.flash.clear_if_over(now, still);
                if let Some(after) = s.flash.repaint_in(now, still) { ui.ctx().request_repaint_after(after); }
                let flash = s.flash.lit(now, still).cloned();
                let mut acts = timeline::Acts { now_playing: s.audio_now(), flash_alpha: flash.as_ref().map_or(0., |id| s.flash.opacity_for(id, now, still)), flash_term: s.flash.term().map(str::to_owned), flash, lift: s.lift.as_ref().map(|l| (l.id.clone(), l.lifted())), wishes: s.ops.wishes().to_vec(), transfer: s.transfer(), saved: s.saved_ids(), ..Default::default() };
                let jump = s.search_jump.as_ref().and_then(|(id, _)| messages.events().iter().position(|e| e.id.id == matrix_domain::TimelineId::Server(id.clone())));
                let room = s.sidebar.selected();
                let members = room.map_or(3, |r| r.info.joined_members);
                let readers = room.map(|r| messages.readers_by_row(&r.info.public_read_receipts)).unwrap_or_default();
                s.heights.fetch = s.fetch();
                // Scroll state per room: one shared id carried the old room's offset into the new one (jump on switch).
                let (first, menu, save, thread) = ui.push_id(room.map(|r| r.scope.room_id.as_str()), |ui| timeline::show(ui, room.map_or(0, |r| r.unread), messages.events(), messages, &mut s.heights, &s.pictures, &s.avatars, &mut acts, members, &readers, t, jump, None)).inner;
                if acts.flash_drawn && let Some(id) = &acts.flash { s.flash.mark_drawn(id, now); }
                if jump.is_some() && let Some((root, open_thread)) = s.search_jump.take() {
                    if open_thread { s.open_thread(root); } else { s.flash.set_target(root, s.jump_term.take()); }
                }
                // The touch menu is open: it follows its bubble, and the chat under it takes nothing (the finger that opened it is still down).
                let (menu, save, thread) = match &mut s.lift {
                    Some(lift) => {
                        lift.rect = acts.lift_rect.unwrap_or(lift.rect);
                        lift.view = acts.view.unwrap_or(lift.view);
                        (acts.reply_jump, acts.picture, acts.play, acts.vote, acts.open_link, acts.profile, acts.scrub) = (None, None, None, None, None, None, None);
                        acts.react = Default::default();
                        (None, None, None)
                    }
                    None => (menu, save, thread),
                };
                // Not in memory: the same loop as a search hit loads older pages until it shows up.
                if let Some(id) = acts.reply_jump.take() { s.search_jump = Some((id, false)); s.jump_term = None; }
                s.first_visible = first;
                if std::mem::take(&mut s.heights.retry) { s.retry_history(); }
                if let Some((id, pos)) = menu {
                    s.open_menu(id.clone(), pos);
                    // A long press: lift the bubble; whatever the finger was dragging lets go.
                    if let (Some(rect), Some(view), true) = (acts.held, acts.view, s.ops.selection() == Some(&id)) {
                        s.lift = Some(touch_menu::Lift::new(id, rect, view));
                        ui.ctx().stop_dragging();
                        mobile::haptic_tick();
                    }
                }
                if let Some((id, name)) = save {
                    s.save(id, &name);
                }
                if let Some(root) = thread {
                    s.open_thread(root);
                }
                if let Some((id, video, name)) = acts.picture.take() { s.open_viewer(id, video, name); }
                if let Some((id, name)) = acts.open_file.take() { s.open_file(id, name); }
                if let Some(id) = acts.reveal_file.take() { s.reveal_file(&id); }
                if std::mem::take(&mut acts.cancel_transfer) { s.cancel_transfer(); }
                if let Some(user) = acts.profile.take() { s.open_profile_from_chat(user); }
                if let Some(id) = acts.play.take() {
                    s.play_audio(id);
                }
                if let Some((id, frac, phase)) = acts.scrub.take() { s.scrub_audio(&id, frac, phase); }
                if let Some((id, ix)) = acts.vote {
                    s.vote(id, ix);
                }
                s.reaction_acts(std::mem::take(&mut acts.react), ui.ctx(), t);
                if acts.open_link.is_some() {
                    self.pending_web = acts.open_link;
                }
            }
            if !stale && let Some(s) = &mut self.session {
                // While the first unread is looked for the rows are hidden: the timeline draws its loading view (`loading`).
                if let Some(at) = s.unread_notice {
                    if status_pill::notice_alive(at.elapsed(), false) {
                        status_pill::show(ui, timeline_view, i18n::t("Unread messages not found"), at.elapsed(), t);
                    } else { s.unread_notice = None; }
                }
            }
            #[cfg(debug_assertions)]
            if matrix_desktop::hooks::var_os("GNOBBI_EGUI_FAKE_UNREAD_NOTICE").is_some() {
                status_pill::show(ui, timeline_view, i18n::t("Unread messages not found"), Duration::from_secs(1), t);
            }
        }));
    }

    /// The windows and drawers of the session (settings, account, devices, new chat…). Drawn in every layout: the one-column list returns before the chat.
    fn overlays(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        if let Some(s) = &mut self.session {
            message_menu(ui.ctx(), s, t);
            touch_menu::show(ui.ctx(), s, t);
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
            list::leave_modal(ui.ctx(), s);
            space::delete_window(ui.ctx(), s, t);
            members::windows(ui.ctx(), s, t);
        }
        for p in &mut self.parked {
            list::leave_modal(ui.ctx(), &mut p.session);
            // Only the shown account's shell reads it; left on, it would send the first tap on this account back to the list.
            p.session.left_open = false;
        }
        secret_clip::tick(ui.ctx());
        let account = self.session.as_ref().and_then(|s| s.account);
        if let Some(s) = &mut self.session
            && let Some(v) = &mut s.verification
        {
            // Debug builds only (screenshots): a fixed list of other devices, every frame so the engine's own answer cannot replace it.
            #[cfg(debug_assertions)]
            if let (Some(account_id), true) = (account, matrix_desktop::hooks::var_os("GNOBBI_EGUI_FAKE_DEVICES").is_some()) {
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
            let suppressed = account.is_some_and(|a| self.prefs.no_recovery_offer_accounts.contains(&a.as_u64())) || matrix_desktop::hooks::var_os("GNOBBI_EGUI_NO_OFFER").is_some();
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
                    let previous = self.root.clone();
                    self.open_account(next.clone());
                    // The way back from the sign-in form: only if the new profile really is on screen and the old one runs.
                    self.add_from = (self.root == next && self.session.is_some()).then_some(previous);
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
        let new = settings::show(ui.ctx(), &mut self.rail, &self.prefs, !self.parked.is_empty(), self.dirs.as_ref(), t)
            .or_else(|| customize::show(ui.ctx(), &mut self.rail, &self.prefs, self.system_scale, self.dirs.as_ref(), t));
        if let Some(new) = new {
            if new.icon != self.prefs.icon
                && let Some(icon) = app_icon(new.icon)
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon))));
                #[cfg(target_os = "macos")]
                mac_dock_icon(new.icon);
            }
            if new.language != self.prefs.language {
                matrix_desktop::i18n::apply(new.language, Some(ui.ctx()));
            }
            // Switching it back on merges the list at once, even after an account was picked from the menu.
            if new.unified_list && !self.prefs.unified_list {
                self.rail.one_account = false;
            }
            self.prefs = new;
            self.applied = None;
        }
    }

    fn account_recovery(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        let entry = self.catalog_entry();
        let mut retry = false;
        let mut again = false;
        let mut switch = None;
        let line = self.session.as_ref().and_then(|s| s.status().error_message()).unwrap_or(i18n::t("Could not open this account."));
        let who = entry.as_ref().map(|e| (e.label.as_str(), e.user_id.as_deref()));
        let (r, a, out) = recovery_screen(ui, t, who, line, |ui, size| {
            if let Some(d) = &self.dirs {
                let c = matrix_desktop::AccountCatalog::load(d);
                let others: Vec<_> = c.accounts().iter().filter_map(|e| {
                    let path = c.data_dir(d, e.id)?;
                    (path != self.root && self.parked.iter().any(|p| p.root == path && p.session.account.is_some()))
                        .then(|| (path, e.label.clone(), e.user_id.clone().unwrap_or_default()))
                }).collect();
                if ui.add_enabled(!others.is_empty(), egui::Button::new(i18n::t("Switch account")).min_size(size)).clicked() {
                    self.recovery_switch = !self.recovery_switch;
                }
                if self.recovery_switch {
                    for (path, name, id) in others {
                        if accounts::switch_row(ui, &name, &id, &avatar::Avatar::Missing, false, 0, t).clicked() { switch = Some(path); }
                    }
                }
            }
        });
        (retry, again) = (r, a);
        if out {
            let name = entry.as_ref().map_or("", |e| e.label.as_str());
            let id = entry.as_ref().and_then(|e| e.user_id.as_deref()).unwrap_or("");
            self.sign_out = Some(accounts::SignOut::new(self.root.clone(), name, id));
        }
        if let Some(path) = switch {
            self.startup = None;
            self.open_account(path);
        } else if retry || again {
            self.startup = None;
            self.account_notice = None;
            self.sign_out = None;
            let control = self.session.take().map(|s| {
                let c = s.control();
                c.stop();
                c
            });
            self.reopening = Some((control, again));
        }
    }

    fn reopen_account(&mut self, again: bool) {
        let entry = self.catalog_entry();
        if let Some(d) = &self.dirs {
            match Session::start_mode(d, self.root.clone(), self.trust.clone(), again) {
                Ok(mut s) => {
                    if again && let Some(e) = entry {
                        let (server, user) = account_login_fields(e.homeserver.as_deref(), e.user_id.as_deref());
                        s.homeserver = server;
                        s.user = user;
                    }
                    self.session = Some(s);
                    self.start_error = None;
                    self.signing_in_again = again;
                    self.remembered_root = None;
                    self.login_sent = false;
                }
                Err(e) => self.start_error = Some(e),
            }
        }
    }

    fn sign_in(&mut self, ui: &mut egui::Ui, t: &DesignTokens) {
        let mut retry = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show(ui, |ui| {
            let room = ui.available_height();
            // Scrolls when the keyboard (it shrinks the content rect) or a large text size leaves too little room.
            // The keyboard shrinks `room` in the very frame it opens; a scroll applied after layout comes a frame late, the focused TextEdit is
            // already clipped, stops asking for the IME and Android closes the keyboard (the show/hide loop). So the offset is fixed before layout.
            let (height_id, focus_id, scroll_id) = (egui::Id::new("signin_height"), egui::Id::new("signin_focus"), egui::Id::new("signin_scroll"));
            let (gap3, last) = (crate::metrics::get().gap * 3., ui.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(0.));
            let spacer = ((room - last) / 2.).max(gap3);
            let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if let (Some((top, bottom)), Some(cur)) = (ui.data(|d| d.get_temp::<(f32, f32)>(focus_id)), ui.data(|d| d.get_temp::<f32>(scroll_id)))
                && mobile::keyboard() > 0.
            {
                let want = keep_in_view(cur, spacer + top, spacer + bottom, room);
                if want != cur { area = area.vertical_scroll_offset(want); }
            }
            let out = area.show(ui, |ui| {
                let m = crate::metrics::get();
                let width = (ui.available_width() - 2. * m.gap * 3.).clamp(200., 400.).min(ui.available_width()); // never wider than the screen at a huge text size
                ui.add_space(spacer);
                let top = ui.cursor().top();
                ui.vertical_centered(|ui| {
                    ui.allocate_ui_with_layout(egui::vec2(width, 0.), dir::col(), |ui| match &mut self.session {
                        Some(session) => retry = sign_in_form(ui, session, t, self.prefs.icon),
                        None if self.reopening.is_some() => { ui.spinner(); }
                        None => { ui.label(i18n::t("Start failed")); }
                    });
                });
                ui.add_space(m.gap * 3.);
                let used = ui.cursor().top() - top;
                ui.data_mut(|d| d.insert_temp(height_id, used));
                // The focused field (any form: sign-in, unlock, recovery) relative to the form top, for the next frame.
                let field = ui.memory(|mem| mem.focused()).and_then(|id| ui.ctx().read_response(id)).map(|r| r.rect).filter(|r| r.top() >= top);
                ui.data_mut(|d| match field {
                    Some(r) => { d.insert_temp(focus_id, (r.top() - top, r.bottom() - top)); }
                    None => d.remove::<(f32, f32)>(focus_id),
                });
            });
            ui.data_mut(|d| d.insert_temp(scroll_id, out.state.offset().y));
        });
        if retry {
            let control = self.session.take().map(|s| { let c = s.control(); c.stop(); c });
            self.reopening = Some((control, false));
        }
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
        ui.set_min_width(crate::dir::fit(ui, 360.));
        ui.heading(i18n::t("Search"));
        if ui.add(crate::metrics::search_line(&mut s.new_term).id(egui::Id::unique("quick-search-field")).hint_text(i18n::t("Search"))).changed() {
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
                let r = ui.push_id(&hit.user_id, |ui| crate::rows::person_row(ui, name, &hit.user_id, duplicate, index == sel, &crate::avatar::Avatar::Missing, t)).inner;
                if index == sel && moved { r.scroll_to_me(None); }
                if r.clicked() { picked = Some(index); }
            }
            if !public.is_empty() { ui.label(i18n::t("Public rooms")); }
            for (i, hit) in public.iter().enumerate() {
                let r = public_room_row(ui, hits.len() + users.len() + i == sel, hit.name.clone(), joins[i]);
                if hits.len() + users.len() + i == sel && moved { r.scroll_to_me(None); }
                if r.clicked() { picked = Some(hits.len() + users.len() + i); }
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

/// Right-click menu: only what the engine says the user may do with the message (a long press shows the same entries in `touch_menu`).
fn message_menu(ctx: &egui::Context, s: &mut Session, t: &DesignTokens) {
    if s.lift.is_some() { return; }
    let Some(pos) = s.menu else { return };
    let fresh = std::mem::take(&mut s.menu_fresh);
    if fresh {
        ctx.data_mut(|d| {
            d.remove_temp::<bool>(egui::Id::new(touch_menu::MORE_ID));
            d.insert_temp(touch_menu::hull_id(), egui::Rect::NOTHING);
        });
    }
    let entries = touch_menu::entries(s);
    let readers = touch_menu::readers(s);
    let mut react: Option<&'static str> = None;
    let own = |key: &str| s.reacted(key);
    let mut picked = None;
    // Two cards with the look of the room list's menu: the reactions as a pill, the entries under it. Opens at the click, pushed back inside the window.
    let (cards, _) = touch_menu::desktop_menu(ctx, pos, |ui, cards| {
        let card = egui::Frame::menu(ui.style()).fill(rgb(t.surface));
        // Both shadows go under both cards: the second one's would darken the first.
        let shadows = [ui.painter().add(egui::Shape::Noop), ui.painter().add(egui::Shape::Noop)];
        let cast = |ui: &egui::Ui, n: usize, rect: egui::Rect, radius: CornerRadius| ui.painter().set(shadows[n], card.shadow.as_shape(rect, radius));
        let card = card.shadow(egui::Shadow::NONE);
        let Some((p, items)) = &entries else {
            cards.push(card.show(ui, |ui| ui.spinner()).response.rect);
            return;
        };
        if p.react {
            let radius = CornerRadius::same(if touch_menu::more_open(ui.ctx()) { 16 } else { 28 });
            let pill = card.corner_radius(radius).show(ui, |ui| react = touch_menu::reaction_rows(ui, &own, t));
            cast(ui, 0, pill.response.rect, radius);
            cards.push(pill.response.rect);
        }
        if !items.is_empty() || !readers.is_empty() {
            let list = card.show(ui, |ui| {
                ui.set_width(touch_menu::width(ui, items));
                picked = touch_menu::body(ui, items, &readers, &s.avatars, touch_menu::row_height(), t);
            });
            cast(ui, 1, list.response.rect, card.corner_radius);
            cards.push(list.response.rect);
        } else if !p.react {
            cards.push(card.show(ui, |ui| ui.label("—")).response.rect);
        }
    });
    match (picked, react) {
        (Some(action), _) => touch_menu::apply(ctx, s, action),
        (None, Some(key)) => s.react(key),
        (None, None) => {
            // The cards as drawn, not `area.response`: an area pushed inside the window keeps the rect it had before the push for hit-testing.
            // A press in the gap between the cards counts as inside (`in_menu`).
            let outside = !fresh && touch_menu::pressed_outside(ctx, &cards);
            if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                s.close_menu();
            }
        }
    }
}

/// The form for the flow the backend asked for: new profile (server, user, password) or unlock.
/// Scroll offset that keeps the focused field `[top, bottom]` (content coordinates) inside a view `height` tall, moving as little as possible.
fn keep_in_view(offset: f32, top: f32, bottom: f32, height: f32) -> f32 {
    if bottom - top >= height || top < offset { top.max(0.) } else if bottom > offset + height { bottom - height } else { offset }
}

/// True when «Retry» was clicked: the profile failed to open for good (the worker is gone), so the session restarts.
fn sign_in_form(ui: &mut egui::Ui, s: &mut Session, t: &DesignTokens, icon: matrix_desktop::IconChoice) -> bool {
    use matrix_engine::signin::SignInMode;
    if s.recovering() {
        recovery_form(ui, s, t);
        return false;
    }
    let status = s.status();
    // A failed open is final: no «Unlocking…» spinner on a dead worker, the error and the one action that works.
    if let (FormStatus::Failed(_), Some(message)) = (status, status.error_message()) {
        ui.label(RichText::new(message).color(rgb(t.error)));
        ui.add_space(crate::metrics::get().gap * 1.5);
        let retry = egui::Button::new(RichText::new(i18n::t("Retry")).strong().color(rgb(t.on_accent))).fill(rgb(t.accent)).corner_radius(CornerRadius::same(crate::metrics::get().radius.max(12.) as u8));
        return ui.add_sized([ui.available_width(), crate::metrics::button() + 4.], retry).clicked();
    }
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
        let side = (height * 1.5).max(64.);
        let id = egui::Id::unique(("signin-icon", format!("{icon:?}")));
        let texture = ui.data(|d| d.get_temp::<egui::TextureHandle>(id)).or_else(|| {
            let image = app_icon(icon).or_else(|| app_icon(matrix_desktop::IconChoice::default()))?;
            let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
            let texture = ui.ctx().load_texture(format!("signin-icon-{icon:?}"), pixels, Default::default());
            ui.data_mut(|d| d.insert_temp(id, texture.clone()));
            Some(texture)
        });
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            if let Some(texture) = &texture {
                egui::Image::new(texture).corner_radius(CornerRadius::same((side / 4.) as u8)).paint_at(ui, rect);
            }
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
    // iOS Passwords AutoFill (egui-winit hands over what it put in the hidden fields): replaces what the form has.
    if let Some((user, pass)) = ui.data_mut(|d| d.remove_temp::<(Option<String>, Option<String>)>(egui::Id::new("winit_autofill_credentials"))) {
        s.user = user.unwrap_or_else(|| std::mem::take(&mut s.user));
        s.password = pass.unwrap_or_else(|| std::mem::take(&mut s.password));
    }
    let mut enter = false;
    let mut focused = None;
    let mut field = |ui: &mut egui::Ui, text: &mut String, label: &str, secret: bool, purpose: egui::IMEPurpose| {
        ui.label(RichText::new(label).size(m.secondary).color(rgb(t.secondary)));
        ui.add_space(m.gap / 2.);
        let eye_id = egui::Id::new(("signin-eye", label));
        let shown = secret && ui.data(|d| d.get_temp::<bool>(eye_id)).unwrap_or(false);
        let (near, far) = (14, if secret { height as i8 } else { 14 });
        let margin = if dir::rtl() { egui::Margin { left: far, right: near, top: 0, bottom: 0 } } else { egui::Margin { left: near, right: far, top: 0, bottom: 0 } };
        let r = ui.add_sized([width, height], crate::metrics::line(text).password(secret && !shown).ime_purpose(purpose).margin(margin));
        if secret {
            let rect = dir::flip(r.rect, egui::Rect::from_center_size(egui::pos2(r.rect.right() - height / 2., r.rect.center().y), egui::vec2(height, height)));
            let (icon, name) = if shown { (icons::Icon::EyeSlash, i18n::t("Hide password")) } else { (icons::Icon::Eye, i18n::t("Show password")) };
            if icons::button_at(ui, rect, eye_id.with("button"), icon, false, m.icon, name, None, true, 1.).clicked() {
                ui.data_mut(|d| d.insert_temp(eye_id, !shown));
            }
        }
        // Right click. On iOS the long press opens the system edit menu on the view (winit), which pastes into every field.
        #[cfg(not(target_os = "ios"))]
        r.context_menu(|ui| {
            if ui.button(i18n::t_ctx("menu", "Paste")).clicked() {
                ui.memory_mut(|mem| mem.request_focus(r.id));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
        });
        if r.gained_focus() {
            if matches!(purpose, egui::IMEPurpose::Url) && text == "https://" {
                let mut state = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(text.chars().count()))));
                state.store(ui.ctx(), r.id);
            }
            r.scroll_to_me(None);
        }
        if r.has_focus() {
            focused = Some(r.rect);
            // A TextEdit that the keyboard pushed out of the scroll area stops asking for the IME: it closes, the area grows back, it opens again.
            if !ui.is_rect_visible(r.rect) { r.scroll_to_me(None); }
        }
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(m.gap * 1.5);
    };
    if let Some(p) = prompt {
        match (p.mode, s.form.vault) {
            (SignInMode::Password, vault) => {
                field(ui, &mut s.homeserver, i18n::t("Server"), false, egui::IMEPurpose::Url);
                field(ui, &mut s.user, i18n::t("Username or Matrix ID"), false, egui::IMEPurpose::Username);
                field(ui, &mut s.password, i18n::t("Password"), true, egui::IMEPurpose::LoginPassword);
                if !vault {
                    field(ui, &mut s.phrase, i18n::t("Local passphrase"), true, egui::IMEPurpose::Password);
                }
            }
            (SignInMode::Unlock, _) => field(ui, &mut s.phrase, i18n::t("Local passphrase"), true, egui::IMEPurpose::Password),
            (SignInMode::UnlockVault, _) => {}
        }
    }
    // The engine gives up on its own only after discovery succeeds: after 20 s of a sign-in (not an unlock) say what the form says for an unreachable server.
    let stalled = !unlock && waited.is_some_and(|w| w > std::time::Duration::from_secs(20));
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
    let ready = !busy && (unlock || matrix_desktop::SignInForm::server_entered(&s.homeserver));
    let r = ui.add_enabled_ui(ready, |ui| ui.add_sized([width, height + 4.], button)).inner;
    if busy {
        // Loading: a spinner at the left edge of the (inert) button.
        let rect = dir::flip(r.rect, egui::Rect::from_center_size(egui::pos2(r.rect.left() + height / 2. + 4., r.rect.center().y), egui::Vec2::splat(m.icon)));
        egui::Spinner::new().color(rgb(t.on_accent)).paint_at(ui, rect);
    } else if ready && (r.clicked() || enter) {
        s.submit();
    }
    // When the keyboard opens or closes, bring the button into view with the focused field, if both fit above the keyboard.
    let (kb_id, kb) = (egui::Id::new("signin_keyboard"), mobile::keyboard());
    let moved = ui.data(|d| d.get_temp::<f32>(kb_id)) != Some(kb);
    ui.data_mut(|d| d.insert_temp(kb_id, kb));
    if moved && let Some(field) = focused {
        let both = field.union(r.rect);
        if both.height() <= ui.clip_rect().height() { ui.scroll_to_rect(both, None); }
    }
    false
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

/// Linux and iOS: the bundled Noto Color Emoji (SIL OFL 1.1, see THIRD_PARTY_NOTICES.md) as the last fallback, so the emoji set is the
/// same on every system and never depends on the app being allowed to read Apple Color Emoji (on iOS the system font provider
/// found nothing on the owner's iPhone). macOS and Windows keep their own emoji font (system_fonts).
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use matrix_desktop::tray::Tray as TrayHandle;
/// Mobile stub: the tray module needs tray-icon, which does not exist on iOS or Android.
#[cfg(any(target_os = "ios", target_os = "android"))]
struct TrayHandle;
#[cfg(any(target_os = "ios", target_os = "android"))]
impl TrayHandle { fn start() -> Option<Self> { None } }

fn register_emoji_font(ctx: &egui::Context) {
    #[cfg(any(target_os = "linux", target_os = "ios", target_os = "android"))]
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
    #[cfg(not(any(target_os = "linux", target_os = "ios", target_os = "android")))]
    let _ = ctx;
}

/// The icon chosen in Settings as the window/Dock icon (Linux Wayland takes it from the installed desktop entry instead).
fn app_icon(choice: matrix_desktop::IconChoice) -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(choice.runtime_png(), image::ImageFormat::Png).ok()?.into_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

/// macOS: winit ignores the window icon, so the Dock icon is set on NSApplication (the menu bar tray keeps its own template glyph).
#[cfg(target_os = "macos")]
fn mac_dock_icon(choice: matrix_desktop::IconChoice) {
    use cocoa::{appkit::NSApp, base::{id, nil}};
    use objc::{class, msg_send, sel, sel_impl};
    let png = choice.runtime_png();
    // SAFETY: called on the main thread (App::new / update) with a live NSApplication; NSData copies the bytes.
    unsafe {
        let data: id = msg_send![class!(NSData), dataWithBytes: png.as_ptr() length: png.len()];
        let image: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![image, initWithData: data];
        if image != nil {
            let _: () = msg_send![NSApp(), setApplicationIconImage: image];
            eprintln!("Gnobbi: Dock icon set to {choice:?}");
        }
    }
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

/// Android entry (NativeActivity): the activity's app handle is the only thing the event loop needs besides the options.
#[cfg(target_os = "android")]
static ANDROID_APP: std::sync::OnceLock<android_activity::AndroidApp> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_tag("gnobbi").with_max_level(log::LevelFilter::Info));
    // No HOME on Android: the XDG fallbacks of app_dirs then land in the app's private dir.
    // Canonical: /data/user/0 is a symlink to /data/data and the engine refuses store paths with a link in them.
    if let Some(dir) = app.internal_data_path().and_then(|d| d.canonicalize().ok()) {
        unsafe { std::env::set_var("HOME", dir) };
    }
    // shortcut: engine logs always on while the port is a spike; drop with the first release build.
    if std::env::var_os("RUST_LOG").is_none() {
        unsafe { std::env::set_var("RUST_LOG", "info") };
    }
    let _ = ANDROID_APP.set(app);
    if let Err(e) = main() {
        log::error!("gnobbi exited: {e}");
    }
}

/// The SDK's message index logs every indexed event (the text) at debug level: never, whatever `RUST_LOG` says.
const NO_INDEX_LOG: &str = "matrix_sdk_search=off";

/// Desktop: room-open timings (no room content) go to `room-open.log` in the data dir, emptied at each start; `RUST_LOG` overrides the filter.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn room_open_log(dir: &std::path::Path) {
    let Ok(file) = std::fs::File::create(dir.join("room-open.log")) else { return };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("matrix_engine::room_sync=info")).add_directive(NO_INDEX_LOG.parse().unwrap());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(file)).try_init();
}

fn main() -> eframe::Result {
    // iOS: stderr goes nowhere, so `log` goes to os_log (Console / `log stream`) and stderr is piped into it line by line.
    // Android is the same: stderr (panics included) goes to logcat through the `log` logger set up in android_main.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        use std::io::BufRead;
        use std::os::fd::FromRawFd;
        #[cfg(target_os = "ios")]
        let _ = oslog::OsLogger::new("app.gnobbi").level_filter(log::LevelFilter::Info).init();
        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
            unsafe { libc::dup2(fds[1], 2); libc::dup2(fds[1], 1); }
            let rd = unsafe { std::fs::File::from_raw_fd(fds[0]) };
            std::thread::spawn(move || std::io::BufReader::new(rd).lines().map_while(Result::ok).for_each(|l| log::info!("{l}")));
        }
        if std::env::var_os("RUST_LOG").is_some() {
            let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(NO_INDEX_LOG.parse().unwrap())).with_writer(std::io::stderr).try_init();
        }
    }
    // Updater (Windows/Linux): the Windows helper mode and `--self-test` must run before anything else.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Some(code) = matrix_desktop::update::helper::dispatch(&args) {
            std::process::exit(code);
        }
        matrix_desktop::update::startup::run_at_start();
    }
    // `gnobbi account …` is a console command: no window, no notifications, no single-instance gate.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("account") {
            std::process::exit(matrix_desktop::account_cli::main(&args[1..]));
        }
    }
    matrix_desktop::notifications::init();
    // A `matrix:` link is not an option: taken out first, validated by the engine, carried out once the rooms are there.
    let (args, raw_link) = matrix_desktop::take_matrix_link(std::env::args().skip(1).collect());
    let link = raw_link.clone().and_then(|raw| matrix_engine::matrix_link::parse_matrix_link(&raw).map_err(|_| eprintln!("Gnobbi: ignored matrix link")).ok());
    #[allow(unused_mut)]
    let mut dirs = matrix_desktop::app_dirs(Platform::CURRENT, |k| std::env::var(k).ok());
    if let Some(d) = &dirs {
        matrix_desktop::crash_log::report_previous(&d.data);
        matrix_desktop::crash_log::install(&d.data);
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        room_open_log(&d.data);
    }
    // iOS: an ambiguous migration must never fall back to a different store.
    #[cfg(all(target_os = "ios", not(target_abi = "sim")))]
    if let Some(d) = &mut dirs
        && d.data != d.config
    {
        match matrix_desktop::app_group::migrate(&d.config, &d.data) {
            Ok(()) => (),
            Err(e @ matrix_desktop::app_group::MigrationError::Ambiguous { .. }) => {
                eprintln!("Gnobbi: {e}");
                std::process::exit(1);
            }
            Err(matrix_desktop::app_group::MigrationError::Io(e)) => {
                eprintln!("Gnobbi: store migration failed: {e}");
                d.data = d.config.clone();
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Some(d) = &dirs {
        matrix_desktop::update::scheduler::start(d.clone(), matrix_desktop::update::shared_state(), Default::default());
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
            let _ = matrix_desktop::AccountCatalog::update(dirs, |c| c.migrate_legacy(dirs));
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
    #[cfg(target_os = "android")]
    {
        options.android_app = ANDROID_APP.get().cloned();
    }
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
            metrics::observe_text_size(&cc.egui_ctx);
        }
        #[cfg(target_os = "ios")]
        pick::purge_temp();
        let app = App::new(dirs.as_ref(), root, trust, link, multi);
        matrix_desktop::i18n::script::use_locale_fonts(&cc.egui_ctx, matrix_desktop::i18n::current());
        Ok(Box::new(app))
    }));
    result
}

#[derive(Debug, PartialEq, Eq)]
enum StartupAction { Wait, Ready, Open(std::path::PathBuf), Exhausted }

fn startup_step(pending: &mut Option<std::collections::VecDeque<std::path::PathBuf>>, opened: bool, failed: bool) -> StartupAction {
    if opened {
        *pending = None;
        return StartupAction::Ready;
    }
    if failed && let Some(queue) = pending {
        if let Some(root) = queue.pop_front() { return StartupAction::Open(root); }
        *pending = None;
        return StartupAction::Exhausted;
    }
    StartupAction::Wait
}

fn startup_roots(default: std::path::PathBuf, mut catalog: Vec<std::path::PathBuf>, saved: Option<&str>) -> Vec<std::path::PathBuf> {
    if catalog.is_empty() { return vec![default]; }
    if let Some(i) = saved.and_then(|s| catalog.iter().position(|p| p.to_str() == Some(s))) {
        let selected = catalog.remove(i);
        catalog.insert(0, selected);
    }
    catalog
}

fn remember_open_account(file: &std::path::Path, root: &std::path::Path, opened: bool) -> std::io::Result<()> {
    if opened { std::fs::write(file, root.to_string_lossy().as_bytes()) } else { Ok(()) }
}

fn restore_failed(status: FormStatus) -> bool {
    use matrix_engine::signin::{SignInMode, SignInPrompt};
    match status {
        FormStatus::Failed(_) => true,
        FormStatus::Prompt(SignInPrompt { mode: SignInMode::Unlock, .. }) => false,
        FormStatus::Prompt(SignInPrompt { mode: SignInMode::UnlockVault, error: None }) => false,
        FormStatus::Prompt(_) => true,
        _ => false,
    }
}

fn account_login_fields(homeserver: Option<&str>, user: Option<&str>) -> (String, String) {
    let user = user.unwrap_or_default().to_owned();
    let server = homeserver.filter(|s| !s.trim().is_empty()).map(str::to_owned).unwrap_or_else(|| server_for_user(&user));
    (server, user)
}

fn server_for_user(user: &str) -> String {
    user.split_once(':').map(|(_, host)| format!("https://{host}")).unwrap_or_default()
}

fn account_row_label<'a>(name: Option<&'a str>, saved: Option<&'a str>, user: Option<&'a str>) -> &'a str {
    [name, saved, user].into_iter().flatten().map(str::trim).find(|s| !s.is_empty()).unwrap_or("?")
}

/// Where the back control of the sign-in form leads. None: no way back (first run, or the account is already in).
#[derive(Debug, PartialEq)]
enum SigninBack {
    /// «Accedi di nuovo»: the recovery screen.
    Recovery,
    /// Add account: the account that was open.
    Previous,
}

fn signin_back(again: bool, previous_running: bool, authenticated: bool) -> Option<SigninBack> {
    if again { Some(SigninBack::Recovery) } else { (previous_running && !authenticated).then_some(SigninBack::Previous) }
}

/// Swaps the previous account back in as the active one and hands out the cancelled new session with its folder.
fn cancel_add<S>(current: &mut Option<S>, root: &mut std::path::PathBuf, recorded: &mut bool, parked: &mut Vec<Parked<S>>, previous: &std::path::Path) -> Option<(S, std::path::PathBuf)> {
    let i = parked.iter().position(|p| p.root == previous)?;
    let back = parked.remove(i);
    let new = current.replace(back.session)?;
    let new_root = std::mem::replace(root, back.root);
    *recorded = back.recorded;
    Some((new, new_root))
}

/// Deletes a profile folder that never got an account: only under `accounts/` and never one the catalog knows.
fn discard_new_profile(dirs: &AppDirs, dir: &std::path::Path) {
    if dir.starts_with(dirs.data.join("accounts")) && matrix_desktop::AccountCatalog::load(dirs).id_for_root(dirs, dir).is_none() {
        accounts::wipe(dir, None);
    }
}

/// The spaces rail: the shown account on top (a click lists the others), then the spaces.
fn rail_panel(ui: &mut egui::Ui, session: &mut Session, rail_state: &mut rail::State, account: u64, totals: (u64, u64), others: &[(std::path::PathBuf, String, String, avatar::Avatar, Option<u64>)], (active_label, active_id): (&str, &str), t: &DesignTokens) -> Option<std::path::PathBuf> {
    let mut pick_account = None;
    dir::panel_start("rail").exact_size(rail::WIDTH).resizable(false).show_separator_line(false).frame(egui::Frame::NONE.fill(chat_background(t))).show(ui, |ui| {
        ui.add_space(8.);
        // Five dots per row fit the rail; more accounts wrap onto further rows, each centred.
        let dots = others.len().div_ceil(5) as f32 * 10.;
        let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(rail::WIDTH, 52. + dots), egui::Sense::click());
        let resp = crate::metrics::clickable(resp);
        let name = active_label;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        if resp.hovered() {
            ui.painter().rect_filled(rect.shrink2(egui::Vec2::new(6., 2.)), 10., rgb(t.input));
        }
        let c = rect.center_top() + egui::Vec2::new(0., 26.);
        let initial: String = name.trim_start_matches('@').chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned());
        avatar::paint(ui, c, 36., rgb(AVATAR), &session.own_avatar(), &initial, 16., rgb(t.on_accent));
        for (i, (_, _, _, _, n)) in others.iter().enumerate() {
            let in_row = (others.len() - i / 5 * 5).min(5);
            let d = egui::Pos2::new(c.x + ((i % 5) as f32 - (in_row - 1) as f32 / 2.) * 10., rect.top() + 55. + (i / 5) as f32 * 10.);
            if n.is_some_and(|n| n > 0) {
                ui.painter().circle_filled(d, 3., rgb(t.accent));
            } else {
                ui.painter().circle_stroke(d, 2.5, egui::Stroke::new(1., rgb(t.secondary)));
            }
        }
        // One account: nothing to switch to, so no menu.
        if !others.is_empty() {
            // Test hook: GNOBBI_EGUI_VIEW=account-menu shows the menu without a click.
            if matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").as_deref() == Ok("account-menu") {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
            }
            egui::Popup::menu(&resp).align(if dir::rtl() { egui::RectAlign::LEFT_START } else { egui::RectAlign::RIGHT_START }).width(280.).frame(egui::Frame::popup(ui.style()).inner_margin(8).corner_radius(10).stroke(egui::Stroke::NONE).fill(rgb(t.surface))).show(|ui| crate::dir::popup(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                if accounts::switch_row(ui, name, active_id, &session.own_avatar(), true, 0, t).clicked() {
                    rail_state.one_account = true;
                    ui.close();
                }
                for (root, other, id, pic, unread) in others {
                    if accounts::switch_row(ui, other, id, pic, false, unread.unwrap_or(0), t).clicked() {
                        pick_account = Some(root.clone());
                        ui.close();
                    }
                }
            }));
        }
        resp.on_hover_text(name);
        let placed = session.space_placed.take();
        rail::show(ui, &mut session.sidebar, &session.avatars, rail_state, account, placed, totals, t);
    });
    pick_account
}

/// Narrow window: the list takes the whole width left of the rail; a tap on a row, the open one too, goes to the chat.
fn narrow_list(ui: &mut egui::Ui, session: &mut Session, list_open: &mut bool, list_fill: egui::Color32, unified: Option<list::Unified>, t: &DesignTokens) {
        dir::panel_start("list-narrow").exact_size(ui.available_width()).resizable(false).show_separator_line(false).frame(bar(t).fill(list_fill)).show(ui, |ui| dir::scope(ui, |ui| {
            let mut new_chat = false;
            if list::rooms(ui, session, &mut new_chat, t, unified) {
                *list_open = false;
            }
            session.new_open |= new_chat;
        }));
}

/// Wide window: the resizable list, its width remembered once the drag is over.
fn wide_list(ui: &mut egui::Ui, session: &mut Session, prefs: &mut Preferences, dirs: Option<&AppDirs>, list_fill: egui::Color32, unified: Option<list::Unified>, t: &DesignTokens) {
        let max = list_max_width(ui.ctx().content_rect().width());
        let resize_id = ui.scope_id().with_salt(egui::IdSalt::new("list")).with("__resize");
        let hovered = ui.style().visuals.widgets.hovered.fg_stroke;
        let active = ui.style().visuals.widgets.active.fg_stroke;
        let highlight = Stroke::NONE;
        ui.style_mut().visuals.widgets.hovered.fg_stroke = highlight;
        ui.style_mut().visuals.widgets.active.fg_stroke = highlight;
        let response = dir::panel_start("list")
            .default_size(prefs.list_width.map_or(t.sidebar, |w| w as f32))
            .size_range(240. ..=max).resizable(true).show_separator_line(false)
            .frame(bar(t).fill(list_fill)).show(ui, |ui| {
                ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
                ui.style_mut().visuals.widgets.active.fg_stroke = active;
                let mut new_chat = false;
                dir::scope(ui, |ui| list::rooms(ui, session, &mut new_chat, t, unified));
                session.new_open |= new_chat;
            });
        ui.style_mut().visuals.widgets.hovered.fg_stroke = hovered;
        ui.style_mut().visuals.widgets.active.fg_stroke = active;
        if let Some(handle) = ui.read_response(resize_id)
            && handle.drag_stopped()
        {
            let width = handle.interact_pointer_pos().map_or(response.response.rect.width(), |p| if dir::rtl() { response.response.rect.right() - p.x } else { p.x - response.response.rect.left() }).clamp(240., max).round() as u32;
            prefs.list_width = Some(width);
            if let Some(dirs) = dirs
                && let Err(error) = Preferences::update(dirs, |p| p.list_width = Some(width))
            {
                eprintln!("Gnobbi: list_width {:?}", error.kind());
            }
        }
}

fn account_switch_effect(pick: Option<std::path::PathBuf>, active: &std::path::Path, narrow: bool) -> (Option<std::path::PathBuf>, bool) {
    (pick.filter(|path| path != active), narrow)
}

/// A public room's row: the name, and under it the join's spinner and one-line state. The state wraps on its own line, so a long name cannot squeeze it to nothing.
fn public_room_row(ui: &mut egui::Ui, selected: bool, name: String, join: Option<(bool, Option<&'static str>, bool)>) -> egui::Response {
    let r = ui.selectable_label(selected, name);
    if let Some((running, line, failed)) = join && (running || line.is_some()) {
        ui.horizontal(|ui| {
            if running { ui.spinner(); }
            if let Some(line) = line {
                ui.add(egui::Label::new(egui::RichText::new(line).color(if failed { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() })).wrap());
            }
        });
    }
    r
}

/// The «Account unavailable» column: never wider than the window (a phone is under 400 pt), symmetric margins, wrapping error.
/// Returns (retry, sign in again, sign out) clicks; `middle` draws the account switcher between Retry and Sign in again.
fn recovery_screen(ui: &mut egui::Ui, t: &DesignTokens, who: Option<(&str, Option<&str>)>, line: &str, middle: impl FnOnce(&mut egui::Ui, egui::Vec2)) -> (bool, bool, bool) {
    let (mut retry, mut again, mut out) = (false, false, false);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline)).inner_margin(16.)).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(400.));
            ui.heading(i18n::t("Account unavailable"));
            if let Some((label, id)) = who {
                ui.label(label);
                if let Some(id) = id { ui.label(id); }
            }
            ui.add(egui::Label::new(RichText::new(line).color(rgb(t.error))).wrap());
            let size = egui::vec2(ui.available_width(), metrics::button() + 4.);
            retry = ui.add_sized(size, egui::Button::new(i18n::t("Retry"))).clicked();
            middle(ui, size);
            again = ui.add_enabled(who.is_some(), egui::Button::new(i18n::t("Sign in again")).min_size(size)).clicked();
            out = ui.add_sized(size, egui::Button::new(i18n::t("Sign out of this account"))).clicked();
        });
    });
    (retry, again, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rectangles (text, buttons, frames) the «Account unavailable» screen paints at `width`, except the full-window background.
    fn recovery_rects(width: f32, who: Option<(&str, Option<&str>)>, line: &str) -> Vec<egui::Rect> {
        let ctx = crate::icons::test_context();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.));
        let mut rects = Vec::new();
        for _ in 0..3 { // a few passes: egui sizes text and areas from the previous pass
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| { recovery_screen(ui, &matrix_desktop::DARK, who, line, |_, _| {}); });
            out.textures_delta.clear();
            rects = out.shapes.iter().map(|s| s.shape.visual_bounding_rect()).filter(|r| r.is_positive() && r.width() < width).collect();
        }
        rects
    }

    /// Public-room row with a 98-character name and a failed join: the state keeps a readable width (at most two lines) and nothing crosses the right edge.
    #[test]
    fn public_room_row_keeps_its_state_readable_after_a_long_name() {
        let ctx = crate::icons::test_context();
        for width in [291., 436.] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.));
            let mut rects = Vec::new();
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| { public_room_row(ui, false, "n".repeat(98), Some((false, Some("Could not join this room"), true))); });
                out.textures_delta.clear();
                rects = out.shapes.iter().map(|s| s.shape.visual_bounding_rect()).filter(|r| r.is_positive()).collect();
            }
            let right = rects.iter().fold(f32::MIN, |r, x| r.max(x.max.x));
            let tall = rects.iter().map(|r| r.height()).fold(0., f32::max);
            assert!(right <= width + 1., "{width}: something reaches x={right}");
            assert!(tall <= 2. * 20. + 8., "{width}: a shape is {tall} pt tall");
        }
    }

    const LONG_ERROR: &str = "Impossibile aprire questo account perche il profilo locale non e disponibile su questo telefono e il portachiavi ha rifiutato la richiesta, riprova piu tardi o accedi di nuovo con la tua password";

    /// Phone widths: nothing the screen paints crosses the right edge, and the left and right margins are the same.
    #[test]
    fn account_unavailable_stays_inside_a_phone_with_a_long_error() {
        for width in [320., 436.] {
            for line in [LONG_ERROR, &"x".repeat(200), "/private/var/mobile/Containers/Data/Application/0000-1111-2222-3333/Library/Application Support/Gnobbi/profile"] {
                let rects = recovery_rects(width, Some(("Account di prova con un nome molto molto lungo", Some("@un.utente.con.un.id.lunghissimo:server.example.org"))), line);
                assert!(rects.len() > 6, "{width}: only {} shapes", rects.len());
                let (left, right) = rects.iter().fold((f32::MAX, f32::MIN), |(l, r), x| (l.min(x.min.x), r.max(x.max.x)));
                assert!(right <= width - 16. + 1., "{width}: something reaches x={right} (limit {})", width - 16.);
                assert!(left >= 16. - 1., "{width}: something starts at x={left}");
                // Above 432 the 400 pt cap leaves the extra room on the right (436: 16 left, 20 right).
                assert!(width > 432. || (left - (width - right)).abs() <= 1., "{width}: margins {left} left, {} right", width - right);
            }
        }
    }

    /// Wider windows: the column keeps its 400 pt maximum and stays inside the window.
    #[test]
    fn account_unavailable_is_capped_at_400_on_a_wide_window() {
        let rects = recovery_rects(600., None, LONG_ERROR);
        let right = rects.iter().fold(f32::MIN, |r, x| r.max(x.max.x));
        assert!((right - 416.).abs() <= 0.5, "right edge at {right}, expected 16 + 400");
    }

    /// Narrower than any phone (a stretched-out Mac window): still no crossing the edge.
    #[test]
    fn account_unavailable_fits_a_very_narrow_window() {
        let rects = recovery_rects(240., None, LONG_ERROR);
        let right = rects.iter().fold(f32::MIN, |r, x| r.max(x.max.x));
        assert!(right <= 240. - 16. + 1., "right edge at {right}");
    }

    #[test]
    fn keep_in_view_moves_only_when_the_field_leaves_the_view() {
        assert_eq!(keep_in_view(0., 100., 150., 200.), 0.); // inside: untouched
        assert_eq!(keep_in_view(0., 300., 350., 200.), 150.); // below the keyboard-shrunk view: field bottom on the view bottom
        assert_eq!(keep_in_view(250., 100., 150., 200.), 100.); // above
        assert_eq!(keep_in_view(0., 40., 300., 200.), 40.); // taller than the view: show its top
    }

    #[test]
    fn signin_back_only_with_a_way_to_go() {
        assert_eq!(signin_back(false, true, false), Some(SigninBack::Previous));
        assert_eq!(signin_back(false, false, false), None); // first run / signed out of all
        assert_eq!(signin_back(false, true, true), None); // already signed in: recovery step has its own skip
        assert_eq!(signin_back(true, true, false), Some(SigninBack::Recovery));
        assert_eq!(signin_back(true, false, false), Some(SigninBack::Recovery));
    }

    #[test]
    fn cancelling_add_restores_the_previous_account_and_removes_only_the_new_profile() {
        let root = std::env::temp_dir().join(format!("gnobbi-cancel-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs { config: root.join("c"), data: root.join("d"), cache: root.join("k") };
        let (old, new) = (dirs.data.join("accounts/1"), dirs.data.join("accounts/2"));
        for d in [&old, &new] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("file"), "x").unwrap();
        }
        // The new profile is on screen, the old one parked (as `open_account` leaves them).
        let (mut current, mut at, mut recorded) = (Some("new"), new.clone(), false);
        let mut parked = vec![Parked { root: old.clone(), session: "old", recorded: true }];
        assert!(cancel_add(&mut current, &mut at, &mut recorded, &mut parked, &root.join("gone")).is_none());
        let (gone, gone_root) = cancel_add(&mut current, &mut at, &mut recorded, &mut parked, &old).unwrap();
        assert_eq!((current, gone, &at, recorded, parked.len()), (Some("old"), "new", &old, true, 0));
        discard_new_profile(&dirs, &gone_root);
        assert!(!new.exists() && old.join("file").exists());
        discard_new_profile(&dirs, &root); // outside accounts/: untouched
        assert!(root.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_recovery_fallback_waits_for_open_and_stops_at_first_success() {
        let mut pending = Some(std::collections::VecDeque::from(["/data/a".into(), "/data/c".into()]));
        assert_eq!(startup_step(&mut pending, false, false), StartupAction::Wait);
        assert_eq!(pending.as_ref().unwrap().len(), 2);
        assert_eq!(startup_step(&mut pending, false, true), StartupAction::Open("/data/a".into()));
        assert_eq!(startup_step(&mut pending, false, false), StartupAction::Wait);
        assert_eq!(startup_step(&mut pending, true, false), StartupAction::Ready);
        assert!(pending.is_none());
    }

    #[test]
    fn account_recovery_exhausts_catalog_once_then_leaves_recovery_actions() {
        let mut pending = Some(std::collections::VecDeque::from(["/data/other".into()]));
        assert_eq!(startup_step(&mut pending, false, true), StartupAction::Open("/data/other".into()));
        assert_eq!(startup_step(&mut pending, false, true), StartupAction::Exhausted);
        assert!(pending.is_none());
        assert_eq!(startup_step(&mut pending, false, true), StartupAction::Wait);
    }

    #[test]
    fn account_recovery_translations_cover_english_and_italian() {
        for (source, italian) in [
            ("Account unavailable", "Account non disponibile"),
            ("Could not open this account.", "Impossibile aprire questo account."),
            ("Could not open {account}. Switched to another account.", "Impossibile aprire {account}. Aperto un altro account."),
            ("Switch account", "Cambia account"),
            ("Sign in again", "Accedi di nuovo"),
            ("Retry", "Riprova"),
            ("Sign out of this account", "Esci da questo account"),
        ] {
            assert_eq!(i18n::tr("en", "", source), source);
            assert_eq!(i18n::tr("it", "", source), italian);
        }
    }

    #[test]
    fn account_recovery_prefills_catalog_identity_and_server() {
        let user = Some("@test:example.org:8448");
        assert_eq!(account_login_fields(Some("https://matrix.example.org"), user), ("https://matrix.example.org".into(), "@test:example.org:8448".into()));
        assert_eq!(account_login_fields(None, user).0, "https://example.org:8448");
        assert_eq!(account_login_fields(Some(" "), user).0, "https://example.org:8448");
        assert_eq!(account_login_fields(None, None), (String::new(), String::new()));
    }

    #[test]
    fn account_recovery_startup_order_prefers_saved_then_catalog() {
        let roots: Vec<std::path::PathBuf> = vec!["/data/a".into(), "/data/b".into(), "/data/c".into()];
        assert_eq!(startup_roots("/data/profile".into(), roots.clone(), Some("/data/b")), vec![std::path::PathBuf::from("/data/b"), "/data/a".into(), "/data/c".into()]);
        assert_eq!(startup_roots("/data/profile".into(), roots.clone(), Some("/gone")), roots);
        assert_eq!(startup_roots("/data/profile".into(), vec![], None), vec![std::path::PathBuf::from("/data/profile")]);
    }

    #[test]
    fn account_recovery_last_account_changes_only_after_open() {
        let dir = std::env::temp_dir().join(format!("gnobbi-last-account-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(LAST_ACCOUNT);
        std::fs::write(&file, "previous").unwrap();
        remember_open_account(&file, std::path::Path::new("failed"), false).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "previous");
        remember_open_account(&file, std::path::Path::new("opened"), true).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "opened");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn account_recovery_classifies_saved_profile_prompts() {
        use matrix_engine::signin::{SignInMode, SignInPrompt, SignInError};
        let prompt = |mode, error| FormStatus::Prompt(SignInPrompt { mode, error });
        assert!(!restore_failed(FormStatus::Opening));
        assert!(!restore_failed(FormStatus::Busy));
        assert!(!restore_failed(FormStatus::Authenticated(matrix_domain::AccountId::new(1))));
        assert!(restore_failed(prompt(SignInMode::UnlockVault, Some(SignInError::Storage))));
        assert!(restore_failed(prompt(SignInMode::Password, None)));
        assert!(!restore_failed(prompt(SignInMode::Unlock, None)));
        assert!(!restore_failed(prompt(SignInMode::Unlock, Some(SignInError::InvalidSecret))));
        assert!(!restore_failed(prompt(SignInMode::UnlockVault, None)));
        assert!(restore_failed(FormStatus::Failed(SignInError::NoSession)));
    }

    #[test]
    fn account_row_label_falls_back_without_blank_names() {
        assert_eq!(account_row_label(Some("Marco"), Some("Saved"), Some("@m:s")), "Marco");
        assert_eq!(account_row_label(Some(""), Some("Saved"), Some("@m:s")), "Saved");
        assert_eq!(account_row_label(Some("  "), Some("  "), Some("@m:s")), "@m:s");
        assert_eq!(account_row_label(None, None, None), "?");
    }

    #[test]
    fn account_switch_effect_preserves_narrow_picks() {
        let active = std::path::Path::new("/active");
        let other = std::path::PathBuf::from("/other");
        assert_eq!(account_switch_effect(Some(other.clone()), active, true), (Some(other.clone()), true));
        assert_eq!(account_switch_effect(Some(other.clone()), active, false), (Some(other), false));
        assert_eq!(account_switch_effect(Some(active.into()), active, true), (None, true));
        assert_eq!(account_switch_effect(None, active, false), (None, false));
    }

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
