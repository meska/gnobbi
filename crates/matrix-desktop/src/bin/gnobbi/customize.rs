//! «Personalizza» drawer, opened from the settings: look (theme, accent, app icon, night) and the interface sizes, with a live preview.
use crate::settings::{choice, section};
use eframe::egui::{self, Vec2};
use matrix_desktop::i18n::{self, t as tr};
use matrix_desktop::{Accent, AppDirs, AutoNight, Density, DesignTokens, IconChoice, Preferences, TextSize, Theme};

/// Draws the drawer; returns the new preferences when something changed (already saved). `system` is the OS text scale.
pub fn show(ctx: &egui::Context, rail: &mut crate::rail::State, prefs: &Preferences, system: f32, dirs: Option<&AppDirs>, t: &DesignTokens) -> Option<Preferences> {
    let mut p = prefs.clone();
    let mut changed = false;
    let mut back = false;
    let closed = crate::rail::drawer_back(ctx, rail.panel, "customize-drawer", tr("Customize"), rail.customize, t, Some(&mut back), |ui, rect| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 20 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.;
                    section(ui, tr("Sizes"), t);
                    let shown = if p.text_size == TextSize::System { format!("{} ({}%)", TextSize::System.label(), (TextSize::System.zoom(system) * 100.).round()) } else { p.text_size.label().to_owned() };
                    choice(ui, tr("Text size"), &shown, |ui| {
                        for s in TextSize::ALL {
                            changed |= ui.selectable_value(&mut p.text_size, s, s.label()).changed();
                        }
                    });
                    choice(ui, tr("Buttons and icons size"), p.density.label(), |ui| {
                        for d in Density::ALL {
                            changed |= ui.selectable_value(&mut p.density, d, d.label()).changed();
                        }
                    });
                    preview(ui, t);
                    if ui.button(tr("Restore defaults")).clicked() && (p.text_size, p.density) != (TextSize::System, Density::System) {
                        (p.text_size, p.density) = (TextSize::System, Density::System);
                        changed = true;
                    }
                    section(ui, tr("Appearance"), t);
                    choice(ui, tr("Language"), crate::settings::lang_name(p.language), |ui| {
                        for l in i18n::Preference::ALL {
                            changed |= ui.selectable_value(&mut p.language, l, crate::settings::lang_name(l)).changed();
                        }
                    });
                    choice(ui, tr("Theme"), p.theme.map_or(tr("System"), Theme::label), |ui| {
                        changed |= ui.selectable_value(&mut p.theme, None, tr("System")).changed();
                        for th in Theme::ALL {
                            changed |= ui.selectable_value(&mut p.theme, Some(th), th.label()).changed();
                        }
                    });
                    choice(ui, tr("Accent"), p.accent.map_or(tr("System"), Accent::label), |ui| {
                        changed |= ui.selectable_value(&mut p.accent, None, tr("System")).changed();
                        for a in Accent::ALL {
                            changed |= ui.selectable_value(&mut p.accent, Some(a), a.label()).changed();
                        }
                    });
                    choice(ui, tr("Icon"), p.icon.label(), |ui| {
                        for i in IconChoice::ALL {
                            changed |= ui.selectable_value(&mut p.icon, i, i.label()).changed();
                        }
                    });
                    choice(ui, tr("Automatic night"), p.auto_night.label(), |ui| {
                        for a in AutoNight::ALL {
                            changed |= ui.selectable_value(&mut p.auto_night, a, a.label()).changed();
                        }
                    });
                });
            });
        });
    });
    if back {
        rail.set_settings(true);
    } else if closed {
        rail.customize = false;
    }
    if !changed {
        return None;
    }
    crate::settings::persist(dirs, &p);
    Some(p)
}

/// A chat-list row, a button and an icon at the scale in use: it changes as soon as a size is chosen.
fn preview(ui: &mut egui::Ui, t: &DesignTokens) {
    let rgb = crate::list::rgb;
    let m = crate::metrics::get();
    ui.add_space(8.);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 8., rgb(t.input));
    let c = egui::pos2(rect.left() + 32., rect.center().y);
    p.circle_filled(c, 20., rgb(crate::AVATAR));
    p.text(c, egui::Align2::CENTER_CENTER, "G", egui::FontId::proportional(m.body + 1.), rgb(t.on_accent));
    let name = crate::list::one_line(ui, tr("Preview"), m.body + 1., rgb(t.text), rect.width() - 80.);
    ui.painter().galley(egui::pos2(rect.left() + 64., rect.top() + 8.), name, rgb(t.text));
    let sub = crate::list::one_line(ui, tr("A sample message"), m.secondary + 1., rgb(t.secondary), rect.width() - 80.);
    ui.painter().galley(egui::pos2(rect.left() + 64., rect.top() + 29.), sub, rgb(t.secondary));
    ui.horizontal(|ui| {
        let _ = ui.button(crate::icons::with_label(crate::icons::Icon::Plus, tr("Button")));
        let (r, _) = ui.allocate_exact_size(Vec2::splat(crate::metrics::button()), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r.center(), crate::icons::Icon::ChatCircleDots, true, m.icon, rgb(t.accent));
        crate::icons::paint(ui.painter(), r.center() + Vec2::new(r.width(), 0.), crate::icons::Icon::Chats, false, m.icon, rgb(t.secondary));
    });
}
