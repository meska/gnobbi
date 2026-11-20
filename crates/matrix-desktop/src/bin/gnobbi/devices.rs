//! «Dispositivi» drawer, its dialogs and the verification popup, driven by the toolkit-free `DeviceVerification`.
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

/// Platform of a device, guessed from its name; `Other` when the name says nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Desktop,
    Phone,
    Web,
    Other,
}

pub fn kind_of(name: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["ios", "iphone", "ipad", "android"]) {
        Kind::Phone
    } else if has(&["web", "firefox", "chrome", "safari", "edge", "browser"]) {
        Kind::Web
    } else if has(&["mac", "linux", "windows", "desktop", "gnobbi", "element"]) {
        Kind::Desktop
    } else {
        Kind::Other
    }
}

/// Icon of the platform, centred in `r`.
pub fn icon(p: &egui::Painter, r: egui::Rect, kind: Kind, ink: egui::Color32) {
    use crate::icons::Icon;
    let icon = match kind {
        Kind::Desktop => Icon::Desktop,
        Kind::Phone => Icon::DeviceMobile,
        Kind::Web => Icon::Globe,
        Kind::Other => Icon::Question,
    };
    crate::icons::paint(p, r.center(), icon, false, crate::metrics::get().icon + 4., ink);
}

fn rgb(c: u32) -> egui::Color32 {
    egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// One device: icon, name (and «Non verificato» badge), «Verificato · Ultima attività · IP», the id in monospace; `actions` on the right.
fn row(ui: &mut egui::Ui, d: &matrix_engine::verification::VerificationDevice, t: &DesignTokens, actions: impl FnOnce(&mut egui::Ui)) {
    let secondary = rgb(t.secondary);
    let mut meta = vec![if d.verified { "Verificato".to_owned() } else { String::new() }];
    meta.extend(d.last_seen_ts.map(|ms| format!("Ultima attività {}", matrix_desktop::time_fmt::day_or_time(ms))));
    let meta = meta.into_iter().filter(|m| !m.is_empty()).collect::<Vec<_>>().join(" · ");
    let m = crate::metrics::get();
    let mut lines = vec![crate::rows::Line { tag: (!d.verified).then_some(("Non verificato", rgb(t.error), rgb(t.on_accent))), ..crate::rows::Line::new(device_title(d), m.body + 1., rgb(t.text)) }];
    if !meta.is_empty() {
        lines.push(crate::rows::Line::new(&meta, m.secondary, secondary));
    }
    if let Some(ip) = &d.last_seen_ip {
        lines.push(crate::rows::Line::new(ip, m.secondary, secondary));
    }
    if d.display_name.is_some() {
        lines.push(crate::rows::Line { mono: true, ..crate::rows::Line::new(&d.device_id, m.secondary - 1., secondary) });
    }
    crate::rows::lines_row(
        ui,
        |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(28., 28.), egui::Sense::hover());
            icon(ui.painter(), r, kind_of(device_title(d)), secondary);
        },
        &lines,
        actions,
    );
}

/// Whether a dialog of this page is open (Esc belongs to it, not to the drawer).
pub fn modal_open(v: &DeviceVerification) -> bool {
    v.delete_dialog().is_some() || v.rename_dialog().is_some() || v.popup_view().is_some() || v.keys.as_ref().is_some_and(|p| p.secret.is_some())
}

/// The «Dispositivi» drawer, opened from «Impostazioni»; «‹» goes back there.
pub fn show(ctx: &egui::Context, v: &mut DeviceVerification, rail: &mut crate::rail::State, f: &mut Fields, t: &DesignTokens) {
    let (mut back, mut begin, mut delete) = (false, None, None);
    let closed = crate::rail::drawer_back(ctx, rail.panel, "devices-drawer", "Dispositivi", rail.devices, t, Some(&mut back), |ui, rect| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.;
                    if let Some(own) = v.own().cloned() {
                        crate::settings::section(ui, "Questo dispositivo", t);
                        row(ui, &own, t, |ui| {
                            let rename = matrix_desktop::i18n::t("Rename");
                            if crate::icons::button(ui, crate::icons::Icon::PencilSimple, false, rename).on_hover_text(rename).clicked() {
                                v.open_rename();
                            }
                        });
                    }
                    crate::settings::section(ui, "Chiavi", t);
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 110.).max(80.);
                        ui.add(crate::metrics::line(&mut f.export_pass).password(true).desired_width(width).hint_text("Password dell'export"));
                        if ui.button("Esporta chiavi").clicked() && !f.export_pass.is_empty() {
                            let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
                            let path = std::path::Path::new(&home).join("Downloads").join("gnobbi-chiavi.txt");
                            let _ = std::fs::create_dir_all(path.parent().unwrap());
                            f.export_path = Some(path.clone());
                            let pass = zeroize::Zeroizing::new(std::mem::take(&mut f.export_pass));
                            // The engine reauthenticates legacy profiles by phrase; this page has no phrase field, so it answers «Frase di sblocco errata» there.
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
                    if v.offers_recover_keys(true) {
                        crate::settings::section(ui, "Recupera chiavi", t);
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 80.).max(80.);
                            ui.add(crate::metrics::line(&mut f.key).password(true).desired_width(width).hint_text("Chiave di recupero"));
                            if ui.button("Recupera").clicked() && !f.key.trim().is_empty() {
                                v.recover_keys(zeroize::Zeroizing::new(std::mem::take(&mut f.key)));
                            }
                        });
                        if let Some(message) = v.key_recovery_message() {
                            ui.colored_label(ui.visuals().error_fg_color, message);
                        }
                    }
                    crate::settings::section(ui, "Altri dispositivi", t);
                    let others = v.devices().to_vec();
                    // Only while the list has not arrived: an account with a single session has no other devices.
                    if others.is_empty() && v.own().is_none() {
                        ui.spinner();
                    }
                    for (i, d) in others.iter().enumerate() {
                        ui.push_id(&d.device_id, |ui| row(ui, d, t, |ui| {
                            use crate::icons::{self, Icon};
                            let disconnect = matrix_desktop::i18n::t("Disconnect");
                            if icons::button(ui, Icon::SignOut, false, disconnect).on_hover_text(disconnect).clicked() {
                                delete = Some(i);
                            }
                            if !d.verified {
                                let verify = matrix_desktop::i18n::t("Verify");
                                ui.scope(|ui| {
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color = rgb(t.accent);
                                    if icons::button(ui, Icon::ShieldCheck, false, verify).on_hover_text(verify).clicked() {
                                        begin = Some(i);
                                    }
                                });
                            }
                        }));
                    }
                });
            });
        });
    });
    if let Some(i) = begin {
        v.begin(i);
    }
    if let Some(i) = delete {
        v.open_delete(i);
    }
    if back {
        rail.set_settings(true);
    } else if closed {
        rail.devices = false;
    }
}

