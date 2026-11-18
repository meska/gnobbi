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

mod composer;
mod list;
mod session;
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
    theme: Theme,
    tokens: DesignTokens,
    /// What `visuals` was last built for: rebuilt only when it changes.
    applied: Option<(Theme, u32)>,
    session: Option<Session>,
    /// Why the backend could not start, shown instead of the form.
    start_error: Option<String>,
}

impl App {
    fn new(dirs: Option<&AppDirs>) -> App {
        let prefs = dirs.map(Preferences::load).unwrap_or_else(Preferences::default);
        let theme = Theme::Classic;
        let (session, start_error) = match dirs.map(Session::start) {
            Some(Ok(s)) => (Some(s), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, Some("Cartella dati non disponibile".to_owned())),
        };
        App { prefs, theme, tokens: theme.tokens_with(matrix_desktop::current_accent()), applied: None, session, start_error }
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
        if let Some(session) = &mut self.session {
            session.poll();
            // The backend answers off-thread: wake up soon while waiting for it.
            if session.account.is_none() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let t = self.tokens;
        if self.session.as_ref().is_some_and(|s| s.account.is_some()) {
            self.shell(ui, &t);
        } else {
            self.sign_in(ui, &t);
        }
    }
}

impl App {
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
                list::rooms(ui, &mut session.sidebar, &mut session.query, t);
            });
        }
        let title = self.session.as_ref().and_then(|s| s.sidebar.selected().map(|r| r.name.clone()));
        egui::Panel::top("header").exact_size(t.header).frame(bar(t)).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(16.);
                ui.label(title.as_deref().unwrap_or(state));
            });
        });
        let mut to_send = None;
        egui::Panel::bottom("composer").min_size(t.bottom_bar).frame(bar(t)).show(ui, |ui| {
            if let Some(s) = &mut self.session
                && s.messages.is_some()
            {
                let mode = s.target.as_ref().map_or(composer::Mode::Send, |(_, m)| *m);
                let mut cancel = false;
                to_send = composer::show(ui, &mut s.draft, mode, &mut cancel);
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
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(rgb(t.timeline))).show(ui, |ui| {
            if let Some(s) = &mut self.session
                && let Some(messages) = &s.messages
            {
                let (first, menu) = timeline::show(ui, messages.events(), messages, &mut s.heights, 3, t);
                s.first_visible = first;
                if let Some((id, pos)) = menu {
                    s.open_menu(id, pos);
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
    single_instance_gate();
    let dirs = matrix_desktop::app_dirs(Platform::CURRENT, |k| std::env::var(k).ok());
    let first = Theme::Classic.tokens();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([first.width, first.height])
            .with_min_inner_size([360., 420.])
            .with_title(TITLE)
            .with_app_id(APP_ID),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(move |_| Ok(Box::new(App::new(dirs.as_ref())))))
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
