//! «Account» drawer: the card of the account on screen (name and picture, editable where the server allows)
//! and the other accounts of the catalog, all running in this app; i18n::t("Open") brings one on screen.
use matrix_desktop::i18n;
use crate::list::rgb;
use crate::avatar_editor::Editor;
use crate::session::Session;
use eframe::egui::{self, RichText};
use matrix_desktop::{AccountCatalog, AppDirs, DesignTokens};
use matrix_engine::verification::ProfileAvatar;
use std::path::PathBuf;

/// The picture the next «Salva» writes.
#[derive(Default)]
enum Photo {
    #[default]
    Keep,
    Remove,
    Set(PathBuf, egui::TextureHandle),
}

/// What the user is editing on the card; nothing is written before «Salva».
#[derive(Default)]
pub struct Form {
    name: String,
    /// The profile the name field was filled from: a new one (saved, other account) refills it.
    seeded: Option<(String, Option<String>)>,
    photo: Photo,
    /// The retouch window; kept (closed) after «Applica» so the same picture can be adjusted again.
    editor: Option<Editor>,
    /// A «Salva» is on its way: a good answer drops the chosen picture (the server's one is shown again).
    sent: bool,
    /// Test hook state (see `card`).
    hooked: bool,
    hook_save: bool,
}

impl Form {
    /// A file from the picker or dropped on the window opens the retouch window; `profile` is the account's private folder.
    pub fn pick(&mut self, path: PathBuf, profile: &std::path::Path, ctx: &egui::Context) {
        self.editor = Some(Editor::from_file(path, draft(profile), ctx));
    }

    /// Test hook, debug builds only: the retouch window alone on GNOBBI_EGUI_PHOTO, with no account (GNOBBI_EGUI_VIEW=avatar-editor).
    #[cfg(debug_assertions)]
    pub fn demo(&mut self, ctx: &egui::Context, t: &DesignTokens) {
        if let (None, Ok(path)) = (&self.editor, matrix_desktop::hooks::var("GNOBBI_EGUI_PHOTO")) {
            self.pick(path.into(), &std::env::temp_dir(), ctx);
        }
        if let Some(e) = &mut self.editor {
            e.show(ctx, t);
        }
    }

    /// The retouch window is on screen (Esc belongs to it).
    pub fn editing(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.open)
    }
}

/// Where «Applica» writes the picture «Salva» uploads: one file in the account's private folder, gone with the account.
fn draft(profile: &std::path::Path) -> PathBuf {
    profile.join("avatar-draft.png")
}

