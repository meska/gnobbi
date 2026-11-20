//! «Account» window: the card of the account on screen (name and picture, editable where the server allows)
//! and the other accounts of the catalog, all running in this window; "Apri" brings one on screen.
use crate::list::rgb;
use crate::session::Session;
use eframe::egui::{self, RichText};
use matrix_desktop::{AccountCatalog, AppDirs, DesignTokens};
use matrix_engine::profile::{PreparedAvatar, ProfileError};
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
    /// A chosen file being decoded off the UI thread.
    loading: Option<(PathBuf, std::sync::mpsc::Receiver<Result<PreparedAvatar, ProfileError>>)>,
    /// A chosen file that is not a usable picture.
    bad_file: Option<&'static str>,
    /// A «Salva» is on its way: a good answer drops the chosen picture (the server's one is shown again).
    sent: bool,
    /// Test hook state (see `card`).
    hooked: bool,
    hook_save: bool,
}

impl Form {
    /// A file from the picker or dropped on the window becomes the preview once decoded.
    pub fn pick(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (file, ctx) = (path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(matrix_engine::profile::prepare_blocking(&file));
            ctx.request_repaint();
        });
        self.loading = Some((path, rx));
        self.bad_file = None;
    }
}

/// The card of the account on screen. False until the profile has arrived (the plain row is drawn instead).
fn card(ui: &mut egui::Ui, s: &mut Session, f: &mut Form, t: &DesignTokens) -> bool {
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
        if let Ok(name) = std::env::var("GNOBBI_EGUI_NAME") {
            f.name = name;
        }
        match std::env::var("GNOBBI_EGUI_PHOTO") {
            Ok(p) if p == "remove" => f.photo = Photo::Remove,
            Ok(p) => f.pick(p.into(), ui.ctx()),
            Err(_) => {}
        }
        f.hook_save = std::env::var_os("GNOBBI_EGUI_CONFIRM").is_some();
    }
    if let Some((path, rx)) = &f.loading
        && let Ok(done) = rx.try_recv()
    {
        match done {
            Ok(p) => {
                let (w, h, rgba) = p.preview;
                let pixels = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                f.photo = Photo::Set(path.clone(), ui.ctx().load_texture("own-avatar-preview", pixels, Default::default()));
            }
            Err(e) => f.bad_file = Some(e.message()),
        }
        f.loading = None;
    }
    if f.sent && !s.profile_busy {
        f.sent = false;
        if s.profile_saved == Some(Ok(())) {
            f.photo = Photo::Keep;
        }
    }
    let (id, name) = shown;
    let current = v.profile_avatar().map(str::to_owned);
    let own = current.as_ref().and_then(|mxc| s.avatars.get(mxc)?.clone());
    let busy = s.profile_busy;
    let name_changed = can_name && f.name.trim() != name.as_deref().unwrap_or("") && !f.name.trim().is_empty();
    let dirty = name_changed || !matches!(f.photo, Photo::Keep);
    let (mut save, mut choose) = (false, false);
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(64.), if can_photo { egui::Sense::click() } else { egui::Sense::hover() });
            let shown_name = name.as_deref().unwrap_or(&id);
            let picture = match &f.photo {
                Photo::Set(_, tex) => Some(tex),
                Photo::Keep => own.as_ref(),
                Photo::Remove => None,
            };
            match picture {
                Some(tex) => {
                    egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(egui::CornerRadius::same(32)).paint_at(ui, rect);
                }
                None => {
                    ui.painter().circle_filled(rect.center(), 32., rgb(crate::AVATAR));
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, matrix_desktop::initials(shown_name), egui::FontId::proportional(24.), rgb(t.on_accent));
                }
            }
            if can_photo {
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Scegli foto"));
                if resp.has_focus() || resp.hovered() {
                    ui.painter().circle_stroke(rect.center(), 32., egui::Stroke::new(2., rgb(t.accent)));
                }
                choose |= resp.clicked();
            }
            ui.add_space(4.);
            ui.vertical(|ui| {
                if can_name {
                    let field = ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut f.name).hint_text("Nome").align(egui::Align2::LEFT_CENTER).char_limit(matrix_engine::profile::MAX_NAME_CHARS));
                    save |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                } else {
                    ui.label(shown_name);
                }
                ui.label(RichText::new(&id).size(12.).color(rgb(t.secondary)));
            });
        });
        if can_name || can_photo {
            ui.add_space(4.);
            ui.horizontal(|ui| {
                if can_photo {
                    choose |= ui.button("Scegli foto…").clicked();
                    let has_photo = match f.photo {
                        Photo::Keep => current.is_some(),
                        Photo::Set(..) => true,
                        Photo::Remove => false,
                    };
                    if has_photo && ui.button("Rimuovi foto").clicked() {
                        // Nothing to remove on the server: dropping the choice is enough.
                        f.photo = if current.is_some() { Photo::Remove } else { Photo::Keep };
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy || f.loading.is_some() {
                        ui.spinner();
                    } else {
                        save |= ui.add_enabled(dirty, egui::Button::new("Salva")).clicked();
                    }
                });
            });
        }
    });
    if f.hook_save && f.loading.is_none() {
        f.hook_save = false;
        save = true;
    }
    if choose && let Some(path) = crate::pick::file("Foto del profilo", true) {
        f.pick(path, ui.ctx());
    }
    // One line: the file's problem, else the answer to the last «Salva».
    match (f.bad_file, s.profile_saved, dirty) {
        (Some(line), ..) | (None, Some(Err(line)), _) => {
            ui.add(egui::Label::new(RichText::new(line).color(ui.visuals().error_fg_color)).truncate());
        }
        (None, Some(Ok(())), false) => {
            ui.label(RichText::new("Salvato").color(rgb(t.secondary)));
        }
        _ => {}
    }
    if save && dirty && !busy && f.loading.is_none() {
        let avatar = match &f.photo {
            Photo::Keep => ProfileAvatar::Keep,
            Photo::Remove => ProfileAvatar::Remove,
            Photo::Set(path, _) => ProfileAvatar::Set(path.clone()),
        };
        if v.save_profile(name_changed.then(|| f.name.trim().to_owned()), avatar) {
            s.profile_busy = true;
            f.sent = true;
            s.profile_saved = None;
            f.bad_file = None;
        }
    }
    true
}

