//! Right panel: the open room's facts and its members.
use eframe::egui::{self, Align2, Color32, FontId, RichText, Vec2};
use matrix_desktop::{DesignTokens, SidebarRoom};
use matrix_engine::operations::Member;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// What the user did in the panel this frame.
#[derive(Default)]
pub struct Out {
    pub open_profile: Option<String>,
    pub edit: bool,
}

pub fn show(ui: &mut egui::Ui, room: &SidebarRoom, members: Option<&[Member]>, t: &DesignTokens) -> Out {
    let mut out = Out::default();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(16.);
        ui.horizontal(|ui| {
            ui.add_space(16.);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 16.);
                ui.heading(&room.name);
                if let Some(topic) = room.info.topic.as_deref().filter(|t| !t.is_empty()) {
                    ui.add_space(4.);
                    ui.label(RichText::new(topic).color(rgb(t.secondary)));
                }
                if room.info.edit_rights.any() && ui.button("Modifica").clicked() {
                    out.edit = true;
                }
                ui.add_space(8.);
                ui.label(RichText::new(format!("{} membri", room.info.joined_members)).color(rgb(t.secondary)));
                if room.info.encrypted == Some(true) {
                    ui.label(RichText::new("Cifrata").color(rgb(t.secondary)));
                }
            });
        });
        ui.add_space(12.);
        ui.separator();
        let Some(members) = members else {
            ui.add_space(12.);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        for m in members {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), egui::Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 0., rgb(t.input));
            }
            if resp.clicked() {
                out.open_profile = Some(m.user_id.clone());
            }
            let c = egui::pos2(rect.left() + 28., rect.center().y);
            let initials: String = m.name.trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
            ui.painter().circle_filled(c, 14., rgb(matrix_desktop::AVATAR_FALLBACK));
            ui.painter().text(c, Align2::CENTER_CENTER, initials, FontId::proportional(12.), rgb(t.on_accent));
            let galley = ui.painter().layout(m.name.clone(), FontId::proportional(14.), rgb(t.text), (rect.width() - 60.).max(20.));
            ui.painter().galley(egui::pos2(rect.left() + 52., rect.center().y - galley.size().y / 2.), galley, rgb(t.text));
        }
    });
    out
}

/// Message search of the open room: term box and hits (a hit cannot be jumped to yet: the timeline only holds what was paged in).
pub fn search(ui: &mut egui::Ui, term: &mut String, state: &matrix_engine::operations::MessageSearchState, t: &DesignTokens) {
    use matrix_engine::operations::MessageSearchState as S;
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        ui.add_sized([w, 32.], egui::TextEdit::singleline(term).hint_text("Cerca nella stanza"));
    });
    ui.add_space(8.);
    match state {
        S::Idle => {}
        S::Searching { .. } => {
            ui.vertical_centered(|ui| ui.spinner());
        }
        S::NoResults { .. } => {
            ui.label(RichText::new("Nessun risultato").color(rgb(t.secondary)));
        }
        S::Error { .. } => {
            ui.colored_label(rgb(t.error), "Ricerca non riuscita");
        }
        S::Results { hits, .. } => {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for h in hits {
                    ui.add_space(6.);
                    ui.horizontal(|ui| {
                        ui.add_space(12.);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 12.);
                            ui.label(RichText::new(&h.sender_name).strong().size(12.).color(rgb(t.accent)));
                            ui.label(&h.excerpt);
                        });
                    });
                    ui.separator();
                }
            });
        }
    }
}

/// A member's card: name, id with the small copy icon, trust and the ignore switch. Returns `Some(ignore)` when it was flipped; `close` is set by the X.
pub fn card(ui: &mut egui::Ui, id: &str, profile: Option<&matrix_engine::operations::UserProfile>, close: &mut bool, t: &DesignTokens) -> Option<bool> {
    let mut flip = None;
    ui.add_space(16.);
    ui.horizontal(|ui| {
        ui.add_space(16.);
        if ui.small_button("✕").clicked() {
            *close = true;
        }
    });
    let Some(p) = profile else {
        ui.add_space(16.);
        ui.vertical_centered(|ui| ui.spinner());
        return None;
    };
    ui.horizontal(|ui| {
        ui.add_space(16.);
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 16.);
            ui.heading(&p.name);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(id).color(rgb(t.secondary)));
                if ui.small_button("⧉").clicked() {
                    ui.ctx().copy_text(id.to_owned());
                }
            });
            match p.verified {
                Some(true) => ui.label("Verificato"),
                Some(false) => ui.label(RichText::new("Non verificato").color(rgb(t.error))),
                None => ui.label(""),
            };
            ui.add_space(8.);
            let mut ignored = p.ignored;
            if crate::settings::switch(ui, "Ignora", &mut ignored, t) {
                flip = Some(ignored);
            }
        });
    });
    flip
}

/// Thread panel: the root, its replies (oldest first) and a line to answer in the thread. Returns the text to send.
pub fn thread(ui: &mut egui::Ui, th: &mut crate::session::ThreadState, close: &mut bool, t: &DesignTokens) -> Option<String> {
    let mut send = None;
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        if ui.small_button("✕").clicked() {
            *close = true;
        }
    });
    egui::Panel::bottom("thread-input").frame(egui::Frame::NONE).show_inside(ui, |ui| {
        ui.add_space(6.);
        let r = ui.add_sized([ui.available_width() - 12., 32.], egui::TextEdit::singleline(&mut th.draft).hint_text("Rispondi nel thread"));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !th.draft.trim().is_empty() {
            send = Some(std::mem::take(&mut th.draft));
            r.request_focus();
        }
        ui.add_space(6.);
    });
    let Some(page) = &th.page else {
        ui.add_space(16.);
        if th.failed {
            ui.colored_label(rgb(t.error), "Thread non disponibile");
        } else {
            ui.vertical_centered(|ui| ui.spinner());
        }
        return send;
    };
    egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
        for e in page.root.iter().chain(page.replies.iter()) {
            ui.add_space(6.);
            ui.horizontal(|ui| {
                ui.add_space(12.);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 12.);
                    let name = e.sender.name.clone().unwrap_or_else(|| e.sender.id.clone());
                    ui.label(RichText::new(name).strong().size(12.).color(rgb(t.accent)));
                    if e.redacted {
                        ui.label(RichText::new("Messaggio eliminato").italics().color(rgb(t.secondary)));
                    } else {
                        ui.label(&e.body);
                    }
                });
            });
            ui.separator();
        }
    });
    send
}
