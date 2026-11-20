//! The one row of the drawers with a lead (avatar or icon), one to three text lines and something on the right (button, badge, label):
//! lead and text block are centred on the same horizontal axis, the right side too. No hand offsets: sizes come from the measured text.
use eframe::egui::{self, Color32, FontId, Sense, Vec2};

/// Standard height of a two-line row.
pub const ROW: f32 = 52.;
/// Space between lead and text.
const GAP: f32 = 12.;
/// Space between two text lines.
const LINE_GAP: f32 = 2.;

pub struct Line<'a> {
    pub text: &'a str,
    pub size: f32,
    pub color: Color32,
    pub mono: bool,
    /// A small filled tag after the text: (label, fill, ink).
    pub tag: Option<(&'a str, Color32, Color32)>,
}

impl<'a> Line<'a> {
    pub fn new(text: &'a str, size: f32, color: Color32) -> Self {
        Line { text, size, color, mono: false, tag: None }
    }
}

fn font(l: &Line) -> FontId {
    if l.mono { FontId::monospace(l.size) } else { FontId::proportional(l.size) }
}

/// `lead` draws its own fixed-size picture (allocate it with `allocate_exact_size`); `trailing` is laid out right to left.
pub fn lines_row(ui: &mut egui::Ui, lead: impl FnOnce(&mut egui::Ui), lines: &[Line], trailing: impl FnOnce(&mut egui::Ui)) {
    let heights: Vec<f32> = lines.iter().map(|l| ui.painter().layout_no_wrap(l.text.to_owned(), font(l), Color32::WHITE).size().y).collect();
    let block = heights.iter().sum::<f32>() + LINE_GAP * lines.len().saturating_sub(1) as f32;
    egui::containers::Sides::new().height(ROW.max(block + 12.)).spacing(GAP).shrink_left().truncate().show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            lead(ui);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), block), Sense::hover());
            let mut y = rect.top();
            for (l, h) in lines.iter().zip(&heights) {
                let tag = l.tag.map(|(text, fill, ink)| (ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.), ink), fill));
                let tag_w = tag.as_ref().map_or(0., |(g, _)| g.size().x + 8. + 6.);
                let mut job = egui::text::LayoutJob::simple_singleline(l.text.to_owned(), font(l), l.color);
                job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - tag_w).max(20.));
                let galley = ui.painter().layout_job(job);
                let x = rect.left() + galley.size().x + 6.;
                ui.painter().galley(egui::pos2(rect.left(), y), galley, l.color);
                if let Some((g, fill)) = tag {
                    let pill = egui::Rect::from_min_size(egui::pos2(x, y + (h - g.size().y - 2.) / 2.), g.size() + Vec2::new(8., 2.));
                    ui.painter().rect_filled(pill, 4., fill);
                    ui.painter().galley(pill.center() - g.size() / 2., g, Color32::WHITE);
                }
                y += h + LINE_GAP;
            }
        },
        trailing,
    );
}
