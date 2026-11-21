//! Reading direction in one place. Arabic and Hebrew mirror the whole shell: every view asks here for its layouts, panel sides and
//! mirrored geometry instead of naming left and right. Read from `i18n::rtl()` on every call, so a language change shows on the next frame.
use eframe::egui::{self, Align, Align2, Layout, Pos2, Rect, Ui};

pub fn rtl() -> bool {
    matrix_desktop::i18n::rtl()
}

/// The side a line starts from.
pub fn start() -> Align {
    if rtl() { Align::Max } else { Align::Min }
}

/// Top-down with lines starting on the reading side (`ui.vertical` in LTR); `ui.horizontal` rows inside follow it.
pub fn col() -> Layout {
    col_of(rtl())
}

/// A row in reading order (`left_to_right` in LTR).
pub fn row(valign: Align) -> Layout {
    row_of(rtl(), valign)
}

/// A row from the far side back (`right_to_left` in LTR): trailing buttons, badges, times.
pub fn row_rev(valign: Align) -> Layout {
    row_of(!rtl(), valign)
}

/// The same for a direction that is not the interface's (see `content_rtl`).
pub fn col_of(rtl: bool) -> Layout {
    Layout::top_down(if rtl { Align::Max } else { Align::Min })
}

pub fn row_of(rtl: bool, valign: Align) -> Layout {
    if rtl { Layout::right_to_left(valign) } else { Layout::left_to_right(valign) }
}

/// Direction of a block of user text (a message, a topic) inside an RTL interface: its first letter decides, the usual paragraph
/// rule, so an Italian sentence cut into links and pills keeps its words in order in a Hebrew window, and a Hebrew one keeps the
/// space next to its links in an Italian window. No letters: the interface's.
pub fn content_rtl(text: &str) -> bool {
    first_letter_rtl(text).unwrap_or_else(rtl)
}

fn first_letter_rtl(text: &str) -> Option<bool> {
    // Hebrew, Arabic, Syriac, Thaana, NKo and their presentation forms.
    text.chars().find(|c| c.is_alphabetic()).map(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC))
}

/// `ui.horizontal_wrapped` running in the given direction: for user text laid out as several widgets.
pub fn wrapped<R>(ui: &mut Ui, rtl: bool, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row_of(rtl, Align::Center).with_main_wrap(true), add)
}

/// Puts the content of a panel, window or popup in reading direction. LTR: the ui as it is.
pub fn scope<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if !rtl() {
        return add(ui);
    }
    let justify = ui.layout().cross_justify;
    ui.with_layout(col().with_cross_justify(justify), add).inner
}

/// `scope` for a popup as wide as its content (a bare `Area`): lines can only be pushed to the right of a known width, so the content
/// is laid out in the width it had the frame before (left to right the first time).
pub fn popup<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if !rtl() {
        return add(ui);
    }
    let id = ui.id().with("rtl-width");
    let justify = ui.layout().cross_justify;
    let out = match ui.data(|d| d.get_temp::<f32>(id)) {
        Some(width) => ui.allocate_ui_with_layout(egui::vec2(width, 0.), col().with_cross_justify(justify), add),
        None => {
            ui.ctx().request_repaint();
            ui.scope(add)
        }
    };
    ui.data_mut(|d| d.insert_temp(id, out.response.rect.width()));
    out.inner
}

/// The panel on the side reading starts from (rail, chat list) and the one on the far side (info, thread).
pub fn panel_start(id: &'static str) -> egui::Panel {
    if rtl() { egui::Panel::right(id) } else { egui::Panel::left(id) }
}

pub fn panel_end(id: &'static str) -> egui::Panel {
    if rtl() { egui::Panel::left(id) } else { egui::Panel::right(id) }
}

fn mirror(outer: Rect, r: Rect, rtl: bool) -> Rect {
    if !rtl {
        return r;
    }
    let sum = outer.left() + outer.right();
    Rect::from_x_y_ranges((sum - r.right())..=(sum - r.left()), r.y_range())
}

/// A rectangle worked out for LTR inside `outer`, where it belongs in the current direction. Hand-painted rows keep their LTR maths
/// and pass every rectangle and point through here.
pub fn flip(outer: Rect, r: Rect) -> Rect {
    mirror(outer, r, rtl())
}

pub fn flip_pos(outer: Rect, p: Pos2) -> Pos2 {
    flip(outer, Rect::from_min_max(p, p)).min
}

/// Top-left corner for something `width` wide (a galley) whose LTR top-left is `p`.
pub fn place(outer: Rect, p: Pos2, width: f32) -> Pos2 {
    flip(outer, Rect::from_min_size(p, egui::vec2(width, 0.))).min
}

fn mirror_align(a: Align2, rtl: bool) -> Align2 {
    let x = match a.0[0] {
        Align::Min if rtl => Align::Max,
        Align::Max if rtl => Align::Min,
        other => other,
    };
    Align2([x, a.0[1]])
}

/// An anchor named for LTR (`LEFT_CENTER`…), on the side it belongs in the current direction.
pub fn align(a: Align2) -> Align2 {
    mirror_align(a, rtl())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirroring_swaps_sides_inside_the_outer_rect_and_is_identity_in_ltr() {
        let outer = Rect::from_min_max(egui::pos2(100., 0.), egui::pos2(400., 50.));
        let lead = Rect::from_min_max(egui::pos2(116., 10.), egui::pos2(156., 40.));
        assert_eq!(mirror(outer, lead, false), lead);
        let m = mirror(outer, lead, true);
        assert_eq!((m.left(), m.right(), m.top(), m.bottom()), (344., 384., 10., 40.));
        assert_eq!(mirror(outer, m, true), lead, "twice is back");
        assert_eq!(mirror_align(Align2::LEFT_CENTER, true), Align2::RIGHT_CENTER);
        assert_eq!(mirror_align(Align2::RIGHT_TOP, true), Align2::LEFT_TOP);
        assert_eq!(mirror_align(Align2::CENTER_CENTER, true), Align2::CENTER_CENTER);
        assert_eq!(mirror_align(Align2::LEFT_CENTER, false), Align2::LEFT_CENTER);
    }

    #[test]
    fn the_first_letter_gives_the_direction_of_a_text() {
        assert_eq!(first_letter_rtl("Ciao שלום"), Some(false));
        assert_eq!(first_letter_rtl("12:30 שלום ciao"), Some(true));
        assert_eq!(first_letter_rtl("مرحبا"), Some(true));
        assert_eq!(first_letter_rtl("👍 123"), None);
    }
}
