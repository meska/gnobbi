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
mod composer;
mod devices;
mod info;
mod list;
mod newchat;
mod poll;
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

struct App {
    prefs: Preferences,
    dirs: Option<AppDirs>,
    settings_open: bool,
    accounts_open: bool,
    /// Profile folder this window runs on, and whether its login is already in the catalog.
    root: std::path::PathBuf,
    recorded: bool,
    tray: Option<matrix_desktop::tray::Tray>,
    quitting: bool,
    window_visible: bool,
    new_form: newchat::Form,
    device_fields: devices::Fields,
    poll_form: poll::Form,
    theme: Theme,
    tokens: DesignTokens,
    /// What `visuals` was last built for: rebuilt only when it changes.
    applied: Option<(Theme, u32)>,
    session: Option<Session>,
    /// Why the backend could not start, shown instead of the form.
    start_error: Option<String>,
}

impl App {
    fn new(dirs: Option<&AppDirs>, root: std::path::PathBuf) -> App {
        let prefs = dirs.map(Preferences::load).unwrap_or_else(Preferences::default);
        let theme = Theme::Classic;
        let (session, start_error) = match dirs.map(|d| Session::start(d, root.clone())) {
            Some(Ok(s)) => (Some(s), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, Some("Cartella dati non disponibile".to_owned())),
        };
        App { prefs, dirs: dirs.cloned(), settings_open: false, accounts_open: false, root: root.clone(), recorded: false, tray: matrix_desktop::tray::Tray::start(), quitting: false, window_visible: true, new_form: Default::default(), device_fields: Default::default(), poll_form: Default::default(), theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, session, start_error }
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
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(session) = &mut self.session {
            if let Some(path) = dropped.first() {
                // With the devices window open a dropped picture is the profile photo, otherwise an attachment.
                match &mut session.verification {
                    Some(v) if session.devices_open => {
                        v.save_profile(None, matrix_engine::verification::ProfileAvatar::Set(path.clone()));
                    }
                    _ => session.attach(path),
                }
            }
            session.poll(&ctx, self.prefs.notifications, self.prefs.message_preview);
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
        let t = self.tokens;
        if self.session.as_ref().is_some_and(|s| s.account.is_some()) {
            self.shell(ui, &t);
        } else {
            self.sign_in(ui, &t);
        }
    }
}

impl App {
    /// Tray clicks, the menu and icon state, and what closing the window does (hide when the tray can be reached).
    fn tray(&mut self, ctx: &egui::Context) {
        use matrix_desktop::tray_menu::{CloseAction, TrayAction, close_action, tray_menu};
        let show = |ctx: &egui::Context| {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        };
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
        if let Some(session) = &mut self.session {
            egui::Panel::left("rail").exact_size(64.).resizable(false).frame(bar(t)).show(ui, |ui| {
                list::rail(ui, &mut session.sidebar, t);
            });
            egui::Panel::left("list").exact_size(t.sidebar).resizable(false).frame(bar(t)).show(ui, |ui| {
                let mut new_chat = false;
                list::rooms(ui, &mut session.sidebar, &mut session.query, &mut new_chat, t);
                session.new_open |= new_chat;
            });
        }
        let has_room = self.session.as_ref().is_some_and(|s| s.messages.is_some());
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        egui::Panel::top("header").exact_size(t.header).frame(bar(t)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(16.);
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
            let mut opened = None;
            egui::Panel::right("info").exact_size(300.).resizable(false).frame(bar(t)).show(ui, |ui| opened = info::show(ui, room, members, t).open_profile);
            if let (Some(user), Some(s)) = (opened, &mut self.session) {
                s.open_profile(user);
            }
        }
        if let Some(s) = &mut self.session
            && s.info_open
            && let Some((id, profile)) = s.profile.clone()
        {
            let (mut close, mut flip) = (false, None);
            egui::Panel::right("member").exact_size(300.).resizable(false).frame(bar(t)).show(ui, |ui| flip = info::card(ui, &id, profile.as_ref(), &mut close, t));
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
                let name = s.attachment.as_ref().map(|(n, _)| n.clone());
                if let Some(notice) = &s.notice {
                    ui.label(notice);
                }
                to_send = composer::show(ui, &mut s.draft, mode, name.as_deref(), &mut cancel, &mut drop_file, &mut open_poll);
                if drop_file {
                    s.attachment = None;
                }
                s.poll_open |= open_poll;
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
        }
        if let Some(s) = &mut self.session
            && s.poll_open
        {
            poll::show(ui.ctx(), s, &mut self.poll_form, t);
        }
        if self.accounts_open
            && let Some(dirs) = &self.dirs
        {
            match accounts::show(ui.ctx(), &mut self.accounts_open, dirs, &self.root, t) {
                Some(accounts::Launch::Open(dir)) => accounts::launch(&dir),
                Some(accounts::Launch::Add) => {
                    let next = matrix_desktop::AccountCatalog::load(dirs).next_root(dirs);
                    accounts::launch(&next);
                }
                None => {}
            }
        }
        if self.settings_open
            && let Some(new) = settings::show(ui.ctx(), &mut self.settings_open, &self.prefs, self.dirs.as_ref(), t)
        {
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
                let mut acts = timeline::Acts::default();
                let (first, menu, save, thread) = timeline::show(ui, messages.events(), messages, &mut s.heights, &s.pictures, &mut acts, s.sidebar.selected().map_or(3, |r| r.info.joined_members), t);
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
                            Some(session) => sign_in_form(ui, session),
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
fn sign_in_form(ui: &mut egui::Ui, s: &mut Session) {
    use matrix_engine::signin::SignInMode;
    let status = s.status();
    let prompt = match status {
        FormStatus::Prompt(p) => Some(p),
        _ => None,
    };
    let mode = prompt.map_or(SignInMode::UnlockVault, |p| p.mode);
    let unlock = mode != SignInMode::Password;
    ui.heading(if unlock { "Sblocca Gnobbi" } else { "Accedi a Gnobbi" });
    ui.add_space(12.);
    let mut enter = false;
    let mut field = |ui: &mut egui::Ui, text: &mut String, hint: &str, secret: bool| {
        let r = ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(text).hint_text(hint).password(secret));
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
    if let Some(message) = s.form.status().error_message() {
        ui.colored_label(ui.visuals().error_fg_color, message);
        ui.add_space(8.);
    }
    if prompt.is_some() {
        let label = if unlock { "Sblocca" } else { "Accedi" };
        if ui.add_sized([ui.available_width(), 36.], egui::Button::new(label)).clicked() || enter {
            s.submit();
        }
    } else {
        ui.spinner();
    }
}

/// First instance listens for later launches; if one is already running (the GPUI app included)
/// the launch is handed to it and this process exits. Socket trouble only costs this behaviour.
#[cfg(target_os = "linux")]
fn single_instance_gate() {
    use matrix_desktop::single_instance::{Claim, claim, default_socket_path, outgoing, serve};
    let Some(path) = default_socket_path() else {
        return eprintln!("Gnobbi: single instance off (no private state directory)");
    };
    match claim(&path, &outgoing(None)) {
        Ok(Claim::First(listener)) => serve(listener),
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

#[cfg(windows)]
fn single_instance_gate() {
    use matrix_desktop::windows_link::{Claim, claim};
    match claim("") {
        Ok(Claim::First) => {}
        Ok(Claim::Delivered) => std::process::exit(0),
        Err(error) => eprintln!("Gnobbi: single instance off ({error})"),
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn single_instance_gate() {}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs = matrix_desktop::app_dirs(Platform::CURRENT, |k| std::env::var(k).ok());
    let default_root = dirs.as_ref().map(|d| d.data.join("profile"));
    // `--signin <folder>` runs another account's profile; the default profile is the first install's.
    let root = match matrix_desktop::SignInOptions::parse(&args) {
        Ok(Some(o)) => o.root,
        Ok(None) => default_root.clone().unwrap_or_default(),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // Another profile is its own window: only the default one hands launches over to a running instance.
    if Some(&root) == default_root.as_ref() {
        single_instance_gate();
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
            .with_app_id(APP_ID),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(move |_| Ok(Box::new(App::new(dirs.as_ref(), root)))))
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
