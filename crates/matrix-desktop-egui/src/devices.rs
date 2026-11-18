//! «Dispositivi» window and the verification popup, driven by the toolkit-free `DeviceVerification`.
use eframe::egui::{self, RichText};
use matrix_desktop::{DesignTokens, DeviceVerification, PopupBody, PopupButton, device_detail, device_title};

/// Text the user is typing in the dialogs of this window; the secrets are wiped as soon as they are handed over.
#[derive(Default)]
pub struct Fields {
    password: String,
    rename: String,
    key: String,
    name: String,
    seeded: bool,
}

pub fn show(ctx: &egui::Context, v: &mut DeviceVerification, open: &mut bool, f: &mut Fields, t: &DesignTokens) {
    egui::Window::new("Dispositivi").open(open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.set_min_width(380.);
        let secondary = egui::Color32::from_rgb((t.secondary >> 16) as u8, (t.secondary >> 8) as u8, t.secondary as u8);
        let time = |ms: u64| matrix_desktop::time_fmt::day_or_time(ms);
        if let Some((id, shown)) = v.profile().cloned() {
            if !f.seeded {
                f.name = shown.clone().unwrap_or_default();
                f.seeded = true;
            }
            ui.label(RichText::new("Profilo").strong());
            ui.label(RichText::new(id).color(secondary));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut f.name).hint_text("Nome visualizzato"));
                let changed = f.name.trim() != shown.as_deref().unwrap_or("") && !f.name.trim().is_empty();
                if ui.add_enabled(changed, egui::Button::new("Salva")).clicked() {
                    v.save_profile(Some(f.name.trim().to_owned()), matrix_engine::verification::ProfileAvatar::Keep);
                }
                if v.profile_avatar().is_some() && ui.button("Rimuovi foto").clicked() {
                    v.save_profile(None, matrix_engine::verification::ProfileAvatar::Remove);
                }
            });
            ui.label(RichText::new("Trascina un'immagine qui per la foto").size(12.).color(secondary));
            ui.separator();
        }
        if let Some(own) = v.own() {
            ui.label(RichText::new("Questo dispositivo").strong());
            ui.label(device_title(own));
            ui.label(RichText::new(device_detail(own, time)).color(secondary));
            if ui.button("Rinomina").clicked() {
                v.open_rename();
            }
            ui.separator();
        }
        if v.offers_recover_keys(true) {
            ui.label(RichText::new("Recupera chiavi").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut f.key).password(true).hint_text("Chiave di recupero"));
                if ui.button("Recupera").clicked() && !f.key.trim().is_empty() {
                    v.recover_keys(zeroize::Zeroizing::new(std::mem::take(&mut f.key)));
                }
            });
            if let Some(message) = v.key_recovery_message() {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
            ui.separator();
        }
        let rows: Vec<(String, String, bool)> = v.devices().iter().map(|d| (device_title(d).to_owned(), device_detail(d, time), d.verified)).collect();
        if rows.is_empty() {
            ui.spinner();
        }
        egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
            let mut begin = None;
            let mut delete = None;
            for (i, (title, detail, verified)) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(title);
                        ui.label(RichText::new(detail).size(12.).color(secondary));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Disconnetti").clicked() {
                            delete = Some(i);
                        }
                        if !verified && ui.button("Verifica").clicked() {
                            begin = Some(i);
                        }
                    });
                });
                ui.separator();
            }
            if let Some(i) = begin {
                v.begin(i);
            }
            if let Some(i) = delete {
                v.open_delete(i);
            }
        });
    });
}

/// The verification popup while a flow is open.
pub fn popup(ctx: &egui::Context, v: &mut DeviceVerification) {
    let Some(view) = v.popup_view() else { return };
    let mut pressed = None;
    egui::Window::new(view.title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        match &view.body {
            PopupBody::Waiting => {
                ui.spinner();
            }
            PopupBody::Empty => {}
            PopupBody::Request(who) => {
                ui.label(who);
            }
            PopupBody::Emoji(list) => {
                ui.horizontal_wrapped(|ui| {
                    for (symbol, name) in list {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(symbol).size(30.));
                            ui.label(RichText::new(name).size(11.));
                        });
                    }
                });
            }
            PopupBody::Decimals((a, b, c)) => {
                ui.label(RichText::new(format!("{a}  {b}  {c}")).size(26.));
            }
            PopupBody::Outcome(text) => {
                ui.label(*text);
            }
        }
        ui.add_space(8.);
        ui.horizontal(|ui| {
            for button in &view.buttons {
                if ui.button(button.label()).clicked() {
                    pressed = Some(*button);
                }
            }
        });
    });
    if let Some(button) = pressed {
        v.popup_press(button);
    } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        v.popup_press(PopupButton::Close);
    }
}

/// «Disconnetti» (needs the account password) and «Rinomina» dialogs.
pub fn dialogs(ctx: &egui::Context, v: &mut DeviceVerification, f: &mut Fields) {
    if let Some(dialog) = v.delete_dialog() {
        let (counter, note, error, enabled) = (dialog.counter(), dialog.unknown_note(), dialog.error, dialog.confirm_enabled());
        let (mut go, mut cancel) = (false, false);
        egui::Window::new("Disconnetti").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
            ui.label(counter);
            if let Some(n) = note {
                ui.label(n);
            }
            let r = ui.add(egui::TextEdit::singleline(&mut f.password).password(true).hint_text("Password"));
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(enabled && !f.password.is_empty(), egui::Button::new("Disconnetti")).clicked()
                    || (r.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) && enabled && !f.password.is_empty())
                {
                    go = true;
                }
                if ui.button("Annulla").clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            v.submit_delete(zeroize::Zeroizing::new(std::mem::take(&mut f.password)));
        }
        if cancel {
            v.cancel_delete();
            zeroize::Zeroize::zeroize(&mut f.password);
        }
    }
    if let Some(dialog) = v.rename_dialog() {
        let (busy, error) = (dialog.busy, dialog.error);
        let (mut go, mut cancel) = (false, false);
        egui::Window::new("Rinomina").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut f.rename).hint_text("Nome del dispositivo"));
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                go = ui.add_enabled(!busy && !f.rename.trim().is_empty(), egui::Button::new("Salva")).clicked();
                cancel = ui.button("Annulla").clicked();
            });
        });
        if go {
            v.submit_rename(f.rename.trim());
        }
        if cancel {
            v.cancel_rename();
        }
    }
}
