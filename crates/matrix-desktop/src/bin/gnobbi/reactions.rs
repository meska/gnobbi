//! Reaction pills under a message: «emoji + count», wrapped onto lines inside the column, our own in the accent colour.
//! A tap adds or takes back ours, a long press or a right click lists who reacted. The line is part of the row: its height
//! comes from `height`, which the timeline also uses for rows out of view, so the cached heights follow a change of reactions.
use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, pos2, vec2};
use matrix_desktop::{DemoEvent, DesignTokens, i18n};
use matrix_engine::history::Reaction;

const PILL_H: f32 = 24.;
const GAP_X: f32 = 4.;
const GAP_Y: f32 = 4.;
/// Space between the bubble and the first line.
const TOP: f32 = 4.;
/// Touch target: the pill's hit area is at least this tall and wide.
const HIT: f32 = 44.;
const PAD_X: f32 = 8.;
const INNER: f32 = 4.;
/// Names the «who reacted» popup lists before «+N».
pub const MAX_NAMES: usize = 10;

/// Beside the count of a message that many people reacted to: nothing for one, «99+» past 99.
pub fn count_label(n: usize) -> Option<String> {
    match n {
        0 | 1 => None,
        2..=99 => Some(n.to_string()),
        _ => Some("99+".into()),
    }
}

/// Where the pills go: per pill its line and offset from the line's start, and the width of each line.
pub struct Flow {
    pub at: Vec<(usize, f32)>,
    pub lines: Vec<f32>,
}

/// Greedy wrap of pills `widths` wide into lines of at most `avail` (a pill wider than that is cut down to it).
pub fn flow(widths: &[f32], avail: f32, gap: f32) -> Flow {
    let mut at = Vec::with_capacity(widths.len());
    let mut lines: Vec<f32> = Vec::new();
    for w in widths {
        let w = w.min(avail);
        let n = lines.len();
        match lines.last_mut() {
            Some(line) if *line + gap + w <= avail => {
                at.push((n - 1, *line + gap));
                *line += gap + w;
            }
            _ => {
                lines.push(w);
                at.push((n, 0.));
            }
        }
    }
    Flow { at, lines }
}

fn block_height(lines: usize, pill_h: f32) -> f32 {
    if lines == 0 { 0. } else { TOP + lines as f32 * pill_h + (lines - 1) as f32 * GAP_Y }
}

/// The room a row leaves for pills, as (start, end) inside a column `[left, right]`: from the bubble's start edge (past the avatar
/// column of a group) to the margin; an own message ends at the margin, an incoming one starts at the bubble.
fn span(left: f32, right: f32, outgoing: bool, group: bool) -> (f32, f32) {
    (left + 12. + if group && !outgoing { 40. } else { 0. }, right - 12.)
}

fn scale() -> f32 {
    crate::metrics::get().body / 14.
}

fn label_of(r: &Reaction) -> String {
    let mut s = match count_label(r.count) {
        Some(n) => format!("{} · {n}", r.key),
        None => r.key.clone(),
    };
    if r.mine {
        s.push_str(" · ");
        s.push_str(i18n::t("Your reaction"));
    }
    s
}

fn galleys(ui: &egui::Ui, r: &Reaction, ink: Color32) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>) {
    let k = scale();
    let emoji = ui.painter().layout_no_wrap(r.key.clone(), egui::FontId::proportional(15. * k), ink);
    let count = count_label(r.count).map(|n| ui.painter().layout_no_wrap(n, egui::FontId::proportional(12. * k), ink));
    (emoji, count)
}

fn widths(ui: &egui::Ui, rs: &[Reaction]) -> Vec<f32> {
    rs.iter()
        .map(|r| {
            let (e, c) = galleys(ui, r, Color32::WHITE);
            2. * PAD_X + e.size().x + c.map_or(0., |c| INNER + c.size().x)
        })
        .collect()
}

/// Height the pills take below a bubble in a column `width` wide (0 without reactions).
pub fn height(ui: &egui::Ui, rs: &[Reaction], outgoing: bool, group: bool, width: f32) -> f32 {
    if rs.is_empty() {
        return 0.;
    }
    let (l, r) = span(0., width, outgoing, group);
    block_height(flow(&widths(ui, rs), r - l, GAP_X).lines.len(), PILL_H * scale())
}

/// What a pill asked for this frame.
#[derive(Default)]
pub struct Acts {
    /// Tapped: the message, the emoji, and whether it is ours now (a tap then takes it back).
    pub toggle: Option<(matrix_domain::EventId, String, bool)>,
    /// Long press or right click: the pill's rectangle and what to list.
    pub who: Option<(Rect, Reaction)>,
}

/// A long press that opened the popup must not also toggle when the finger lifts.
fn suppress_id() -> egui::Id {
    egui::Id::new("reaction-suppress-click")
}