/// The verification popup while a flow is open.
pub fn popup(ctx: &egui::Context, v: &mut DeviceVerification) {
    let Some(view) = v.popup_view() else { return };
    let mut pressed = None;
    egui::Window::new(view.title).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
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
        egui::Window::new("Disconnetti").order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
            ui.label(counter);
            if let Some(n) = note {
                ui.label(n);
            }
            let r = ui.add(crate::metrics::line(&mut f.password).password(true).hint_text("Password"));
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
        egui::Window::new("Rinomina").order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
            ui.add(crate::metrics::line(&mut f.rename).hint_text("Nome del dispositivo"));
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

pub enum Offer {
    Create,
    NotNow,
    Never,
}

/// The first-sign-in question when the account has no recovery key; nothing is written until «Crea chiave di recupero».
pub fn offer(ctx: &egui::Context, v: &DeviceVerification, suppressed: bool) -> Option<Offer> {
    use matrix_desktop::key_export::*;
    if !v.offers_recovery_key(suppressed) {
        return None;
    }
    let mut answer = None;
    egui::Window::new(OFFER_TITLE).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_max_width(380.);
        ui.label(OFFER_TEXT);
        ui.add_space(8.);
        ui.horizontal_wrapped(|ui| {
            if ui.button(CREATE_KEY_LABEL).clicked() {
                answer = Some(Offer::Create);
            }
            if ui.button(NOT_NOW_LABEL).clicked() {
                answer = Some(Offer::NotNow);
            }
            if ui.button(NEVER_ASK_LABEL).clicked() {
                answer = Some(Offer::Never);
            }
        });
    });
    answer
}

/// The one view of a freshly created recovery key (here and in the sign-in step): readable, selectable,
/// «Copia» (clipboard wiped after `CLIPBOARD_SECONDS`) and «Salva su file»; either one sets `kept`.
pub fn key_view(ui: &mut egui::Ui, key: &str, kept: &mut bool) {
    use matrix_desktop::key_export::*;
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(RichText::new(key).monospace().size(15.)).selectable(true));
        if crate::info::copy_icon(ui).on_hover_text(COPY_KEY_LABEL).clicked() {
            crate::secret_clip::copy(ui.ctx(), key);
            *kept = true;
        }
    });
    ui.label(SAVE_WARNING);
    if ui.button(SAVE_FILE_LABEL).clicked()
        && let Some(path) = crate::pick::save_as(KEY_FILE_NAME)
        && write_key_file(&path, key).is_ok()
    {
        *kept = true;
        crate::pick::share(&path);
    }
}

/// The new recovery key, shown once: readable, selectable, with «Copia» and «Salva su file»; «Fatto» wipes it.
/// Also the progress and error of the creation while the Devices drawer may be closed.
pub fn new_key(ctx: &egui::Context, v: &mut DeviceVerification) {
    use matrix_desktop::key_export::*;
    let Some(page) = v.keys.as_mut().filter(|p| p.kind == KeysKind::Reset) else { return };
    let key = page.secret.as_ref().map(|s| s.expose().to_owned());
    if key.is_none() && !page.busy && page.line.is_none() {
        return;
    }
    let mut close = false;
    egui::Window::new("Nuova chiave di recupero").order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_max_width(380.);
        let Some(key) = &key else {
            if page.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RESET_RUNNING);
                });
            } else if let Some((text, _)) = &page.line {
                ui.colored_label(ui.visuals().error_fg_color, text.as_str());
                close = ui.button("Chiudi").clicked();
            }
            return;
        };
        key_view(ui, key, &mut page.kept);
        ui.horizontal(|ui| {
            ui.checkbox(&mut page.kept, KEPT_LABEL);
            close = ui.add_enabled(page.kept, egui::Button::new("Fatto")).clicked();
        });
    });
    if close {
        v.confirm_key_saved();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_comes_from_the_name() {
        assert_eq!(kind_of("Gnobbi (macOS)"), Kind::Desktop);
        assert_eq!(kind_of("Element iOS"), Kind::Phone);
        assert_eq!(kind_of("Firefox su Linux"), Kind::Web);
        assert_eq!(kind_of("ABCDEFGHIJ"), Kind::Other);
    }
}
