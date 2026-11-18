//! «Account» window: the accounts of the catalog; each one runs in its own window (process), like the GPUI app's "Add account".
use eframe::egui::{self, RichText};
use matrix_desktop::{AccountCatalog, AppDirs, DesignTokens};
use std::path::PathBuf;

/// What the user asked for: open a profile folder in a new window.
pub enum Launch {
    Open(PathBuf),
    Add,
}

pub fn show(ctx: &egui::Context, open: &mut bool, dirs: &AppDirs, current: &std::path::Path, t: &DesignTokens) -> Option<Launch> {
    let mut out = None;
    let catalog = AccountCatalog::load(dirs);
    egui::Window::new("Account").open(open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(320.);
        let secondary = egui::Color32::from_rgb((t.secondary >> 16) as u8, (t.secondary >> 8) as u8, t.secondary as u8);
        for entry in catalog.accounts() {
            let dir = catalog.data_dir(dirs, entry.id);
            let active = dir.as_deref() == Some(current);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(&entry.label);
                    ui.label(RichText::new(entry.user_id.as_deref().unwrap_or("")).size(12.).color(secondary));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if active {
                        ui.label("Attivo");
                    } else if let Some(dir) = dir
                        && ui.button("Apri").clicked()
                    {
                        out = Some(Launch::Open(dir));
                    }
                });
            });
            ui.separator();
        }
        if ui.button("Aggiungi account").clicked() {
            out = Some(Launch::Add);
        }
    });
    out
}

/// Starts this app on `root` in a new process (its own window and store lock).
pub fn launch(root: &std::path::Path) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--signin").arg(root).spawn();
    }
}
