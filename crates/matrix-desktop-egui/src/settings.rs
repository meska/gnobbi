//! Settings window: the saved preferences the GPUI app offers for look and behaviour. Every change is written at once.
use eframe::egui::{self, Color32, Sense, Vec2};
use matrix_desktop::{Accent, AppDirs, AutoNight, DesignTokens, IconChoice, Preferences, Theme};

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The shared switch: fixed positive label, track on the right, on means active.
pub fn switch(ui: &mut egui::Ui, label: &str, on: &mut bool, t: &DesignTokens) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(36., 20.), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                *on = !*on;
                changed = true;
            }
            let (track, knob) = if *on { (rgb(t.accent), 26.) } else { (rgb(t.disabled), 10.) };
            ui.painter().rect_filled(rect, 10., track);
            ui.painter().circle_filled(egui::pos2(rect.left() + knob, rect.center().y), 8., rgb(t.on_accent));
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Switch, true, *on, label));
        });
    });
    changed
}

/// Draws the window; returns the new preferences when something was changed (already saved).
pub fn show(ctx: &egui::Context, open: &mut bool, prefs: &Preferences, dirs: Option<&AppDirs>, t: &DesignTokens) -> Option<Preferences> {
    let mut p = prefs.clone();
    let mut changed = false;
    egui::Window::new("Impostazioni").open(open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        let theme_name = p.theme.map_or("Sistema", Theme::label);
        egui::ComboBox::from_label("Tema").selected_text(theme_name).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut p.theme, None, "Sistema").changed();
            for th in Theme::ALL {
                changed |= ui.selectable_value(&mut p.theme, Some(th), th.label()).changed();
            }
        });
        let accent_name = p.accent.map_or("Sistema", Accent::label);
        egui::ComboBox::from_label("Accento").selected_text(accent_name).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut p.accent, None, "Sistema").changed();
            for a in Accent::ALL {
                changed |= ui.selectable_value(&mut p.accent, Some(a), a.label()).changed();
            }
        });
        egui::ComboBox::from_label("Icona").selected_text(p.icon.label()).show_ui(ui, |ui| {
            for i in IconChoice::ALL {
                changed |= ui.selectable_value(&mut p.icon, i, i.label()).changed();
            }
        });
        egui::ComboBox::from_label("Notte automatica").selected_text(p.auto_night.label()).show_ui(ui, |ui| {
            for a in AutoNight::ALL {
                changed |= ui.selectable_value(&mut p.auto_night, a, a.label()).changed();
            }
        });
        ui.separator();
        changed |= switch(ui, "Conferme di lettura", &mut p.read_receipts, t);
        changed |= switch(ui, "Nascondi messaggi eliminati", &mut p.hide_redacted, t);
        changed |= switch(ui, "Anteprima dei messaggi", &mut p.message_preview, t);
        changed |= switch(ui, "Notifiche", &mut p.notifications, t);
        changed |= switch(ui, "Chiudi nella tray", &mut p.close_to_tray, t);
        #[cfg(target_os = "linux")]
        {
            let id = egui::Id::new("desktop-entry-result");
            if ui.button("Aggiungi al menu").clicked() {
                let installed = crate::desktop_entry::install(p.icon).is_ok();
                let opted_in = installed && (!p.desktop_entry_dont_ask || dirs.is_some_and(|d| Preferences::update(d, |saved| saved.desktop_entry_dont_ask = false).is_ok()));
                ui.ctx().data_mut(|data| data.insert_temp(id, opted_in));
                if opted_in && p.desktop_entry_dont_ask {
                    p.desktop_entry_dont_ask = false;
                    changed = true;
                }
            }
            if let Some(ok) = ui.ctx().data(|data| data.get_temp::<bool>(id)) {
                ui.label(if ok { "Aggiunto al menu" } else { "Impossibile aggiungere al menu" });
            }
        }
    });
    if !changed {
        return None;
    }
    if let Some(dirs) = dirs {
        // Field by field, so a setting this window does not know is never rewritten.
        let new = p.clone();
        let _ = Preferences::update(dirs, |saved| {
            saved.theme = new.theme;
            saved.accent = new.accent;
            saved.auto_night = new.auto_night;
            saved.icon = new.icon;
            saved.read_receipts = new.read_receipts;
            saved.hide_redacted = new.hide_redacted;
            saved.message_preview = new.message_preview;
            saved.notifications = new.notifications;
            saved.close_to_tray = new.close_to_tray;
        });
    }
    Some(p)
}
