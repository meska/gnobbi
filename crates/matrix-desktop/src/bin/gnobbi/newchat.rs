//! «Nuova chat» window: direct chat by user search, new room, new space, join by address.
use matrix_desktop::i18n;
use crate::session::Session;
use eframe::egui::{self, RichText};
use matrix_engine::operations::{SearchState, SpaceAvatar};

#[derive(Default)]
pub struct Form {
    tab: u8,
    name: String,
    topic: String,
    public: bool,
    target: String,
    /// «Spazio»: its own name and switch (private unless turned on), the picked picture with its preview.
    space: String,
    space_public: bool,
    avatar: Option<(SpaceAvatar, egui::TextureHandle)>,
    sent: bool,
    /// «Unisciti» was pressed for `target`: the field empties once that join is done.
    joining: bool,
}

#[cfg(test)]
mod creation_tests {
    use super::*;

    #[test]
    fn organizer_opens_the_requested_creation_tab() {
        let mut form = Form::default();
        form.open_creation(false);
        assert_eq!(form.tab, 1);
        form.open_creation(true);
        assert_eq!(form.tab, 2);
    }
}

impl Form {
    pub fn open_creation(&mut self, space: bool) {
        self.tab = if space { 2 } else { 1 };
    }

    /// A `matrix:` link to a room we are not in: the «Unisciti» tab with its address; nothing is joined until the user clicks.
    // shortcut: the link's `via` servers are not passed on, so the join goes through the user's own server only.
    pub fn prefill_join(&mut self, target: &str) {
        self.tab = 3;
        self.target = target.to_owned();
    }

    /// Test hook: the «Spazio» tab, with «Pubblica» on when asked.
    pub fn open_space(&mut self, public: bool) {
        self.tab = 2;
        self.space_public = public;
    }
}

pub fn show(ctx: &egui::Context, s: &mut Session, form: &mut Form, t: &matrix_desktop::DesignTokens) {
    // The space was created (the window closed on its own): the next one starts empty. After an error the fields stay.
    if form.sent && !s.new_busy {
        form.sent = false;
        if !s.new_failed {
            (form.space, form.space_public, form.avatar) = Default::default();
        }
    }
    // Joined (its entry is gone): the next one starts empty. After an error the address stays for «Unisciti» again.
    if form.joining {
        match s.joins.get(form.target.trim()) {
            None => (form.joining, form.target) = Default::default(),
            Some(j) => form.joining = j.is_running(),
        }
    }
    let mut open = true;
    egui::Window::new(i18n::t("New chat")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_min_width(340.);
        ui.horizontal(|ui| {
            for (i, label) in [i18n::t("Direct chat"), i18n::t("Room"), i18n::t("Space"), i18n::t("Join a room")].into_iter().enumerate() {
                ui.selectable_value(&mut form.tab, i as u8, label);
            }
        });
        ui.separator();
        match form.tab {
            0 => {
                if let Some(user) = people(ui, s, t) {
                    s.start_dm(user);
                }
            }
            1 => {
                ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut form.name).hint_text(i18n::t("Name")));
                ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut form.topic).hint_text(i18n::t("Topic")));
                ui.add_space(4.);
                crate::settings::switch(ui, i18n::t("Public"), &mut form.public, t);
                ui.add_space(8.);
                if ui.add_enabled(!form.name.trim().is_empty(), egui::Button::new(i18n::t("Create"))).clicked() {
                    s.create_room(std::mem::take(&mut form.name), std::mem::take(&mut form.topic), form.public);
                }
            }
            2 => {
                ui.add_enabled_ui(!s.new_busy, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(48.), egui::Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, i18n::t("Image")));
                        match &form.avatar {
                            Some((_, tex)) => {
                                crate::avatar::paint(ui, rect.center(), 48., crate::list::rgb(crate::AVATAR), &crate::avatar::Avatar::Ready(tex.clone()), "", 0., egui::Color32::TRANSPARENT);
                            }
                            None => {
                                ui.painter().circle_filled(rect.center(), 24., crate::list::rgb(t.input));
                                crate::icons::paint(ui.painter(), rect.center(), crate::icons::Icon::Plus, false, crate::metrics::get().icon + 6., crate::list::rgb(t.secondary));
                            }
                        }
                        if resp.has_focus() || resp.hovered() {
                            ui.painter().circle_stroke(rect.center(), 24., egui::Stroke::new(2., crate::list::rgb(t.accent)));
                        }
                        if resp.clicked() {
                            crate::pick::start(ui.ctx(), i18n::t("Image"), true);
                        }
                        if let Some(path) = crate::pick::done(i18n::t("Image")) {
                            form.avatar = read_avatar(ui.ctx(), &path);
                            s.new_failed = form.avatar.is_none();
                        }
                        ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut form.space).hint_text(i18n::t("Name")));
                    });
                    ui.add_space(4.);
                    crate::settings::switch(ui, i18n::t("Public"), &mut form.space_public, t);
                });
                ui.add_space(8.);
                if s.new_busy {
                    ui.spinner();
                } else if ui.add_enabled(!form.space.trim().is_empty(), egui::Button::new(i18n::t("Create"))).clicked() {
                    form.sent = true;
                    s.create_space(&form.space, form.space_public, form.avatar.as_ref().map(|(a, _)| a.clone()));
                }
            }
            _ => {
                let join = s.joins.get(form.target.trim());
                let (running, line, failed) = (join.is_some_and(|j| j.is_running()), join.and_then(|j| j.line()), join.is_some_and(|j| j.error().is_some()));
                ui.add_enabled_ui(!running, |ui| ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut form.target).hint_text(i18n::t("#room:server or !id:server"))));
                ui.add_space(8.);
                // One row: the spinner takes the button's place for as long as the server is joining, then the outcome.
                ui.horizontal(|ui| {
                    if running {
                        ui.spinner();
                    } else if ui.add_enabled(!form.target.trim().is_empty(), egui::Button::new(i18n::t("Join a room"))).clicked() {
                        form.joining = true;
                        s.join(&form.target);
                    }
                    if let Some(line) = line {
                        ui.colored_label(if failed { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() }, line);
                    }
                });
            }
        }
        if s.new_failed {
            ui.add_space(6.);
            ui.colored_label(ui.visuals().error_fg_color, i18n::t("Failed"));
        }
    }));
    if !open {
        s.new_open = false;
    }
}

