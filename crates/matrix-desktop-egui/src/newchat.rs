//! «Nuova chat» window: direct chat by user search, new room, join by address.
use crate::session::Session;
use eframe::egui::{self, RichText};
use matrix_engine::operations::SearchState;

#[derive(Default)]
pub struct Form {
    tab: u8,
    name: String,
    topic: String,
    public: bool,
    target: String,
}

pub fn show(ctx: &egui::Context, s: &mut Session, form: &mut Form, t: &matrix_desktop::DesignTokens) {
    let mut open = true;
    egui::Window::new("Nuova chat").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.set_min_width(340.);
        ui.horizontal(|ui| {
            for (i, label) in ["Chat diretta", "Stanza", "Unisciti"].into_iter().enumerate() {
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
