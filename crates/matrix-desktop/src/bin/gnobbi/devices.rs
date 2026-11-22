//! «Dispositivi» drawer, its dialogs and the verification popup, driven by the toolkit-free `DeviceVerification`.
use crate::list::rgb;
use matrix_desktop::i18n;
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

/// One device: icon, name (and «Non verificato» badge), «Verificato · Ultima attività · IP», the id in monospace; `actions` on the right.
fn row(ui: &mut egui::Ui, d: &matrix_engine::verification::VerificationDevice, t: &DesignTokens, actions: impl FnOnce(&mut egui::Ui)) {
    let secondary = rgb(t.secondary);
    let mut meta = vec![if d.verified { i18n::t("Verified").to_owned() } else { String::new() }];
    meta.extend(d.last_seen_ts.map(|ms| i18n::t("Last active {when}").replace("{when}", &matrix_desktop::time_fmt::day_or_time(ms))));
    let meta = meta.into_iter().filter(|m| !m.is_empty()).collect::<Vec<_>>().join(" · ");
    let m = crate::metrics::get();
    let mut lines = vec![crate::rows::Line { tag: (!d.verified).then_some((i18n::t("Not verified"), rgb(t.error), rgb(t.on_accent))), ..crate::rows::Line::new(device_title(d), m.body + 1., rgb(t.text)) }];
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
    let closed = crate::rail::drawer_back(ctx, rail.panel, "devices-drawer", i18n::t("Devices"), rail.devices, t, Some(&mut back), |ui, rect| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.;
                    if let Some(own) = v.own().cloned() {
                        crate::settings::section(ui, i18n::t("This device"), t);
                        row(ui, &own, t, |ui| {
                            let rename = matrix_desktop::i18n::t("Rename");
                            if crate::icons::button(ui, crate::icons::Icon::PencilSimple, false, rename).on_hover_text(rename).clicked() {
                                v.open_rename();
                            }
                        });
                    }
                    crate::settings::section(ui, i18n::t("Keys"), t);
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 110.).max(80.);
                        ui.add(crate::metrics::line(&mut f.export_pass).password(true).desired_width(width).hint_text(i18n::t("Export password")));
                        if ui.button(i18n::t("Export keys")).clicked() && !f.export_pass.is_empty() {
                            let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
                            let path = std::path::Path::new(&home).join("Downloads").join(i18n::t("gnobbi-keys.txt"));
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
                                (false, Some(p)) => ui.label(i18n::t("{text} in {path}").replace("{text}", text).replace("{path}", &p.display().to_string())),
                                (true, _) => ui.colored_label(ui.visuals().error_fg_color, text.as_str()),
                                _ => ui.label(text.as_str()),
                            };
                        }
                    }
                    if ui.button(i18n::t("Regenerate recovery key")).clicked() {
                        v.keys = Some(matrix_desktop::key_export::KeysPage::new(matrix_desktop::key_export::KeysKind::Reset));
                        v.reset_recovery(None);
                    }
                    if v.offers_recover_keys(true) {
                        crate::settings::section(ui, i18n::t("Recover keys"), t);
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 80.).max(80.);
                            ui.add(crate::metrics::line(&mut f.key).password(true).desired_width(width).hint_text(i18n::t("Recovery key")));
                            if ui.button(i18n::t("Recover")).clicked() && !f.key.trim().is_empty() {
                                v.recover_keys(zeroize::Zeroizing::new(std::mem::take(&mut f.key)));
                            }
                        });
                        if let Some(message) = v.key_recovery_message() {
                            ui.colored_label(ui.visuals().error_fg_color, message);
                        }
                    }
                    crate::settings::section(ui, i18n::t("Other devices"), t);
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
    egui::Window::new(view.title).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_min_width(crate::dir::fit(ui, 320.));
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
                        ui.with_layout(crate::dir::col(), |ui| {
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
    }));
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
        egui::Window::new(i18n::t("Sign out")).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.label(counter);
            if let Some(n) = note {
                ui.label(n);
            }
            let r = ui.add(crate::metrics::line(&mut f.password).password(true).hint_text(i18n::t("Password")));
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(enabled && !f.password.is_empty(), egui::Button::new(i18n::t("Sign out"))).clicked()
                    || (r.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) && enabled && !f.password.is_empty())
                {
                    go = true;
                }
                if ui.button(i18n::t("Cancel")).clicked() {
                    cancel = true;
                }
            });
        }));
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
        egui::Window::new(i18n::t("Rename")).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.add(crate::metrics::line(&mut f.rename).hint_text(i18n::t("Device name")));
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                go = ui.add_enabled(!busy && !f.rename.trim().is_empty(), egui::Button::new(i18n::t("Save"))).clicked();
                cancel = ui.button(i18n::t("Cancel")).clicked();
            });
        }));
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
    Recover(zeroize::Zeroizing<String>),
    NotNow,
    Never,
}

