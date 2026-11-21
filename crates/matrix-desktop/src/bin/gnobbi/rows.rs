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

pub fn duplicate_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<bool> {
    let names: Vec<String> = names.into_iter().map(|name| name.trim().to_lowercase()).collect();
    let mut counts = std::collections::HashMap::new();
    for name in &names { *counts.entry(name).or_insert(0usize) += 1; }
    names.iter().map(|name| counts[name] > 1).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn person_row_is_clickable() {
        use eframe::egui::{self, Event, Modifiers, PointerButton};
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut clicked = false;
        for step in 0..4 {
            let pos = rect.center();
            let events = match step {
                2 => vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
                3 => vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 200.))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let r = super::person_row(ui, "Marco", "@one:example.org", true, false, &matrix_desktop::LIGHT);
                rect = r.rect;
                clicked |= r.clicked();
            });
            output.textures_delta.clear();
        }
        assert!(clicked);
    }

    #[test]
    fn duplicate_names_ignore_case_and_surrounding_space() {
        assert_eq!(super::duplicate_names([" Marco ", "MARCO", "Anna", "Éva", "éVA"]), [true, true, false, true, true]);
        assert!(super::duplicate_names(["Solo"]).iter().all(|x| !x));
    }
}

fn font(l: &Line) -> FontId {
    if l.mono { FontId::monospace(l.size) } else { FontId::proportional(l.size) }
}

pub fn person_row(ui: &mut egui::Ui, name: &str, id: &str, duplicate: bool, selected: bool, t: &matrix_desktop::DesignTokens) -> egui::Response {
    let name = crate::list::clean_single_line(name);
    let clean_id = crate::list::clean_single_line(id);
    let mut lines = vec![Line::new(&name, 14., crate::list::rgb(t.text))];
    if duplicate { lines.push(Line::new(&clean_id, 12., crate::list::rgb(t.secondary))); }
    let row = ui.scope(|ui| {
        lines_row(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(28.), Sense::hover());
            crate::avatar::paint(ui, r.center(), 28., crate::list::rgb(matrix_desktop::AVATAR_FALLBACK), &crate::avatar::Avatar::Missing, &matrix_desktop::initials(&name), 12., crate::list::rgb(t.on_accent));
        }, &lines, |_| {});
    }).response;
    let resp = crate::metrics::clickable(ui.interact(row.rect, row.id, Sense::click()));
    if selected { ui.painter().rect_stroke(row.rect, 3., egui::Stroke::new(1., crate::list::rgb(t.accent)), egui::StrokeKind::Inside); }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{name}, {id}")));
    resp
}

/// `lead` draws its own fixed-size picture (allocate it with `allocate_exact_size`); `trailing` is laid out right to left.
pub fn lines_row(ui: &mut egui::Ui, lead: impl FnOnce(&mut egui::Ui), lines: &[Line], trailing: impl FnOnce(&mut egui::Ui)) {
    let heights: Vec<f32> = lines.iter().map(|l| ui.painter().layout_no_wrap(l.text.to_owned(), font(l), Color32::WHITE).size().y).collect();
    let block = heights.iter().sum::<f32>() + LINE_GAP * lines.len().saturating_sub(1) as f32;
    let main = |ui: &mut egui::Ui| {
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
                ui.painter().galley(crate::dir::place(rect, egui::pos2(rect.left(), y), galley.size().x), galley, l.color);
                if let Some((g, fill)) = tag {
                    let pill = crate::dir::flip(rect, egui::Rect::from_min_size(egui::pos2(x, y + (h - g.size().y - 2.) / 2.), g.size() + Vec2::new(8., 2.)));
                    ui.painter().rect_filled(pill, 4., fill);
                    ui.painter().galley(pill.center() - g.size() / 2., g, Color32::WHITE);
                }
                y += h + LINE_GAP;
            }
    };
    // `Sides` names its halves left and right: in RTL lead and text go to the right one.
    let sides = egui::containers::Sides::new().height(ROW.max(block + 12.)).spacing(GAP).truncate();
    if crate::dir::rtl() { sides.shrink_right().show(ui, trailing, main); } else { sides.shrink_left().show(ui, main, trailing); }
}