/// The user search («Cerca persone» and its results); returns the user that was clicked.
pub fn people(ui: &mut egui::Ui, s: &mut Session, t: &matrix_desktop::DesignTokens) -> Option<String> {
    let mut picked = None;
    ui.add_sized([ui.available_width(), crate::metrics::button()], crate::metrics::line(&mut s.new_term).hint_text(i18n::t("Search people")));
    ui.add_space(6.);
    match &s.users {
        SearchState::Searching { .. } => {
            ui.spinner();
        }
        SearchState::NoResults { .. } => {
            ui.label(RichText::new(i18n::t("No results")).color(crate::list::rgb(t.secondary)));
        }
        SearchState::Results { hits, .. } => {
            egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
                let duplicates = crate::rows::duplicate_names(hits.iter().map(|h| h.display_name.as_deref().unwrap_or(&h.user_id)));
                for (h, duplicate) in hits.iter().zip(duplicates) {
                    let name = h.display_name.as_deref().unwrap_or(&h.user_id);
                    if ui.push_id(&h.user_id, |ui| crate::rows::person_row(ui, name, &h.user_id, duplicate, false, t)).inner.clicked() {
                        picked = Some(h.user_id.clone());
                    }
                }
            });
        }
        _ => {}
    }
    picked
}

/// The picked file as the engine wants it (bytes + MIME from the content) and its preview; None when it is not a picture.
// The jpeg/webp/gif decoders come from matrix-engine's `image` features (same crate version, unified by cargo).
fn read_avatar(ctx: &egui::Context, path: &std::path::Path) -> Option<(SpaceAvatar, egui::TextureHandle)> {
    let data = std::fs::read(path).ok()?;
    let mime = image::guess_format(&data).ok()?.to_mime_type().to_owned();
    let small = image::load_from_memory(&data).ok()?.thumbnail(96, 96).to_rgba8();
    let pixels = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small);
    Some((SpaceAvatar { data, mime }, ctx.load_texture("new-space-avatar", pixels, Default::default())))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_jpeg_becomes_the_space_avatar_and_other_files_do_not() {
        let dir = std::env::temp_dir();
        let (jpg, txt) = (dir.join("gnobbi-space-avatar-test.jpg"), dir.join("gnobbi-space-avatar-test.txt"));
        image::RgbImage::from_pixel(200, 100, image::Rgb([200, 100, 50])).save_with_format(&jpg, image::ImageFormat::Jpeg).unwrap();
        std::fs::write(&txt, "ciao").unwrap();
        let ctx = crate::icons::test_context();
        let (avatar, preview) = super::read_avatar(&ctx, &jpg).unwrap();
        assert_eq!(avatar.mime, "image/jpeg");
        assert_eq!(preview.size(), [96, 48]);
        assert!(super::read_avatar(&ctx, &txt).is_none());
        let _ = (std::fs::remove_file(jpg), std::fs::remove_file(txt));
    }
}
