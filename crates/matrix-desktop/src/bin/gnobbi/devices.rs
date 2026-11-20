//! «Dispositivi» window and the verification popup, driven by the toolkit-free `DeviceVerification`.
use eframe::egui::{self, RichText};
use matrix_desktop::{DesignTokens, DeviceVerification, PopupBody, PopupButton, device_detail, device_title};

/// Text the user is typing in the dialogs of this window; the secrets are wiped as soon as they are handed over.
#[derive(Default)]
pub struct Fields {
    password: String,
    rename: String,
    key: String,
    export_pass: String,
    export_path: Option<std::path::PathBuf>,
}

pub fn show(ctx: &egui::Context, v: &mut DeviceVerification, open: &mut bool, f: &mut Fields, t: &DesignTokens) {
    egui::Window::new("Dispositivi").open(open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(380.);
        let secondary = egui::Color32::from_rgb((t.secondary >> 16) as u8, (t.secondary >> 8) as u8, t.secondary as u8);
        let time = |ms: u64| matrix_desktop::time_fmt::day_or_time(ms);
        if let Some(own) = v.own() {
            ui.label(RichText::new("Questo dispositivo").strong());
            ui.label(device_title(own));
            ui.label(RichText::new(device_detail(own, time)).color(secondary));
            if ui.button("Rinomina").clicked() {
                v.open_rename();
            }
            ui.separator();
        }
        ui.label(RichText::new("Chiavi").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut f.export_pass).password(true).hint_text("Password dell'export"));
            if ui.button("Esporta chiavi").clicked() && !f.export_pass.is_empty() {
                let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
                let path = std::path::Path::new(&home).join("Downloads").join("gnobbi-chiavi.txt");
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                f.export_path = Some(path.clone());
                let pass = zeroize::Zeroizing::new(std::mem::take(&mut f.export_pass));
                // The engine reauthenticates legacy profiles by phrase; this window has no phrase field, so it answers «Frase di sblocco errata» there.
                start_export(v, path, &pass);
            }
        });
        if let Some(page) = &mut v.keys {
            // The refill copy is for the GPUI fields; here nothing refills, so nothing is kept.
            page.held = None;
            if let Some((text, error)) = &page.line {
                match (error, &f.export_path) {
                    (false, Some(p)) => ui.label(format!("{text} in {}", p.display())),
                    (true, _) => ui.colored_label(ui.visuals().error_fg_color, text.as_str()),
                    _ => ui.label(text.as_str()),
                };
            }
        }
        if ui.button("Rigenera chiave di recupero").clicked() {
            v.keys = Some(matrix_desktop::key_export::KeysPage::new(matrix_desktop::key_export::KeysKind::Reset));
            v.reset_recovery(None);
        }
        ui.separator();
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
        // Only while the list has not arrived: an account with a single session has no other devices.
        if rows.is_empty() && v.own().is_none() {
            ui.spinner();
        }
        egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
            let mut begin = None;
            let mut delete = None;
            for (i, (title, detail, verified)) in rows.iter().enumerate() {
                // Buttons under the text: the detail line (time, IP, id) is long and would run under them.
                ui.label(title);
                ui.label(RichText::new(detail).size(12.).color(secondary));
                ui.horizontal(|ui| {
                    if !verified && ui.button("Verifica").clicked() {
                        begin = Some(i);
                    }
                    if ui.button("Disconnetti").clicked() {
                        delete = Some(i);
                    }
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

/// Opens the export page and queues the export, or leaves the password error on the page.
pub fn start_export(v: &mut DeviceVerification, path: std::path::PathBuf, pass: &zeroize::Zeroizing<String>) {
    use matrix_desktop::key_export::{KeysKind, KeysPage, check_password};
    let mut page = KeysPage::new(KeysKind::Export);
    let check = check_password(pass, pass);
    if let Err(error) = check {
        page.line = Some((error.text().into(), true));
    }
    v.keys = Some(page);
    if check.is_ok() {
        v.export_keys(path, pass, None);
    }
}

/// The new recovery key, shown once: readable, selectable and with the small copy icon; closing wipes it.
pub fn new_key(ctx: &egui::Context, v: &mut DeviceVerification) {
    let Some(key) = v.keys.as_ref().and_then(|p| p.secret.as_ref()).map(|s| s.expose().to_owned()) else { return };
    let mut close = false;
    egui::Window::new("Nuova chiave di recupero").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_max_width(380.);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&key).monospace().size(15.));
            if crate::info::copy_icon(ui).clicked() {
                ctx.copy_text(key.clone());
            }
        });
        ui.add_space(8.);
        if ui.button("Fatto").clicked() {
            close = true;
        }
    });
    if close {
        v.confirm_key_saved();
    }
}