/// The card of the account on screen. False until the profile has arrived (the plain row is drawn instead).
fn card(ui: &mut egui::Ui, s: &mut Session, f: &mut Form, profile: &std::path::Path, logout: &mut bool, t: &DesignTokens) -> bool {
    let Some(v) = &mut s.verification else { return false };
    let Some(shown) = v.profile().cloned() else { return false };
    // Until the server's answer arrives nothing is editable, so no control appears and then vanishes.
    let (can_name, can_photo) = s.profile_caps.unwrap_or((false, false));
    if f.seeded.as_ref() != Some(&shown) {
        f.name = shown.1.clone().unwrap_or_default();
        f.seeded = Some(shown.clone());
    }
    // Test hook: GNOBBI_EGUI_NAME=<text> and GNOBBI_EGUI_PHOTO=<file>|remove fill the card once; GNOBBI_EGUI_CONFIRM presses «Salva».
    if !f.hooked && s.profile_caps.is_some() {
        f.hooked = true;
        if let Ok(name) = matrix_desktop::hooks::var("GNOBBI_EGUI_NAME") {
            f.name = name;
        }
        match matrix_desktop::hooks::var("GNOBBI_EGUI_PHOTO") {
            Ok(p) if p == "remove" => f.photo = Photo::Remove,
            Ok(p) => f.pick(p.into(), profile, ui.ctx()),
            Err(_) => {}
        }
        f.hook_save = matrix_desktop::hooks::var_os("GNOBBI_EGUI_CONFIRM").is_some();
        // With the confirm hook the retouch window applies the picture as it is.
        if let Some(e) = f.editor.as_mut().filter(|_| f.hook_save) {
            e.auto_apply = true;
        }
    }
    if let Some(e) = &mut f.editor {
        if let Some((path, preview)) = e.show(ui.ctx(), t) {
            f.photo = Photo::Set(path, preview);
        }
        if e.unused() {
            f.editor = None;
        }
    }
    if f.sent && !s.profile_busy {
        f.sent = false;
        if s.profile_saved == Some(Ok(())) {
            f.photo = Photo::Keep;
            f.editor = None;
        }
    }
    let (id, name) = shown;
    let current = v.profile_avatar().map(str::to_owned);
    let own = crate::avatar::lookup(&s.avatars, current.as_deref());
    let busy = s.profile_busy;
    let name_changed = can_name && f.name.trim() != name.as_deref().unwrap_or("") && !f.name.trim().is_empty();
    let dirty = name_changed || !matches!(f.photo, Photo::Keep);
    let (mut save, mut choose, mut retouch) = (false, false, false);
    ui.add_enabled_ui(!busy, |ui| {
        // Avatar and the name/id block share one axis: the block has a known height, so the row centres both.
        let block = crate::metrics::button() + 2. + ui.painter().layout_no_wrap(id.clone(), egui::FontId::proportional(12.), egui::Color32::WHITE).size().y;
        let main = |ui: &mut egui::Ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(64.), if can_photo { egui::Sense::click() } else { egui::Sense::hover() });
            let resp = crate::metrics::clickable(resp);
            let shown_name = name.as_deref().unwrap_or(&id);
            let picture = match &f.photo {
                Photo::Set(_, tex) => crate::avatar::Avatar::Ready(tex.clone()),
                Photo::Keep => own.clone(),
                Photo::Remove => crate::avatar::Avatar::Missing,
            };
            crate::avatar::paint(ui, rect.center(), 64., rgb(crate::AVATAR), &picture, &matrix_desktop::initials(shown_name), 24., rgb(t.on_accent));
            if can_photo {
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, matrix_desktop::i18n::t("Edit photo")));
                if resp.has_focus() || resp.hovered() {
                    ui.painter().circle_stroke(rect.center(), 32., egui::Stroke::new(2., rgb(t.accent)));
                }
                retouch |= resp.clicked();
            }
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), block), crate::dir::col(), |ui| {
                ui.spacing_mut().item_spacing.y = 2.;
                if can_name {
                    let field = ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut f.name).hint_text(i18n::t("Name")).char_limit(matrix_engine::profile::MAX_NAME_CHARS));
                    save |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                } else {
                    ui.label(shown_name);
                }
                ui.label(RichText::new(&id).size(12.).color(rgb(t.secondary)));
            });
        };
        // `Sides` names its halves left and right: in RTL picture and name go to the right one.
        let sides = egui::containers::Sides::new().height(72.);
        if crate::dir::rtl() { sides.shrink_right().show(ui, |ui| *logout = sign_out_button(ui), main); } else { sides.shrink_left().show(ui, main, |ui| *logout = sign_out_button(ui)); }
        if can_name || can_photo {
            ui.add_space(4.);
            ui.horizontal(|ui| {
                if can_photo {
                    choose |= ui.button(i18n::t("Choose photo…")).clicked();
                    let has_photo = match f.photo {
                        Photo::Keep => current.is_some(),
                        Photo::Set(..) => true,
                        Photo::Remove => false,
                    };
                    if has_photo && ui.button(i18n::t("Remove photo")).clicked() {
                        // Nothing to remove on the server: dropping the choice is enough.
                        f.photo = if current.is_some() { Photo::Remove } else { Photo::Keep };
                        f.editor = None;
                    }
                }
                ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
                    if busy {
                        ui.spinner();
                    } else {
                        save |= ui.add_enabled(dirty, egui::Button::new(i18n::t("Save"))).clicked();
                    }
                });
            });
        }
    });
    if f.hook_save && !f.editing() {
        f.hook_save = false;
        save = true;
    }
    if choose {
        crate::pick::start(ui.ctx(), i18n::t("Profile photo"), true);
    }
    if let Some(path) = crate::pick::done(i18n::t("Profile photo")) {
        f.pick(path, profile, ui.ctx());
    }
    // One line: the answer to the last «Salva».
    match (s.profile_saved, dirty) {
        (Some(Err(line)), _) => {
            ui.add(egui::Label::new(RichText::new(line).color(ui.visuals().error_fg_color)).truncate());
        }
        (Some(Ok(())), false) => {
            ui.label(RichText::new(i18n::t("Saved")).color(rgb(t.secondary)));
        }
        _ => {}
    }
    if save && dirty && !busy && !f.editing() {
        let avatar = match &f.photo {
            Photo::Keep => ProfileAvatar::Keep,
            Photo::Remove => ProfileAvatar::Remove,
            Photo::Set(path, _) => ProfileAvatar::Set(path.clone()),
        };
        if v.save_profile(name_changed.then(|| f.name.trim().to_owned()), avatar) {
            s.profile_busy = true;
            f.sent = true;
            s.profile_saved = None;
        }
    }
    // The picture in the circle is the one to retouch: the applied one, else the account's; with none, a file is asked for first.
    if retouch {
        if let Some(e) = &mut f.editor {
            e.open = true;
        } else if let Photo::Set(path, _) = &f.photo {
            f.pick(path.clone(), profile, ui.ctx());
        } else if let (Photo::Keep, Some(mxc)) = (&f.photo, &current) {
            f.editor = Some(Editor::from_server(s.avatar_original(mxc), draft(profile), ui.ctx()));
        } else {
            crate::pick::start(ui.ctx(), i18n::t("Profile photo"), true);
        }
    }
    true
}

