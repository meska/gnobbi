//! «Modifica stanza»: name, topic and picture, limited to what the room's power levels allow.
use crate::session::{RoomEdit, Session};
use eframe::egui;
use matrix_desktop::i18n;

pub fn show(ctx: &egui::Context, s: &mut Session, rights: matrix_engine::EditRights, title: &str) {
    let (mut save, mut close) = (false, false);
    let Some(e): Option<&mut RoomEdit> = s.edit.as_mut() else { return };
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_min_width(crate::dir::fit(ui, 320.));
        ui.add_enabled(rights.name, crate::metrics::line(&mut e.name).hint_text(i18n::t("Name")));
        ui.add_enabled(rights.topic, egui::TextEdit::multiline(&mut e.topic).hint_text(i18n::t("Topic")).desired_rows(3));
        if rights.avatar {
            match &e.avatar {
                Some(p) => ui.label(i18n::t("Photo: {name}").replace("{name}", &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                None => ui.label(i18n::t("Drop an image here for the photo")),
            };
            if ui.button(i18n::t("Choose photo…")).clicked() {
                crate::pick::start(ui.ctx(), i18n::t("Room photo"), true);
            }
            if let Some(p) = crate::pick::done(i18n::t("Room photo")) {
                e.avatar = Some(p);
            }
        }
        if e.failed {
            ui.colored_label(ui.visuals().error_fg_color, i18n::t("Failed"));
        }
        ui.horizontal(|ui| {
            save = ui.add_enabled(!e.busy && !e.name.trim().is_empty(), egui::Button::new(i18n::t("Save"))).clicked();
            close = ui.button(i18n::t("Cancel")).clicked();
        });
    }));
    if save {
        s.save_edit();
    }
    if close {
        s.edit = None;
    }
}