/// What the user asked for: open a profile folder in a new window.
pub enum Launch {
    Open(PathBuf),
    Add,
}

pub fn show(ctx: &egui::Context, open: &mut bool, dirs: &AppDirs, current: &std::path::Path, session: Option<&mut Session>, form: &mut Form, t: &DesignTokens) -> Option<Launch> {
    let mut out = None;
    let catalog = AccountCatalog::load(dirs);
    egui::Window::new("Account").open(open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        let secondary = rgb(t.secondary);
        let carded = session.is_some_and(|s| card(ui, s, form, t));
        if carded {
            ui.separator();
        }
        for entry in catalog.accounts() {
            let dir = catalog.data_dir(dirs, entry.id);
            let active = dir.as_deref() == Some(current);
            if active && carded {
                continue;
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(&entry.label);
                    ui.label(RichText::new(entry.user_id.as_deref().unwrap_or("")).size(12.).color(secondary));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if active {
                        ui.label("Attivo");
                    } else if let Some(dir) = dir
                        && ui.button("Apri").clicked()
                    {
                        out = Some(Launch::Open(dir));
                    }
                });
            });
            ui.separator();
        }
        if ui.button("Aggiungi account").clicked() {
            out = Some(Launch::Add);
        }
    });
    out
}


/// One row of the account switcher in the rail: small round picture, name, the account id in grey under it, a check on the shown one, the unread badge on the others.
pub fn switch_row(ui: &mut egui::Ui, name: &str, id: &str, picture: Option<&egui::TextureHandle>, shown: bool, unread: u64, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(264., 44.), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name} {id}")));
    let p = ui.painter();
    if resp.hovered() || resp.has_focus() {
        p.rect_filled(rect, 8., rgb(t.input));
    }
    let c = egui::Pos2::new(rect.left() + 8. + 12., rect.center().y);
    p.circle_filled(c, 12., rgb(crate::AVATAR));
    p.text(c, egui::Align2::CENTER_CENTER, matrix_desktop::initials(name), egui::FontId::proportional(11.), rgb(t.on_accent));
    if let Some(tex) = picture {
        let r = egui::Rect::from_center_size(c, egui::Vec2::splat(24.));
        egui::Image::new(tex).fit_to_exact_size(r.size()).corner_radius(egui::CornerRadius::same(12)).paint_at(ui, r);
    }
    let x = c.x + 20.;
    let right = rect.right() - 12.;
    let (name_pos, id_pos) = if id.is_empty() { (egui::Pos2::new(x, rect.center().y - 8.), None) } else { (egui::Pos2::new(x, rect.top() + 6.), Some(egui::Pos2::new(x, rect.top() + 24.))) };
    ui.painter().galley(name_pos, crate::list::one_line(ui, name, 13., rgb(t.text), right - x - 32.), rgb(t.text));
    if let Some(pos) = id_pos {
        ui.painter().galley(pos, crate::list::one_line(ui, id, 11., rgb(t.secondary), right - x - 32.), rgb(t.secondary));
    }
    if shown {
        ui.painter().text(egui::Pos2::new(right, rect.center().y), egui::Align2::RIGHT_CENTER, "✓", egui::FontId::proportional(14.), rgb(t.accent));
    } else if unread > 0 {
        let count = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
        let g = p.layout_no_wrap(count, egui::FontId::proportional(9.), rgb(t.on_accent));
        let badge = egui::Rect::from_center_size(egui::Pos2::new(right - 7., rect.center().y), egui::Vec2::new((g.size().x + 6.).max(14.), 14.));
        p.rect_filled(badge, 7., rgb(t.accent));
        p.galley(badge.center() - g.size() / 2., g, rgb(t.on_accent));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