/// Draws the pills of `e` below its bubble, at the column's current cursor. `rs` already has the pending taps laid over.
pub fn show(ui: &mut egui::Ui, e: &DemoEvent, rs: &[Reaction], group: bool, bubble: Rect, press: Option<egui::Pos2>, out: &mut Acts, t: &DesignTokens) {
    let target = match &e.id.id {
        matrix_domain::TimelineId::Server(id) => Some(id.clone()),
        _ => None,
    };
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let pill_h = PILL_H * scale();
    let (left, width) = (ui.cursor().left(), ui.available_width());
    let (l, r) = span(left, left + width, e.outgoing, group);
    let widths = widths(ui, rs);
    let f = flow(&widths, r - l, GAP_X);
    let h = block_height(f.lines.len(), pill_h);
    let (block, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    if ui.input(|i| i.pointer.any_pressed()) {
        ui.data_mut(|d| d.remove_temp::<bool>(suppress_id()));
    }
    let column = Rect::from_min_size(block.min, vec2(width, h));
    #[cfg(debug_assertions)]
    let log = std::env::var_os("GNOBBI_DEBUG_REACTIONS_FIXTURE").is_some();
    #[cfg(not(debug_assertions))]
    let _ = bubble;
    #[cfg(debug_assertions)]
    if log {
        // Test hook: the geometry in points, for the scripts that measure the screenshots.
        let g = |r: Rect| format!("{:.1},{:.1},{:.1},{:.1}", r.left(), r.top(), r.right(), r.bottom());
        eprintln!("reaction-block frame {} zoom {:.2} col {} bubble {} block {} out {} clip {}", ui.ctx().cumulative_pass_nr(), ui.ctx().zoom_factor(), g(Rect::from_min_max(pos2(l, block.top()), pos2(r, block.bottom()))), g(bubble), g(block), u8::from(e.outgoing), g(ui.clip_rect()));
    }
    for ((r_, w), (line, x)) in rs.iter().zip(&widths).zip(&f.at) {
        let w = w.min(r - l);
        // An own message's lines end at the margin; an incoming one's start at the bubble.
        let start = if e.outgoing { r - f.lines[*line] } else { l };
        let min = pos2(start + x, block.top() + TOP + *line as f32 * (pill_h + GAP_Y));
        let pill = crate::dir::flip(column, Rect::from_min_size(min, vec2(w, pill_h)));
        let ink = if r_.mine { rgb(t.on_accent) } else { rgb(t.text) };
        #[cfg(debug_assertions)]
        if log {
            eprintln!("reaction-pill frame {} zoom {:.2} pill {:.1},{:.1},{:.1},{:.1} mine {}", ui.ctx().cumulative_pass_nr(), ui.ctx().zoom_factor(), pill.left(), pill.top(), pill.right(), pill.bottom(), u8::from(r_.mine));
        }
        let hit = Rect::from_center_size(pill.center(), vec2(pill.width().max(HIT), HIT));
        let resp = match &target {
            Some(id) => Some(ui.interact(hit, ui.id().with(("reaction", id, &r_.key)), Sense::click())),
            None => None,
        };
        let hot = resp.as_ref().is_some_and(|r| r.hovered() || r.is_pointer_button_down_on());
        let dim = |c: Color32| if r_.pending { c.gamma_multiply(0.6) } else { c };
        let fill = if r_.mine { rgb(t.accent) } else if hot { rgb(t.selected) } else { rgb(t.surface) };
        let painter = ui.painter().with_clip_rect(pill);
        painter.rect_filled(pill, CornerRadius::same((pill_h / 2.) as u8), dim(fill));
        let (emoji, count) = galleys(ui, r_, dim(ink));
        let content = emoji.size().x + count.as_ref().map_or(0., |c| INNER + c.size().x);
        let mut x = pill.center().x - content.min(pill.width() - 2. * PAD_X).max(0.) / 2.;
        if crate::dir::rtl() {
            x = pill.center().x + content.min(pill.width() - 2. * PAD_X).max(0.) / 2. - emoji.size().x;
        }
        painter.galley(pos2(x, pill.center().y - emoji.size().y / 2.), emoji.clone(), ink);
        if let Some(c) = count {
            let cx = if crate::dir::rtl() { x - INNER - c.size().x } else { x + emoji.size().x + INNER };
            painter.galley(pos2(cx, pill.center().y - c.size().y / 2.), c, dim(ink));
        }
        let (Some(resp), Some(id)) = (resp, &target) else { continue };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, r_.mine, label_of(r_)));
        crate::metrics::clickable(resp.clone());
        let held = press.is_some_and(|p| hit.contains(p) && ui.is_rect_visible(hit));
        if held || resp.secondary_clicked() {
            out.who = Some((pill, r_.clone()));
            if held {
                ui.data_mut(|d| d.insert_temp(suppress_id(), true));
            }
        } else if resp.clicked() {
            if ui.data_mut(|d| d.remove_temp::<bool>(suppress_id())).unwrap_or(false) {
                continue;
            }
            out.toggle = Some((id.clone(), r_.key.clone(), r_.mine));
        }
    }
}