/// What the user asked for: open a profile folder in a new window.
pub enum Launch {
    Open(PathBuf),
    Add,
    /// «Esci» on the account whose profile folder this is.
    SignOut(PathBuf),
}

/// What the running accounts (the ones not on screen) add to their row.
pub struct Running {
    pub dir: PathBuf,
    pub picture: crate::avatar::Avatar,
    pub unread: u64,
}

/// One account: round picture, name, id and server in grey, then «Attivo» or the unread badge and «Apri».
fn row(ui: &mut egui::Ui, name: &str, line: &str, picture: &crate::avatar::Avatar, active: bool, unread: u64, t: &DesignTokens) -> (bool, bool) {
    let (mut open, mut logout) = (false, false);
    let m = crate::metrics::get();
    crate::rows::lines_row(
        ui,
        |ui| {
            let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(32.), egui::Sense::hover());
            crate::avatar::paint(ui, r.center(), 32., rgb(crate::AVATAR), picture, &matrix_desktop::initials(name), 13., rgb(t.on_accent));
        },
        &[crate::rows::Line::new(name, m.body, rgb(t.text)), crate::rows::Line::new(line, m.secondary, rgb(t.secondary))],
        |ui| {
            logout = sign_out_button(ui);
            if active {
                ui.label(RichText::new(i18n::t("Active")).size(m.secondary).color(rgb(t.accent)));
            } else {
                open = ui.add(egui::Button::new(i18n::t("Open")).wrap_mode(egui::TextWrapMode::Extend)).clicked();
                if unread > 0 {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(24., 20.), egui::Sense::hover());
                    crate::list::badge(ui, r.center(), unread, t);
                }
            }
        },
    );
    (open, logout)
}

/// The icon-only «Esci» of a row or of the card.
fn sign_out_button(ui: &mut egui::Ui) -> bool {
    let label = i18n::t("Log out");
    crate::icons::button(ui, crate::icons::Icon::SignOut, false, label).on_hover_text(label).clicked()
}

