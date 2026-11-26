//! Settings drawer: the saved preferences the GPUI app offers for look and behaviour. Every change is written at once.
use crate::list::rgb;
use eframe::egui::{self, Color32, Sense, Vec2};
use matrix_desktop::i18n::t as tr;
use matrix_desktop::{AppDirs, DesignTokens, Preferences};

/// The shared switch: fixed positive label, track on the right, on means active.
pub fn switch(ui: &mut egui::Ui, label: &str, on: &mut bool, t: &DesignTokens) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
            changed = toggle(ui, label, on, t);
            wrapped_label(ui, label);
        });
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
    ui.painter().circle_filled(crate::dir::flip_pos(rect, egui::pos2(rect.left() + knob, rect.center().y)), 8., rgb(t.on_accent));
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.), 12., egui::Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Switch, true, *on, label));
    resp.clicked()
}

/// Section title of the drawer.
pub fn section(ui: &mut egui::Ui, title: &str, t: &DesignTokens) {
    ui.add_space(14.);
    ui.label(egui::RichText::new(title).size(13.).strong().color(rgb(t.accent)));
}

/// A row that opens another drawer: icon, label, «›» on the right.
fn nav_row(ui: &mut egui::Ui, label: &str, icon: crate::icons::Icon, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), crate::metrics::button() + 8.), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, 8., rgb(t.input));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, 8., egui::Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Inside);
    }
    let flip = |p| crate::dir::flip_pos(rect, p);
    crate::icons::paint(ui.painter(), flip(rect.left_center() + Vec2::new(20., 0.)), icon, false, crate::metrics::get().icon + 4., rgb(t.secondary));
    ui.painter().text(flip(rect.left_center() + Vec2::new(44., 0.)), crate::dir::align(egui::Align2::LEFT_CENTER), label, egui::FontId::proportional(14.), rgb(t.text));
    crate::icons::paint(ui.painter(), flip(rect.right_center() - Vec2::new(16., 0.)), crate::icons::Icon::CaretRight, false, crate::metrics::get().icon - 2., rgb(t.secondary));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp
}

/// The row label, placed after the control so that it wraps in the width the control leaves (long translations).
fn wrapped_label(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.with_layout(crate::dir::row(egui::Align::Center), |ui| ui.add(egui::Label::new(label).wrap())).inner
}

/// One choice among a few: fixed label, the list on the right.
pub fn choice(ui: &mut egui::Ui, label: &str, selected: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
            let combo = crate::metrics::clickable(egui::ComboBox::from_id_salt(label).selected_text(selected).width(150.).show_ui(ui, |ui| crate::dir::scope(ui, add)).response);
            combo.labelled_by(wrapped_label(ui, label).id);
        });
    });
}

/// One line: «Update available · v» with «Update», or «Updating…», or the error line. Nothing when idle.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn update_row(ui: &mut egui::Ui, dirs: Option<&AppDirs>) {
    use matrix_desktop::update::{self, Status};
    let status = update::status();
    match status {
        Status::Idle => {}
        Status::Available { version } => {
            ui.horizontal(|ui| {
                ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
                    if ui.button(tr("Update")).clicked()
                        && let Some(d) = dirs
                    {
                        update::start_update(d.clone(), || matrix_engine::unsent_messages().map(|n| n > 0));
                    }
                    ui.add(egui::Label::new(update::indicator_row(&version)).wrap());
                });
            });
        }
        Status::Working => {
            ui.label(tr("Updating…"));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500)); // the worker thread cannot wake the UI
        }
        Status::Error(line) => {
            ui.label(line);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
    }
}