/// The first-sign-in question: no recovery key → «Crea chiave di recupero» (nothing is written until then);
/// a key exists but this device lacks it → «Inserisci chiave di recupero» (never Create: it would replace the key).
pub fn offer(ctx: &egui::Context, v: &DeviceVerification, suppressed: bool, f: &mut Fields) -> Option<Offer> {
    use matrix_desktop::key_export::*;
    use matrix_engine::signin::RecoveryState;
    let kind = v.recovery_offer(suppressed)?;
    let enter = kind == OfferKind::Enter;
    let mut answer = None;
    egui::Window::new(i18n::t("Recovery key")).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_max_width(crate::dir::fit(ui, 380.));
        ui.label(if enter { i18n::t("This device does not have the account's recovery keys: enter the key to read old encrypted messages") } else { i18n::t("This account has no recovery key: without one, encrypted messages are lost with the devices") });
        ui.add_space(8.);
        let busy = v.key_recovery() == Some(RecoveryState::Pending);
        if enter {
            let eye = egui::Id::new("offer-key-eye");
            let shown = ui.data(|d| d.get_temp::<bool>(eye)).unwrap_or(false);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - crate::metrics::button() - 8.).max(80.);
                ui.add_enabled(!busy, crate::metrics::line(&mut f.key).password(!shown).desired_width(width).hint_text(i18n::t("Recovery key")));
                let (icon, name) = if shown { (crate::icons::Icon::EyeSlash, i18n::t("Hide password")) } else { (crate::icons::Icon::Eye, i18n::t("Show password")) };
                if crate::icons::button(ui, icon, false, name).on_hover_text(name).clicked() {
                    ui.data_mut(|d| d.insert_temp(eye, !shown));
                }
            });
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n::t("Restoring…"));
                });
            } else if let Some(message) = v.key_recovery_message() {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
            ui.add_space(4.);
        }
        ui.horizontal_wrapped(|ui| {
            if enter {
                if ui.add_enabled(!busy, egui::Button::new(i18n::t_ctx("menu", "Paste"))).clicked()
                    && let Some(text) = clipboard_text()
                {
                    f.key = text.trim().to_owned();
                }
                if ui.add_enabled(!busy && !f.key.trim().is_empty(), egui::Button::new(i18n::t("Restore"))).clicked() {
                    answer = Some(Offer::Recover(zeroize::Zeroizing::new(std::mem::take(&mut f.key))));
                }
            } else if ui.button(i18n::t("Create recovery key")).clicked() {
                answer = Some(Offer::Create);
            }
            // Not while restoring: «Annulla»/«Non ora» would hide the progress and the outcome.
            if ui.add_enabled(!busy, egui::Button::new(if enter { i18n::t("Cancel") } else { i18n::t("Not now") })).clicked() {
                answer = Some(Offer::NotNow);
            }
            if ui.add_enabled(!busy, egui::Button::new(i18n::t("Don't ask again for this account"))).clicked() {
                answer = Some(Offer::Never);
            }
        });
    }));
    if matches!(answer, Some(Offer::NotNow | Offer::Never)) {
        f.key.clear();
    }
    answer
}

/// The one view of a freshly created recovery key (here and in the sign-in step): readable, selectable,
/// «Copia» (clipboard wiped after `CLIPBOARD_SECONDS`) and «Salva su file»; either one sets `kept`.
pub fn key_view(ui: &mut egui::Ui, key: &str, kept: &mut bool) {
    use matrix_desktop::key_export::*;
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(RichText::new(key).monospace().size(15.)).selectable(true));
        if crate::info::copy_icon(ui).on_hover_text(i18n::t("Copy recovery key")).clicked() {
            crate::secret_clip::copy(ui.ctx(), key);
            *kept = true;
        }
    });
    ui.label(i18n::t("Save it now: it will not be shown again"));
    if ui.button(i18n::t("Save to file")).clicked()
        && let Some(path) = crate::pick::save_as(i18n::t("gnobbi-recovery-key.txt"))
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
    egui::Window::new(i18n::t("New recovery key")).order(egui::Order::Tooltip).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_max_width(crate::dir::fit(ui, 380.));
        let Some(key) = &key else {
            if page.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n::t("Regenerating…"));
                });
            } else if let Some((text, _)) = &page.line {
                ui.colored_label(ui.visuals().error_fg_color, text.as_str());
                close = ui.button(i18n::t("Close")).clicked();
            }
            return;
        };
        key_view(ui, key, &mut page.kept);
        ui.horizontal(|ui| {
            ui.checkbox(&mut page.kept, i18n::t("I kept it elsewhere"));
            close = ui.add_enabled(page.kept, egui::Button::new(i18n::t("Done"))).clicked();
        });
    }));
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

/// iOS has no arboard (the system paste menu fills the field instead).
fn clipboard_text() -> Option<String> {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    return arboard::Clipboard::new().and_then(|mut b| b.get_text()).ok();
    #[cfg(any(target_os = "ios", target_os = "android"))]
    None
}