/// The «Account» drawer, opened from «Impostazioni»; «‹» goes back there.
pub fn show(ctx: &egui::Context, rail: &mut crate::rail::State, dirs: &AppDirs, current: &std::path::Path, session: Option<&mut Session>, running: &[Running], form: &mut Form, t: &DesignTokens) -> Option<Launch> {
    let mut out = None;
    let mut back = false;
    let own = session.as_ref().map_or(crate::avatar::Avatar::Missing, |s| s.own_avatar());
    let closed = crate::rail::drawer_back(ctx, rail.panel, "accounts-drawer", i18n::t("Account"), rail.accounts, t, Some(&mut back), |ui, rect| {
        let size = crate::metrics::button() + 4.;
        let action = crate::dir::flip(rect, egui::Rect::from_center_size(egui::pos2(rect.right() - 30. - size, rect.top() - 28.), egui::Vec2::splat(size)));
        let label = matrix_desktop::i18n::t("Add account");
        if crate::icons::button_at(ui, action, egui::Id::new("add-account"), crate::icons::Icon::Plus, false, crate::metrics::get().icon, label, Some(rgb(t.accent)), true, 1.).on_hover_text(label).clicked() {
            out = Some(Launch::Add);
        }
        let catalog = AccountCatalog::load(dirs);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.;
                    let mut logout = false;
                    let carded = session.is_some_and(|s| {
                        let will = s.verification.as_ref().is_some_and(|v| v.profile().is_some());
                        if will {
                            crate::settings::section(ui, i18n::t("Active account"), t);
                        }
                        card(ui, s, form, current, &mut logout, t)
                    });
                    if logout {
                        out = Some(Launch::SignOut(current.to_owned()));
                    }
                    if carded {
                        crate::settings::section(ui, i18n::t("Other accounts"), t);
                    }
                    for entry in catalog.accounts() {
                        let dir = catalog.data_dir(dirs, entry.id);
                        let active = dir.as_deref() == Some(current);
                        if active && carded {
                            continue;
                        }
                        // The id tells apart accounts with the same name; the server follows unless the id already says it.
                        let host = entry.homeserver.as_deref().map(|h| h.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/'));
                        let line = match (entry.user_id.as_deref(), host) {
                            (Some(id), Some(h)) if !id.ends_with(h) => format!("{id} · {h}"),
                            (Some(id), _) => id.to_owned(),
                            (None, h) => h.unwrap_or_default().to_owned(),
                        };
                        let other = running.iter().find(|r| Some(&r.dir) == dir.as_ref());
                        let picture = if active { &own } else { other.map_or(&crate::avatar::Avatar::Missing, |r| &r.picture) };
                        let (open, logout) = row(ui, &entry.label, &line, picture, active, other.map_or(0, |r| r.unread), t);
                        if let Some(dir) = dir {
                            if logout {
                                out = Some(Launch::SignOut(dir));
                            } else if open {
                                out = Some(Launch::Open(dir));
                            }
                        }
                    }
                    // Debug builds only (screenshots): three accounts with the same name.
                    #[cfg(debug_assertions)]
                    if matrix_desktop::hooks::var_os("GNOBBI_EGUI_FAKE_ACCOUNTS").is_some() {
                        for (id, unread) in [("@marco:example.com · example.com", 0), ("@marco:gnobbi.chat · gnobbi.chat", 3), ("@marco:matrix.org", 120)] {
                            row(ui, "Marco", id, &crate::avatar::Avatar::Missing, id.contains("example") && !carded, unread, t);
                        }
                    }
                });
            });
        });
    });
    if back {
        rail.set_settings(true);
    } else if closed {
        rail.accounts = false;
    }
    out
}

