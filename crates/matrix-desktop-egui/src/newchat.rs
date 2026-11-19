//! «Nuova chat» window: direct chat by user search, new room, new space, join by address.
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
}

impl Form {
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
    let mut open = true;
    egui::Window::new("Nuova chat").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(340.);
        ui.horizontal(|ui| {
            for (i, label) in ["Chat diretta", "Stanza", "Spazio", "Unisciti"].into_iter().enumerate() {
                ui.selectable_value(&mut form.tab, i as u8, label);
            }
        });
        ui.separator();
        match form.tab {
            0 => {
                ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut s.new_term).hint_text("Cerca persone").align(egui::Align2::LEFT_CENTER));
                ui.add_space(6.);
                match s.users.clone() {
                    SearchState::Searching { .. } => {
                        ui.spinner();
                    }
                    SearchState::NoResults { .. } => {
                        ui.label(RichText::new("Nessun risultato").color(crate::list::rgb(t.secondary)));
                    }
                    SearchState::Results { hits, .. } => {
                        egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
                            for h in hits {
                                let name = h.display_name.clone().unwrap_or_else(|| h.user_id.clone());
                                if ui.button(format!("{name}  {}", h.user_id)).clicked() {
                                    s.start_dm(h.user_id);
                                }
                            }
                        });
                    }
                    _ => {}
                }
            }
            1 => {
                ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut form.name).hint_text("Nome"));
                ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut form.topic).hint_text("Argomento"));
                ui.add_space(4.);
                crate::settings::switch(ui, "Pubblica", &mut form.public, t);
                ui.add_space(8.);
                if ui.add_enabled(!form.name.trim().is_empty(), egui::Button::new("Crea")).clicked() {
                    s.create_room(std::mem::take(&mut form.name), std::mem::take(&mut form.topic), form.public);
                }
            }
            2 => {
                ui.add_enabled_ui(!s.new_busy, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(48.), egui::Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Immagine"));
                        match &form.avatar {
                            Some((_, tex)) => {
                                egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(egui::CornerRadius::same(24)).paint_at(ui, rect);
                            }
                            None => {
                                ui.painter().circle_filled(rect.center(), 24., crate::list::rgb(t.input));
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(22.), crate::list::rgb(t.secondary));
                            }
                        }
                        if resp.has_focus() || resp.hovered() {
                            ui.painter().circle_stroke(rect.center(), 24., egui::Stroke::new(2., crate::list::rgb(t.accent)));
                        }
                        if resp.clicked()
                            && let Some(path) = crate::pick::file("Immagine", true)
                        {
                            form.avatar = read_avatar(ui.ctx(), &path);
                            s.new_failed = form.avatar.is_none();
                        }
                        ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut form.space).hint_text("Nome"));
                    });
                    ui.add_space(4.);
                    crate::settings::switch(ui, "Pubblica", &mut form.space_public, t);
                });
                ui.add_space(8.);
                if s.new_busy {
                    ui.spinner();
                } else if ui.add_enabled(!form.space.trim().is_empty(), egui::Button::new("Crea")).clicked() {
                    form.sent = true;
                    s.create_space(&form.space, form.space_public, form.avatar.as_ref().map(|(a, _)| a.clone()));
                }
            }
            _ => {
                ui.add_sized([ui.available_width(), 32.], egui::TextEdit::singleline(&mut form.target).hint_text("#stanza:server o !id:server"));
                ui.add_space(8.);
                if ui.add_enabled(!form.target.trim().is_empty(), egui::Button::new("Unisciti")).clicked() {
                    s.join(std::mem::take(&mut form.target));
                }
            }
        }
        if s.new_failed {
            ui.add_space(6.);
            ui.colored_label(ui.visuals().error_fg_color, "Non riuscito");
        }
    });
    if !open {
        s.new_open = false;
    }
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
        let ctx = eframe::egui::Context::default();
        let (avatar, preview) = super::read_avatar(&ctx, &jpg).unwrap();
        assert_eq!(avatar.mime, "image/jpeg");
        assert_eq!(preview.size(), [96, 48]);
        assert!(super::read_avatar(&ctx, &txt).is_none());
        let _ = (std::fs::remove_file(jpg), std::fs::remove_file(txt));
    }
}