/// Draws the drawer beside the rail; returns the new preferences when something was changed (already saved).
pub fn show(ctx: &egui::Context, rail: &mut crate::rail::State, prefs: &Preferences, several_accounts: bool, dirs: Option<&AppDirs>, t: &DesignTokens) -> Option<Preferences> {
    let mut p = prefs.clone();
    let mut changed = false;
    let (mut devices, mut accounts, mut customize, mut feedback) = (false, false, false, false);
    // One column: the pages that return here («‹») slide over this one, which stays under them.
    let under = crate::layout::compact(ctx) && (rail.accounts || rail.devices || rail.customize);
    let closed = crate::rail::drawer(ctx, rail.panel, "settings-drawer", tr("Settings"), rail.settings || under, t, |ui, rect| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.;
                    section(ui, tr("Account and security"), t);
                    accounts = nav_row(ui, tr("Account"), crate::icons::Icon::Users, t).clicked();
                    devices = nav_row(ui, tr("Devices"), crate::icons::Icon::Desktop, t).clicked();
                    customize = nav_row(ui, tr("Customize"), crate::icons::Icon::Palette, t).clicked();
                    section(ui, tr("Chat"), t);
                    changed |= switch(ui, tr("Read receipts"), &mut p.read_receipts, t);
                    changed |= switch(ui, tr("Hide deleted messages"), &mut p.hide_redacted, t);
                    changed |= switch(ui, tr("Show status messages"), &mut p.show_status, t);
                    changed |= switch(ui, tr("Message preview"), &mut p.message_preview, t);
                    if several_accounts {
                        changed |= switch(ui, tr("One list for all accounts"), &mut p.unified_list, t);
                    }
                    section(ui, tr("Notifications"), t);
                    changed |= switch(ui, tr("Notifications"), &mut p.notifications, t);
                    // iOS has no tray (nor other desktop-only system rows): no System section.
                    #[cfg(not(target_os = "ios"))]
                    {
                        section(ui, tr("System"), t);
                        changed |= switch(ui, tr("Close to tray"), &mut p.close_to_tray, t);
                    }
                    // The updater engine exists on Linux and Windows only.
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    {
                        changed |= switch(ui, tr("Automatic updates"), &mut p.update_check_enabled, t);
                        update_row(ui, dirs);
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let id = egui::Id::new("desktop-entry-result");
                        if ui.button(tr("Add to menu")).clicked() {
                            let installed = crate::desktop_entry::install(p.icon).is_ok();
                            let opted_in = installed && (!p.desktop_entry_dont_ask || dirs.is_some_and(|d| Preferences::update(d, |saved| saved.desktop_entry_dont_ask = false).is_ok()));
                            ui.ctx().data_mut(|data| data.insert_temp(id, opted_in));
                            if opted_in && p.desktop_entry_dont_ask {
                                p.desktop_entry_dont_ask = false;
                                changed = true;
                            }
                        }
                        if let Some(ok) = ui.ctx().data(|data| data.get_temp::<bool>(id)) {
                            ui.label(if ok { tr("Added to menu") } else { tr("Could not add to menu") });
                        }
                    }
                    feedback = nav_row(ui, tr("Send feedback"), crate::icons::Icon::ChatCircleDots, t).clicked();
                });
            });
        });
    });
    rail.feedback |= feedback;
    if closed {
        rail.settings = false;
    }
    if devices {
        rail.set_devices(true);
    }
    if accounts {
        rail.set_accounts(true);
    }
    if customize {
        rail.set_customize(true);
    }
    if !changed {
        return None;
    }
    persist(dirs, &p);
    Some(p)
}

/// Saves what the settings drawers change, field by field, so a setting they do not know is never rewritten.
pub fn persist(dirs: Option<&AppDirs>, p: &Preferences) {
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
            saved.show_status = new.show_status;
            saved.message_preview = new.message_preview;
            saved.unified_list = new.unified_list;
            saved.notifications = new.notifications;
            saved.close_to_tray = new.close_to_tray;
            saved.update_check_enabled = new.update_check_enabled;
            saved.language = new.language;
            saved.text_size = new.text_size;
            saved.density = new.density;
        });
    }
}
