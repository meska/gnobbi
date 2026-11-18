//! Central column: the selected room's messages, read-only. Rows have different heights, so the
//! heights seen so far are cached and off-screen rows are replaced by empty space.
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Vec2};
use matrix_desktop::{DemoEvent, DesignTokens};

const GAP: f32 = 8.;
const TIGHT: f32 = 2.;
const GUESS: f32 = 56.;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Measured row heights for the open room and the width they were measured at.
#[derive(Default)]
pub struct Heights {
    width: f32,
    rows: Vec<f32>,
}

/// True when the message was right-clicked (or long-pressed).
fn row(ui: &mut egui::Ui, e: &DemoEvent, label: Option<&str>, day: Option<&str>, t: &DesignTokens) -> Option<egui::Pos2> {
    if let Some(day) = day {
        ui.add_space(8.);
        ui.vertical_centered(|ui| ui.label(RichText::new(day).size(12.).color(rgb(t.secondary))));
        ui.add_space(8.);
    }
    if e.status {
        ui.vertical_centered(|ui| ui.label(RichText::new(&e.text).size(12.).color(rgb(t.secondary))));
        return None;
    }
    let max = (ui.available_width() * 0.72).max(120.);
    let layout = if e.outgoing { Layout::right_to_left(Align::TOP) } else { Layout::left_to_right(Align::TOP) };
    let mut opened = None;
    ui.with_layout(layout, |ui| {
        ui.add_space(12.);
        let fill = if e.highlighted { rgb(t.mention_row) } else if e.outgoing { rgb(t.outgoing) } else { rgb(t.surface) };
        let frame = egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(max);
            ui.vertical(|ui| {
                if let Some(name) = label {
                    ui.label(RichText::new(name).size(12.).strong().color(rgb(t.accent)));
                }
                if e.redacted {
                    ui.label(RichText::new("Messaggio eliminato").italics().color(rgb(t.secondary)));
                } else if !e.text.is_empty() {
                    ui.label(RichText::new(&e.text).size(15.));
                }
                ui.label(RichText::new(matrix_desktop::bubble_stamp(&e.time, e.outgoing, e.send_state)).size(11.).color(rgb(t.secondary)));
            });
        });
        let hit = ui.interact(frame.response.rect, ui.next_auto_id().with("menu").with(&e.id.id), egui::Sense::click());
        if hit.secondary_clicked() {
            opened = hit.interact_pointer_pos();
        }
    });
    opened
}

/// Draws the rows in view; returns the index of the first one in view, for paging back.
pub fn show(ui: &mut egui::Ui, events: &[DemoEvent], history: &matrix_desktop::DemoHistory, heights: &mut Heights, members: u64, t: &DesignTokens) -> (usize, Option<(matrix_domain::ScopedTimelineId, egui::Pos2)>) {
    let mut menu = None;
    if (heights.width - ui.available_width()).abs() > 1. {
        heights.width = ui.available_width();
        heights.rows.clear();
    }
    heights.rows.resize(events.len(), GUESS);
    let mut first = 0;
    egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show_viewport(ui, |ui, view| {
        ui.spacing_mut().item_spacing = Vec2::new(0., 0.);
        let mut y = 0.;
        let mut seen_first = false;
        ui.add_space(GAP);
        for (i, e) in events.iter().enumerate() {
            let h = heights.rows[i];
            let gap = if history.continues_series(i) { TIGHT } else { GAP };
            if y + h < view.min.y - 400. || y > view.max.y + 400. {
                ui.add_space(h + gap);
            } else {
                let top = ui.cursor().top();
                if let Some(pos) = row(ui, e, history.sender_label(i, members).as_deref(), history.day_separator(i).as_deref(), t) {
                    menu = Some((e.id.clone(), pos));
                }
                heights.rows[i] = ui.cursor().top() - top;
                ui.add_space(gap);
            }
            if !seen_first && y + h >= view.min.y {
                first = i;
                seen_first = true;
            }
            y += heights.rows[i] + gap;
        }
        ui.add_space(GAP);
    });
    (first, menu)
}
