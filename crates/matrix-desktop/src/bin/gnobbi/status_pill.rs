//! Overlay status shared by room loading and first-unread feedback.
use eframe::egui::{self, Rect, Vec2};
use matrix_desktop::DesignTokens;
use std::time::Duration;

const LIFE: Duration = Duration::from_secs(4);

fn opacity(age: Duration, dismissed: bool) -> f32 {
    if dismissed || age >= LIFE { return 0.; }
    (age.as_secs_f32() / 0.2).min((LIFE - age).as_secs_f32() / 0.25).clamp(0., 1.)
}

fn position(view: Rect, size: Vec2) -> egui::Pos2 {
    egui::pos2((view.center().x - size.x / 2.).max(view.left()), view.top() + 8.)
}

pub fn notice_alive(age: Duration, dismissed: bool) -> bool { !dismissed && age < LIFE }

pub fn show(ui: &mut egui::Ui, view: Rect, text: &str, loading: bool, age: Duration, t: &DesignTokens) {
    let alpha = if loading { 1. } else { opacity(age, false) };
    if alpha <= 0. { ui.ctx().request_repaint_after(Duration::from_millis(16)); return; }
    let font = egui::FontId::proportional(14.);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, crate::rgb(t.text));
    let size = Vec2::new((galley.size().x + 16. + 6. + 24.).min(view.width()), galley.size().y.max(16.) + 16.);
    let pos = position(view, size);
    egui::Area::new(ui.id().with("status-pill")).order(egui::Order::Foreground).fixed_pos(pos).interactable(false).show(ui.ctx(), |ui| {
        ui.set_max_width(view.width());
        egui::Frame::NONE.fill(crate::rgb(t.surface).gamma_multiply(0.93 * alpha)).corner_radius(egui::CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                if loading { ui.add(egui::Spinner::new().size(16.)); }
                else {
                    let (icon, _) = ui.allocate_exact_size(Vec2::splat(16.), egui::Sense::hover());
                    crate::icons::paint(ui.painter(), icon.center(), crate::icons::Icon::MagnifyingGlass, false, 16., crate::rgb(t.text).gamma_multiply(alpha));
                }
                ui.label(egui::RichText::new(text).size(14.).color(crate::rgb(t.text).gamma_multiply(alpha)));
            });
        });
    });
    ui.ctx().request_repaint_after(Duration::from_millis(if loading { 100 } else { 30 }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notice_expires_or_is_dismissed() {
        assert!(notice_alive(Duration::from_millis(3999), false));
        assert!(!notice_alive(Duration::from_secs(4), false));
        assert!(!notice_alive(Duration::ZERO, true));
        assert!(opacity(Duration::from_secs(1), false) > 0.9);
        assert_eq!(opacity(Duration::from_secs(4), false), 0.);
    }
    #[test]
    fn geometry_stays_inside_the_timeline_above_composer() {
        let view = Rect::from_min_max(egui::pos2(0., 70.), egui::pos2(320., 600.));
        let size = Vec2::new(270., 34.);
        let rect = Rect::from_min_size(position(view, size), size);
        assert!(view.contains_rect(rect));
        assert!(rect.left() >= view.left() + 12. && rect.right() <= view.right() - 12.);
        assert!(rect.bottom() < 600.);
    }
}
