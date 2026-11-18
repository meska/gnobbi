//! «Modifica stanza»: name, topic and picture, limited to what the room's power levels allow.
use crate::session::{RoomEdit, Session};
use eframe::egui;

pub fn show(ctx: &egui::Context, s: &mut Session, rights: matrix_engine::EditRights) {
    let (mut save, mut close) = (false, false);
    let Some(e): Option<&mut RoomEdit> = s.edit.as_mut() else { return };
    egui::Window::new("Modifica stanza").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        ui.add_enabled(rights.name, egui::TextEdit::singleline(&mut e.name).hint_text("Nome"));
        ui.add_enabled(rights.topic, egui::TextEdit::multiline(&mut e.topic).hint_text("Argomento").desired_rows(3));
        if rights.avatar {
            match &e.avatar {
                Some(p) => ui.label(format!("Foto: {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                None => ui.label("Trascina un'immagine qui per la foto"),
            };
        }
        if e.failed {
            ui.colored_label(ui.visuals().error_fg_color, "Non riuscito");
        }
        ui.horizontal(|ui| {
            save = ui.add_enabled(!e.busy && !e.name.trim().is_empty(), egui::Button::new("Salva")).clicked();
            close = ui.button("Annulla").clicked();
        });
    });
    if save {
        s.save_edit();
    }
    if close {
        s.edit = None;
    }
}
