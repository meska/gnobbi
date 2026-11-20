//! Settings drawer: the saved preferences the GPUI app offers for look and behaviour. Every change is written at once.
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| changed = toggle(ui, label, on, t));
    });
    changed
}

/// The switch alone; `label` is what a screen reader says.
pub fn toggle(ui: &mut egui::Ui, label: &str, on: &mut bool, t: &DesignTokens) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(36., 20.), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
    let (track, knob) = if *on { (rgb(t.accent), 26.) } else { (rgb(t.disabled), 10.) };
    ui.painter().rect_filled(rect, 10., track);
    ui.painter().circle_filled(egui::pos2(rect.left() + knob, rect.center().y), 8., rgb(t.on_accent));
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.), 12., egui::Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Switch, true, *on, label));
    resp.clicked()
}

/// Section title of the drawer.
fn section(ui: &mut egui::Ui, title: &str, t: &DesignTokens) {
    ui.add_space(14.);
    ui.label(egui::RichText::new(title).size(13.).strong().color(rgb(t.accent)));
}

/// One choice among a few: fixed label, the list on the right.
fn choice(ui: &mut egui::Ui, label: &str, selected: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let name = ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt(label).selected_text(selected).width(150.).show_ui(ui, add).response.labelled_by(name.id);
        });
    });
}

/// Draws the drawer beside the rail; returns the new preferences when something was changed (already saved).
pub fn show(ctx: &egui::Context, rail: &mut crate::rail::State, prefs: &Preferences, dirs: Option<&AppDirs>, t: &DesignTokens) -> Option<Preferences> {
    let mut p = prefs.clone();
    let mut changed = false;
    let closed = crate::rail::drawer(ctx, rail.panel, "settings-drawer", "Impostazioni", rail.settings, t, |ui, rect| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 36.;
                    ui.spacing_mut().item_spacing.y = 6.;
                    section(ui, "Aspetto", t);
                    choice(ui, "Tema", p.theme.map_or("Sistema", Theme::label), |ui| {
                        changed |= ui.selectable_value(&mut p.theme, None, "Sistema").changed();
                        for th in Theme::ALL {
                            changed |= ui.selectable_value(&mut p.theme, Some(th), th.label()).changed();
                        }
                    });
                    choice(ui, "Accento", p.accent.map_or("Sistema", Accent::label), |ui| {
                        changed |= ui.selectable_value(&mut p.accent, None, "Sistema").changed();
                        for a in Accent::ALL {
                            changed |= ui.selectable_value(&mut p.accent, Some(a), a.label()).changed();
                        }
                    });
                    choice(ui, "Icona", p.icon.label(), |ui| {
                        for i in IconChoice::ALL {
                            changed |= ui.selectable_value(&mut p.icon, i, i.label()).changed();
                        }
                    });
                    choice(ui, "Notte automatica", p.auto_night.label(), |ui| {
                        for a in AutoNight::ALL {
                            changed |= ui.selectable_value(&mut p.auto_night, a, a.label()).changed();
                        }
                    });
                    section(ui, "Chat", t);
                    changed |= switch(ui, "Conferme di lettura", &mut p.read_receipts, t);
                    changed |= switch(ui, "Nascondi messaggi eliminati", &mut p.hide_redacted, t);
                    changed |= switch(ui, "Anteprima dei messaggi", &mut p.message_preview, t);
                    section(ui, "Notifiche", t);
                    changed |= switch(ui, "Notifiche", &mut p.notifications, t);
                    section(ui, "Sistema", t);
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
            });
        });
    });
    if closed {
        rail.settings = false;
    }
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