/// Who reacted: the popup of a pill, up to `MAX_NAMES` names and «+N». Returns false once it should close.
pub fn popup(ctx: &egui::Context, anchor: Rect, r: &Reaction, fresh: bool, t: &DesignTokens) -> bool {
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let area = egui::Area::new(egui::Id::new("reaction-who")).order(egui::Order::Tooltip).fixed_pos(pos2(anchor.left(), anchor.bottom() + 4.)).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(260.);
            let head = match count_label(r.count) {
                Some(n) => format!("{}  {n}", r.key),
                None => r.key.clone(),
            };
            ui.label(egui::RichText::new(head).strong().color(rgb(t.text)));
            for s in r.senders.iter().take(MAX_NAMES) {
                ui.add(egui::Label::new(egui::RichText::new(s.name.as_deref().unwrap_or(&s.user_id)).color(rgb(t.text))).truncate());
            }
            let more = r.count.saturating_sub(r.senders.len().min(MAX_NAMES));
            if more > 0 {
                ui.label(egui::RichText::new(format!("+{more}")).color(rgb(t.secondary)));
            }
        });
    });
    let outside = !fresh && ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    !(outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)))
}

/// Debug builds, GNOBBI_DEBUG_REACTIONS_FIXTURE=<1|2|3>: every message shows made-up reactions (survives the live refreshes), no server:
/// 1 = ours and another's, 2 = many (a wrapped line, 9, 10, 99+), 3 = only others'.
#[cfg(debug_assertions)]
pub fn fixture() -> Option<Vec<Reaction>> {
    let set: &[(&str, usize, bool)] = match std::env::var("GNOBBI_DEBUG_REACTIONS_FIXTURE").ok()?.as_str() {
        "1" => &[("👍", 2, true), ("❤️", 1, false)],
        "2" => &[("👍", 12, true), ("❤️", 1, false), ("😂", 3, false), ("😮", 9, false), ("😢", 10, false), ("🙏", 1, true), ("🎉", 100, false), ("🔥", 2, false), ("👀", 1, false)],
        "3" => &[("👍", 1, false), ("🎉", 3, false)],
        _ => return None,
    };
    Some(set.iter().map(|(key, count, mine)| Reaction {
        key: (*key).into(),
        count: *count,
        mine: *mine,
        pending: false,
        senders: (0..(*count).min(12)).map(|n| matrix_engine::history::ReactionSender { user_id: format!("@p{n}:example.org"), name: Some(format!("Persona {n}")) }).collect(),
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_hidden_for_one_and_capped() {
        assert_eq!([0, 1, 2, 9, 10, 99, 100, 4000].map(count_label), [None, None, Some("2".into()), Some("9".into()), Some("10".into()), Some("99".into()), Some("99+".into()), Some("99+".into())]);
    }

    #[test]
    fn pills_wrap_inside_the_column_whatever_its_width() {
        // 20 pills of 40..=64 wide (a count of one, ten, 99+) in the columns the app has: 300, 402 and 600 pt, at zoom 1.0 / 1.15 / 1.34
        // (the zoom shrinks the column in points).
        let widths: Vec<f32> = (0..20).map(|i| 40. + (i % 4) as f32 * 8.).collect();
        for column in [300., 402., 600.] {
            for zoom in [1.0, 1.15, 1.34] {
                let avail = (column / zoom - 24.) as f32;
                let f = flow(&widths, avail, GAP_X);
                for ((line, x), w) in f.at.iter().zip(&widths) {
                    assert!(x + w <= avail + 0.01 && *x >= 0., "pill leaves the column at {column}/{zoom}");
                    assert!(*line < f.lines.len());
                }
                assert!(f.lines.iter().all(|w| *w <= avail + 0.01));
                assert_eq!(f.at.iter().map(|(l, _)| *l).max(), Some(f.lines.len() - 1));
                let h = block_height(f.lines.len(), PILL_H);
                assert_eq!(h, TOP + f.lines.len() as f32 * PILL_H + (f.lines.len() - 1) as f32 * GAP_Y);
            }
        }
    }

    #[test]
    fn a_line_holds_what_fits_and_the_next_starts_at_zero() {
        let f = flow(&[50., 50., 50.], 110., 4.);
        assert_eq!(f.at, [(0, 0.), (0, 54.), (1, 0.)]);
        assert_eq!(f.lines, [104., 50.]);
        // Wider than the column: cut down to it, alone on its line.
        let f = flow(&[200., 30.], 100., 4.);
        assert_eq!(f.at, [(0, 0.), (1, 0.)]);
        assert_eq!(f.lines, [100., 30.]);
        assert!(flow(&[], 100., 4.).lines.is_empty());
        assert_eq!(block_height(0, PILL_H), 0.);
        assert_eq!(block_height(1, PILL_H), TOP + PILL_H);
    }

    #[test]
    fn the_room_for_pills_follows_the_bubble_side() {
        assert_eq!(span(0., 300., false, false), (12., 288.));
        assert_eq!(span(0., 300., false, true), (52., 288.), "incoming in a group: past the avatar");
        assert_eq!(span(0., 300., true, true), (12., 288.));
    }
}