/// One row of the account switcher in the rail: small round picture, name, the account id in grey under it, a check on the shown one, the unread badge on the others.
pub fn switch_row(ui: &mut egui::Ui, name: &str, id: &str, picture: &crate::avatar::Avatar, shown: bool, unread: u64, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(264., 44.), egui::Sense::click());
    let resp = crate::metrics::clickable(resp);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name} {id}")));
    let p = ui.painter();
    if resp.hovered() || resp.has_focus() {
        p.rect_filled(rect, 8., rgb(t.input));
    }
    // LTR maths; `flip`/`place` put each piece on its side.
    let (flip, place) = (|p: egui::Pos2| crate::dir::flip_pos(rect, p), |p: egui::Pos2, g: &egui::Galley| crate::dir::place(rect, p, g.size().x));
    let c = egui::Pos2::new(rect.left() + 8. + 12., rect.center().y);
    crate::avatar::paint(ui, flip(c), 24., rgb(crate::AVATAR), picture, &matrix_desktop::initials(name), 11., rgb(t.on_accent));
    let p = ui.painter();
    let x = c.x + 20.;
    let right = rect.right() - 12.;
    let (name_pos, id_pos) = if id.trim().is_empty() || id.trim() == name.trim() { (egui::Pos2::new(x, rect.center().y - 8.), None) } else { (egui::Pos2::new(x, rect.top() + 6.), Some(egui::Pos2::new(x, rect.top() + 24.))) };
    let g = crate::list::one_line(ui, name, 13., rgb(t.text), right - x - 32.);
    ui.painter().galley(place(name_pos, &g), g, rgb(t.text));
    if let Some(pos) = id_pos {
        let g = crate::list::one_line(ui, id, 11., rgb(t.secondary), right - x - 32.);
        ui.painter().galley(place(pos, &g), g, rgb(t.secondary));
    }
    if shown {
        crate::icons::paint(ui.painter(), flip(egui::Pos2::new(right - 8., rect.center().y)), crate::icons::Icon::Check, true, crate::metrics::get().icon - 2., rgb(t.accent));
    } else if unread > 0 {
        let count = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
        let g = p.layout_no_wrap(count, egui::FontId::proportional(9.), rgb(t.on_accent));
        let badge = egui::Rect::from_center_size(flip(egui::Pos2::new(right - 7., rect.center().y)), egui::Vec2::new((g.size().x + 6.).max(14.), 14.));
        p.rect_filled(badge, 7., rgb(t.accent));
        p.galley(badge.center() - g.size() / 2., g, rgb(t.on_accent));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The account being signed out: asked, waiting for the server, or refused by it.
pub struct SignOut {
    pub dir: PathBuf,
    name: String,
    user_id: String,
    phase: Phase,
}

enum Phase {
    Ask,
    Waiting(matrix_engine::WorkerControl),
    /// The server did not take the logout (offline): remove here anyway, or keep the account.
    Failed,
}

/// What the user did in the window, or what the worker finished with.
pub enum SignOutStep {
    /// «Esci» pressed: end this account's worker with the server logout.
    Start,
    /// Gone from the server, or the user chose to remove it here anyway.
    Remove,
    /// «Annulla» after a failed logout: the stopped account must be started again.
    Restart,
    Close,
}

impl SignOut {
    /// `name` and `user_id` come from the catalog entry of `dir`.
    pub fn new(dir: PathBuf, name: &str, user_id: &str) -> Self {
        SignOut { dir, name: name.to_owned(), user_id: user_id.to_owned(), phase: Phase::Ask }
    }

    /// No worker could open the profile: keep the existing explicit local-removal confirmation.
    pub fn local_only(&mut self) {
        self.phase = Phase::Failed;
    }

    pub fn wait_for(&mut self, control: matrix_engine::WorkerControl) {
        self.phase = Phase::Waiting(control);
    }

    /// The confirmation window (small, name and id, destructive «Esci»); drawn every frame while it is open.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<SignOutStep> {
        if let Phase::Waiting(c) = &self.phase
            && c.is_done()
        {
            if c.revoked() {
                return Some(SignOutStep::Remove);
            }
            self.phase = Phase::Failed;
        }
        let mut step = None;
        let modal = egui::Modal::new(egui::Id::new("sign-out-account")).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.set_max_width(crate::dir::fit(ui, 320.));
            ui.heading(i18n::t("Log out"));
            ui.label(&self.name);
            ui.label(RichText::new(&self.user_id).size(12.).color(ui.visuals().weak_text_color()));
            let line = match &self.phase {
                Phase::Ask => i18n::t("Encryption keys on this device not backed up are lost."),
                Phase::Waiting(_) => "",
                Phase::Failed => i18n::t("The server did not answer. Remove it from this device anyway?"),
            };
            if !line.is_empty() {
                ui.label(line);
            }
            let tall = egui::vec2(0., crate::metrics::button());
            ui.horizontal(|ui| match &self.phase {
                Phase::Waiting(_) => {
                    ui.spinner();
                }
                phase => {
                    let failed = matches!(phase, Phase::Failed);
                    if ui.add(egui::Button::new(i18n::t("Cancel")).min_size(tall)).clicked() {
                        step = Some(if failed { SignOutStep::Restart } else { SignOutStep::Close });
                    }
                    let red = ui.visuals().error_fg_color;
                    let go = egui::Button::new(RichText::new(i18n::t(if failed { "Remove from this device" } else { "Log out" })).color(egui::Color32::WHITE)).fill(red).min_size(tall);
                    if ui.add(go).clicked() {
                        step = Some(if failed { SignOutStep::Remove } else { SignOutStep::Start });
                    }
                }
            });
        }));
        if step.is_none() && modal.should_close() && !matches!(self.phase, Phase::Waiting(_)) {
            step = Some(if matches!(self.phase, Phase::Failed) { SignOutStep::Restart } else { SignOutStep::Close });
        }
        step
    }
}

