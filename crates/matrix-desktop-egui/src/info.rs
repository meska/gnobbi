//! Right panel: the open room's facts and its members.
use eframe::egui::{self, Align2, Color32, FontId, RichText, Vec2};
use matrix_desktop::{DesignTokens, SidebarRoom};
use matrix_engine::operations::Member;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn show(ui: &mut egui::Ui, room: &SidebarRoom, members: Option<&[Member]>, t: &DesignTokens) {
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
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), egui::Sense::hover());
            let c = egui::pos2(rect.left() + 28., rect.center().y);
            let initials: String = m.name.trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
            ui.painter().circle_filled(c, 14., rgb(matrix_desktop::AVATAR_FALLBACK));
            ui.painter().text(c, Align2::CENTER_CENTER, initials, FontId::proportional(12.), rgb(t.on_accent));
            let galley = ui.painter().layout(m.name.clone(), FontId::proportional(14.), rgb(t.text), (rect.width() - 60.).max(20.));
            ui.painter().galley(egui::pos2(rect.left() + 52., rect.center().y - galley.size().y / 2.), galley, rgb(t.text));
        }
    });
}