/// Deletes everything one account left on this device: vault items, profile folder (the `-video` sibling and the media cache too).
/// Blocking. Vault and file errors are ignored: nothing here may stop the removal of the others' data being untouched.
pub fn wipe(dir: &std::path::Path, cache: Option<PathBuf>) {
    wipe_with(dir, cache, |id, stores| {
        // iOS forget_secrets preserves legacy worker-ID items and deletes only these profile stores.
        let _ = matrix_engine::signin::forget_secrets(id, stores);
    });
}

fn wipe_with(dir: &std::path::Path, cache: Option<PathBuf>, mut forget: impl FnMut(matrix_domain::AccountId, &[PathBuf])) {
    // The store folders are `account-<n>`; their vault items are keyed by that path and number.
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(n) = name.strip_prefix("account-").and_then(|n| n.parse::<u64>().ok()) {
            forget(matrix_domain::AccountId::new(n), &[entry.path()]);
        }
    }
    let mut video = dir.file_name().unwrap_or_default().to_owned();
    video.push("-video");
    for d in [Some(dir.to_owned()), Some(dir.with_file_name(video)), cache].into_iter().flatten() {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wipe_only_forgets_stores_under_the_selected_catalog_profile() {
        let base = std::env::temp_dir().join(format!("gnobbi-wipe-stores-{}", std::process::id()));
        let (a, b) = (base.join("accounts/2"), base.join("accounts/3"));
        for path in [a.join("account-1"), b.join("account-1"), a.join("unrelated")] {
            std::fs::create_dir_all(path).unwrap();
        }
        let mut forgotten = Vec::new();
        super::wipe_with(&a, None, |id, paths| forgotten.push((id.as_u64(), paths.to_vec())));
        assert_eq!(forgotten, vec![(1, vec![a.join("account-1")])]);
        assert!(b.join("account-1").is_dir());
        assert!(!a.exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn wipe_removes_the_profile_its_video_folder_and_cache_only() {
        let base = std::env::temp_dir().join(format!("gnobbi-wipe-{}", std::process::id()));
        let (a, b, cache) = (base.join("accounts/1"), base.join("accounts/2"), base.join("cache1"));
        for d in [&a, &b, &cache, &base.join("accounts/1-video")] {
            std::fs::create_dir_all(d).unwrap();
        }
        super::wipe(&a, Some(cache.clone()));
        assert!(!a.exists() && !cache.exists() && !base.join("accounts/1-video").exists());
        assert!(b.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
