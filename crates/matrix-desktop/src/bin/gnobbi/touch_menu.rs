//! The message menu on touch: a long press lifts the bubble over a dimmed chat, with a bar of quick reactions on one side and the
//! list of actions on the other. The entries are the same list the right-click menu shows (`actions`).
//!
//! Who gets the finger (one rule, in `LongPress` and `long_press`):
//! - only a touch counts (iOS, a touch screen, GNOBBI_EGUI_TOUCH): a held mouse button stays text selection and drag;
//! - a press that starts in the back-swipe strip (`mobile::BACK_EDGE`) belongs to the swipe, never to the menu;
//! - moving more than `SLOP` or any scroll before `HOLD` gives the press back to the timeline (scroll, swipe) for good;
//! - lifting before `HOLD` is a tap: links, pictures and buttons get it as before;
//! - at `HOLD` the menu wins: the drag in progress is stopped and what the timeline reports while the menu is open is dropped,
//!   so the release of the same finger cannot open the link or the picture under it.
use crate::icons::Icon;
use crate::session::Session;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Vec2, pos2, vec2};
use matrix_desktop::{DesignTokens, i18n};
use matrix_engine::operations::MessagePermissions;
use std::time::Instant;

/// Seconds a finger rests on a message before the menu opens.
pub const HOLD: f64 = 0.35;
/// How far it may wander meanwhile.
pub const SLOP: f32 = 10.;
const GAP: f32 = 8.;
const MARGIN: f32 = 16.;
const ROW: f32 = 48.;
const CELL: f32 = 44.;
const PAD: f32 = 4.;
const WIDTH: f32 = 280.;
const RADIUS: u8 = 14;
const OPEN_SECS: f32 = 0.18;
const CLOSE_SECS: f32 = 0.12;

/// Quick reactions of the bar (name read by screen readers, emoji). shortcut: fixed set, the app keeps no reaction history; fill from it when it does.
pub const QUICK: [(&str, &str); 6] = [("Thumbs up", "👍"), ("Red heart", "❤️"), ("Face with tears of joy", "😂"), ("Face with open mouth", "😮"), ("Crying face", "😢"), ("Folded hands", "🙏")];

/// A finger resting on one spot. Fed once per frame with injected time, so it is testable without a screen.
#[derive(Default, Clone, Copy)]
pub struct LongPress {
    start: Option<(f64, Pos2)>,
    fired: Option<(f64, Pos2)>,
}

impl LongPress {
    /// `pressed`: the finger went down this frame at this point, and whether the spot is free for a long press;
    /// `down`: where it is while it stays down; `scrolled`: the view moved. Returns the pressed point once, when the hold completes
    /// (the same answer for every caller of that frame).
    pub fn step(&mut self, now: f64, pressed: Option<(Pos2, bool)>, down: Option<Pos2>, scrolled: bool) -> Option<Pos2> {
        if let Some((at, p)) = self.fired && at == now {
            return Some(p);
        }
        if let Some((p, free)) = pressed {
            self.start = free.then_some((now, p));
        }
        let (t0, p0) = self.start?;
        match down {
            Some(p) if !scrolled && p.distance(p0) <= SLOP => {
                if now - t0 < HOLD {
                    return None;
                }
                self.start = None;
                self.fired = Some((now, p0));
                Some(p0)
            }
            _ => {
                self.start = None;
                None
            }
        }
    }
}

/// The long press of this frame, if one just completed (see the module comment for what may start one).
pub fn long_press(ctx: &egui::Context) -> Option<Pos2> {
    let id = egui::Id::new("message-long-press");
    let (touch, now, pressed, down, scrolled) = ctx.input(|i| {
        (crate::mobile::touch() || i.any_touches(), i.time, i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten(), i.pointer.primary_down().then(|| i.pointer.interact_pos()).flatten(), i.smooth_scroll_delta != Vec2::ZERO)
    });
    if !touch {
        return None;
    }
    let mut state = ctx.data(|d| d.get_temp::<LongPress>(id)).unwrap_or_default();
    let hit = state.step(now, pressed.map(|p| (p, !crate::mobile::in_back_edge(ctx, p))), down, scrolled);
    if state.start.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30)); // a still finger sends no events: keep the clock running
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    hit
}

/// One entry of the message menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Reply,
    Thread,
    Copy,
    Edit,
    Sticker,
    EndPoll,
    Delete,
}

impl Action {
    fn icon(self) -> Icon {
        match self {
            Action::Reply => Icon::ArrowUUpLeft,
            Action::Thread => Icon::Chats,
            Action::Copy => Icon::Copy,
            Action::Edit => Icon::PencilSimple,
            Action::Sticker => Icon::Sticker,
            Action::EndPoll => Icon::ChartBarHorizontal,
            Action::Delete => Icon::Trash,
        }
    }

    /// `confirm`: the entry was chosen once and waits for the second tap.
    pub fn label(self, confirm: bool) -> &'static str {
        match (self, confirm) {
            (Action::Reply, _) => i18n::t("Reply"),
            (Action::Thread, _) => i18n::t("Reply in thread"),
            (Action::Copy, _) => i18n::t("Copy"),
            (Action::Edit, _) => i18n::t("Edit"),
            (Action::Sticker, _) => i18n::t("Add to my stickers"),
            (Action::EndPoll, false) => i18n::t("Close poll"),
            (Action::EndPoll, true) => i18n::t("Confirm closing"),
            (Action::Delete, false) => i18n::t("Delete"),
            (Action::Delete, true) => i18n::t("Confirm deletion"),
        }
    }
}

/// What the menu offers for a message, right-click and long press alike: the engine's permissions, `text` = it has text to copy,
/// `picture` = a picture or sticker that can join the own stickers, `end_poll` = `LiveOperations::can_end_poll`.
pub fn actions(p: MessagePermissions, text: bool, picture: bool, end_poll: bool) -> Vec<Action> {
    [(p.reply, Action::Reply), (p.reply, Action::Thread), (text, Action::Copy), (p.edit, Action::Edit), (picture, Action::Sticker), (end_poll, Action::EndPoll), (p.redact, Action::Delete)].into_iter().filter_map(|(on, a)| on.then_some(a)).collect()
}

/// The message the menu is open on.
pub fn selected(s: &Session) -> Option<&matrix_desktop::DemoEvent> {
    let id = s.ops.selection()?;
    s.messages.as_ref()?.events().iter().find(|e| e.id == *id)
}

/// The selected message when it is a picture or a sticker the server knows: it can be added to the own stickers.
fn picture(s: &Session) -> Option<matrix_domain::EventId> {
    match (&selected(s)?.media, &s.ops.selection()?.id) {
        (Some(m), matrix_domain::TimelineId::Server(event)) if m.kind == matrix_engine::history::MediaKind::Image => Some(event.clone()),
        _ => None,
    }
}

/// The entries for the selected message, with their «waiting for the second tap» state; `None` until the engine has answered.
pub fn entries(s: &Session) -> Option<(MessagePermissions, Vec<(Action, bool)>)> {
    let p = s.ops.details()?.permissions;
    let e = selected(s);
    let text = e.is_some_and(|e| !e.redacted && e.poll.is_none() && !e.text.trim().is_empty());
    let list = actions(p, text, picture(s).is_some(), s.ops.can_end_poll(e.and_then(|e| e.poll.as_ref())));
    Some((p, list.into_iter().map(|a| (a, (a == Action::Delete && s.confirm_delete) || (a == Action::EndPoll && s.confirm_end_poll))).collect()))
}

/// Runs a chosen entry. Delete and «close poll» act on the second tap.
pub fn apply(ctx: &egui::Context, s: &mut Session, action: Action) {
    match action {
        Action::Reply => s.begin(crate::composer::Mode::Reply),
        Action::Edit => s.begin(crate::composer::Mode::Edit),
        Action::Thread => {
            let root = s.ops.selection().and_then(|id| match &id.id {
                matrix_domain::TimelineId::Server(id) => Some(id.clone()),
                _ => None,
            });
            s.close_menu();
            if let Some(root) = root {
                s.open_thread(root);
            }
        }
        Action::Copy => {
            if let Some(e) = selected(s) {
                ctx.copy_text(e.text.clone());
            }
            s.close_menu();
        }
        Action::Sticker => {
            let id = picture(s);
            s.close_menu();
            if let Some(id) = id {
                s.sticker_from_message(id);
            }
        }
        Action::EndPoll if s.confirm_end_poll => s.end_poll(),
        Action::EndPoll => s.confirm_end_poll = true,
        Action::Delete if s.confirm_delete => s.delete(),
        Action::Delete => s.confirm_delete = true,
    }
}

/// Where the reaction bar and the action list go around a lifted bubble that stays where it is.
/// `area` is the room they may use (margins already taken off), `bar` and `list` their sizes.
/// Enough room on both sides: bar above, list below. Message near the bottom: both above it; near the top: both below.
/// A message taller than the room: over its visible part, bar at the top end and list at the bottom end.
pub fn place(area: Rect, bubble: Rect, bar: Vec2, list: Vec2) -> (Rect, Rect) {
    let bar = vec2(bar.x.min(area.width()), bar.y);
    let list = vec2(list.x.min(area.width()), list.y.min((area.height() - bar.y - GAP).max(ROW)));
    // The part of the bubble in view.
    let top = bubble.top().clamp(area.top(), area.bottom());
    let bottom = bubble.bottom().clamp(top, area.bottom());
    let (above, below) = (top - area.top(), area.bottom() - bottom);
    let (bh, lh) = (bar.y + GAP, list.y + GAP);
    let (bar_y, list_y) = if above >= bh && below >= lh {
        (top - bh, bottom + GAP)
    } else if above >= bh + lh {
        (top - bh, top - bh - lh)
    } else if below >= bh + lh {
        (bottom + GAP, bottom + GAP + bh)
    } else {
        let list_y = if below >= lh { bottom + GAP } else { area.bottom() - list.y };
        let bar_y = if above >= bh { top - bh } else { area.top() }.min(list_y - bh).max(area.top());
        (bar_y, list_y.max(bar_y + bh))
    };
    // On the bubble's side of the chat (own messages sit on the far side), never past the margins.
    let far = bubble.center().x > area.center().x;
    let x = |w: f32| if far { bubble.right() - w } else { bubble.left() }.clamp(area.left(), (area.right() - w).max(area.left()));
    (Rect::from_min_size(pos2(x(bar.x), bar_y), bar), Rect::from_min_size(pos2(x(list.x), list_y), list))
}

/// Opacity and scale of the menu `open` seconds after it appeared and `closing` seconds into its dismissal.
/// In: a short spring (ease-out-back from 0.9). Out: a fade. Reduce Motion: fades only.
pub fn anim(open: f32, closing: Option<f32>, reduce_motion: bool) -> (f32, f32) {
    let x = (open / OPEN_SECS).clamp(0., 1.);
    let (c1, c3) = (1.70158, 2.70158);
    let back = 1. + c3 * (x - 1.).powi(3) + c1 * (x - 1.).powi(2);
    let alpha = (1. - (1. - x).powi(3)) * closing.map_or(1., |c| 1. - (c / CLOSE_SECS).clamp(0., 1.));
    (alpha, if reduce_motion || closing.is_some() { 1. } else { 0.9 + 0.1 * back })
}

/// A long-pressed message: lifted over the dimmed chat until the menu is gone.
pub struct Lift {
    pub id: matrix_domain::ScopedTimelineId,
    /// The bubble on screen, followed every frame (new messages may push it).
    pub rect: Rect,
    /// The timeline's viewport: where the menu may go.
    pub view: Rect,
    opened: Instant,
    /// The engine answered and the bar and list came in.
    shown: Option<Instant>,
    closing: Option<Instant>,
    /// The full emoji grid replaced the list («+»).
    pub picker: bool,
    /// What was on screen last: drawn inert while it fades out (the selection is gone by then).
    react: bool,
    own: [bool; QUICK.len()],
    items: Vec<(Action, bool)>,
}

impl Lift {
    pub fn new(id: matrix_domain::ScopedTimelineId, rect: Rect, view: Rect) -> Self {
        Self { id, rect, view, opened: Instant::now(), shown: None, closing: None, picker: false, react: false, own: [false; QUICK.len()], items: Vec::new() }
    }

    fn alpha(&self, now: Instant) -> f32 {
        anim((now - self.opened).as_secs_f32(), self.closing.map(|c| (now - c).as_secs_f32()), true).0
    }

    /// How far the bubble is lifted (0..1), for the timeline that draws it.
    pub fn lifted(&self) -> f32 {
        self.alpha(Instant::now())
    }
}

/// The layer the timeline paints the lifted bubble on.
pub fn lift_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Foreground, egui::Id::new("message-lift"))
}

fn rgb(c: u32) -> Color32 {
    crate::list::rgb(c)
}

/// A rounded card of the menu on its own layer; returns what `add` returns.
/// Its shadow goes on `under` (the scrim's layer): on the card's own layer it would darken the other card and the lifted bubble next to it.
fn card<R>(ctx: &egui::Context, under: &egui::Painter, name: &'static str, rect: Rect, alpha: f32, to_global: egui::emath::TSTransform, t: &DesignTokens, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let radius = CornerRadius::same(RADIUS);
    under.add(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha((56. * alpha) as u8) }.as_shape(rect, radius));
    let area = egui::Area::new(egui::Id::new(name)).order(egui::Order::Tooltip).fixed_pos(rect.min).constrain(false).sense(Sense::hover()).show(ctx, |ui| {
        ui.set_opacity(alpha);
        let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
        ui.painter().rect_filled(r, radius, rgb(t.surface));
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(PAD)).layout(crate::dir::col()));
        inner.spacing_mut().item_spacing = Vec2::ZERO;
        add(&mut inner)
    });
    ctx.set_transform_layer(area.response.layer_id, to_global);
    ctx.move_to_top(area.response.layer_id);
    area.inner
}

/// One emoji button of `size`; `on` = the user's own reaction, shown on an accent disc.
fn emoji_cell(ui: &mut egui::Ui, size: Vec2, name: &str, emoji: &str, on: bool, t: &DesignTokens) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, i18n::t(name)));
    let disc = rect.width().min(rect.height()) / 2. - 2.;
    if on {
        ui.painter().circle_filled(rect.center(), disc, rgb(t.accent).gamma_multiply(0.3));
    } else if resp.is_pointer_button_down_on() || resp.hovered() || resp.has_focus() {
        ui.painter().circle_filled(rect.center(), disc, rgb(t.selected));
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, emoji, egui::FontId::proportional(24.), rgb(t.text));
    resp.clicked()
}

/// What a tap in the menu chose.
enum Pick {
    React(&'static str),
    Picker,
    Do(Action),
}

/// Draws the touch menu of the lifted message, if any, and runs what is chosen.
pub fn show(ctx: &egui::Context, s: &mut Session, t: &DesignTokens) {
    let Some(mut lift) = s.lift.take() else { return };
    let now = Instant::now();
    let reset = |ctx: &egui::Context| ctx.set_transform_layer(lift_layer(), egui::emath::TSTransform::IDENTITY);
    if s.menu.is_some() && s.ops.selection() == Some(&lift.id) && s.ops.inspection_error().is_some() {
        s.close_menu(); // the engine offers nothing for this event: let the bubble down
    }
    let open = s.menu.is_some() && s.ops.selection() == Some(&lift.id);
    if !open && lift.closing.is_none() {
        lift.closing = Some(now);
    }
    if lift.closing.is_some_and(|c| (now - c).as_secs_f32() >= CLOSE_SECS) {
        return reset(ctx);
    }
    if open && let Some((p, items)) = entries(s) {
        (lift.react, lift.items) = (p.react, items);
        lift.own = QUICK.map(|(_, emoji)| s.reacted(emoji));
        lift.shown.get_or_insert(now);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("GNOBBI_DEBUG_LONGPRESS_PICKER").is_some() {
        lift.picker = true; // test hook: the emoji grid, for screenshots
    }
    let reduce = crate::mobile::reduce_motion();
    let dim = lift.alpha(now);
    // Dimmed chat under everything of the menu: a tap or a pull down on it closes the menu. shortcut: no blur (egui has none); dim only.
    let screen = ctx.viewport_rect();
    // Three orders keep the stack fixed whatever was created or touched first: scrim (Middle) < bubble (Foreground) < cards (Tooltip).
    let scrim = egui::Area::new(egui::Id::new("message-scrim")).order(egui::Order::Middle).fixed_pos(screen.min).constrain(false).sense(Sense::hover()).show(ctx, |ui| {
        let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        // The dialogs' scrim; a dark theme needs more of it for the lifted bubble to stand out. Own painter: the area's stops at the safe area.
        let strength = (matrix_desktop::SCRIM & 0xff) as f32 * if ui.visuals().dark_mode { 1.5 } else { 1. };
        egui::Painter::new(ui.ctx().clone(), ui.layer_id(), screen).rect_filled(rect, 0., Color32::from_black_alpha((strength * dim) as u8));
        resp
    });
    ctx.move_to_top(scrim.response.layer_id);
    let under = egui::Painter::new(ctx.clone(), scrim.response.layer_id, screen);
    // The bubble (painted by the timeline on its own layer): slightly bigger, around its centre.
    let grow = if reduce { 1. } else { 1. + 0.02 * dim };
    ctx.set_transform_layer(lift_layer(), egui::emath::TSTransform::new(lift.rect.center().to_vec2() * (1. - grow), grow));
    ctx.move_to_top(lift_layer());
    let mut pick = None;
    if let Some(shown) = lift.shown {
        let (alpha, scale) = anim((now - shown).as_secs_f32(), lift.closing.map(|c| (now - c).as_secs_f32()), reduce);
        // Room for the menu: the timeline's viewport inside the safe area; when that is too short (keyboard up) it may cover the composer.
        let safe = ctx.content_rect();
        let columns = (((WIDTH.min(safe.width() - 2. * MARGIN) - 2. * PAD) / CELL).floor() as usize).max(1);
        let rows = if lift.picker { matrix_desktop::EMOJI_SET.len().div_ceil(columns) as f32 * CELL } else { lift.items.len() as f32 * ROW };
        let bar_size = if lift.react { vec2((QUICK.len() + 1) as f32 * CELL + 2. * PAD, CELL + 2. * PAD) } else { Vec2::ZERO };
        let list_size = if rows > 0. { vec2(WIDTH, rows + 2. * PAD) } else { Vec2::ZERO };
        let mut area = lift.view.intersect(safe).shrink2(vec2(MARGIN, GAP));
        if area.height() < bar_size.y + list_size.y + 2. * GAP {
            area.max.y = safe.bottom() - MARGIN;
        }
        let (bar, list) = place(area, lift.rect, bar_size, list_size);
        #[cfg(debug_assertions)]
        if std::env::var_os("GNOBBI_DEBUG_LONGPRESS_MSG").is_some() {
            // Test hook: the geometry in points, for the scripts that measure the screenshots.
            let r = |r: Rect| format!("{:.1},{:.1},{:.1},{:.1}", r.left(), r.top(), r.right(), r.bottom());
            eprintln!("touch-menu zoom {:.2} screen {} safe {} view {} bubble {} bar {} list {}", ctx.zoom_factor(), r(screen), r(safe), r(lift.view), r(lift.rect), r(bar), r(list));
        }
        // Both cards grow out of the bubble's corner between them.
        let pivot = pos2(if lift.rect.center().x > area.center().x { bar.right() } else { bar.left() }, lift.rect.center().y.clamp(area.top(), area.bottom()));
        let spring = egui::emath::TSTransform::new(pivot.to_vec2() * (1. - scale), scale);
        if lift.react {
            card(ctx, &under, "message-reactions", bar, alpha, spring, t, |ui| {
                ui.horizontal(|ui| {
                    let size = vec2(ui.available_width() / (QUICK.len() + 1) as f32, CELL);
                    for ((name, emoji), on) in QUICK.into_iter().zip(lift.own) {
                        if emoji_cell(ui, size, name, emoji, on, t) {
                            pick = Some(Pick::React(emoji));
                        }
                    }
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    if crate::icons::button_at(ui, rect, ui.id().with("more"), Icon::Plus, lift.picker, crate::metrics::get().icon, i18n::t("Emoji"), Some(rgb(t.secondary)), true, 1.).clicked() {
                        pick = Some(Pick::Picker);
                    }
                });
            });
        }
        if rows > 0. {
            card(ctx, &under, "message-actions", list, alpha, spring, t, |ui| {
                let mine: Vec<String> = s.selected_reactions().into_iter().filter(|r| r.mine).map(|r| r.key).collect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if lift.picker {
                        let size = vec2(ui.available_width() / columns as f32, CELL);
                        for line in matrix_desktop::EMOJI_SET.chunks(columns) {
                            ui.horizontal(|ui| {
                                for (name, emoji) in line {
                                    if emoji_cell(ui, size, name, emoji, mine.iter().any(|k| k == emoji), t) {
                                        pick = Some(Pick::React(*emoji));
                                    }
                                }
                            });
                        }
                        return;
                    }
                    for (action, confirm) in &lift.items {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                        let label = action.label(*confirm);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
                        let danger = *action == Action::Delete;
                        // Waiting for the second tap: the row turns into a filled destructive button.
                        let ink = if *confirm { rgb(t.on_accent) } else if danger { rgb(t.error) } else { rgb(t.text) };
                        let radius = CornerRadius::same(RADIUS - PAD as u8);
                        if *confirm {
                            ui.painter().rect_filled(rect, radius, rgb(t.error));
                        } else if resp.is_pointer_button_down_on() || resp.hovered() || resp.has_focus() {
                            ui.painter().rect_filled(rect, radius, rgb(t.selected));
                        }
                        let m = crate::metrics::get();
                        crate::icons::paint(ui.painter(), crate::dir::flip_pos(rect, pos2(rect.left() + 24., rect.center().y)), action.icon(), false, m.icon + 2., ink);
                        let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(m.body + 1.), ink);
                        let at = crate::dir::place(rect, pos2(rect.left() + 52., rect.center().y - galley.size().y / 2.), galley.size().x);
                        ui.painter().galley(at, galley, ink);
                        if resp.clicked() {
                            pick = Some(Pick::Do(*action));
                        }
                    }
                });
            });
        }
    }
    if dim < 1. || lift.closing.is_some() || lift.shown.is_none_or(|at| (now - at).as_secs_f32() < OPEN_SECS) {
        ctx.request_repaint();
    }
    if !open {
        s.lift = Some(lift);
        return;
    }
    let pulled = scrim.inner.dragged() && ctx.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos())).is_some_and(|(from, to)| to.y - from.y > 40.);
    match pick {
        Some(Pick::Picker) => lift.picker = !lift.picker,
        Some(Pick::React(emoji)) => s.react(emoji),
        Some(Pick::Do(action)) => {
            apply(ctx, s, action);
            if matches!(action, Action::Reply | Action::Edit) {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text"))); // on a phone: the keyboard comes up
            }
        }
        None if scrim.inner.clicked() || pulled => s.close_menu(),
        None => {}
    }
    s.lift = Some(lift);
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Pos2 = pos2(100., 300.);

    #[test]
    fn long_press_fires_once_after_the_hold() {
        let mut lp = LongPress::default();
        assert_eq!(lp.step(0., Some((P, true)), Some(P), false), None);
        assert_eq!(lp.step(0.2, None, Some(P + vec2(3., -4.)), false), None, "too early");
        assert_eq!(lp.step(0.36, None, Some(P + vec2(3., -4.)), false), Some(P), "fires at the pressed point");
        assert_eq!(lp.step(0.36, None, Some(P), false), Some(P), "the same frame gives the same answer to a second timeline");
        assert_eq!(lp.step(0.5, None, Some(P), false), None, "once per press");
        assert_eq!(lp.step(2., None, Some(P), false), None);
    }

    #[test]
    fn long_press_gives_way_to_tap_scroll_and_back_swipe() {
        // A tap: released before the hold.
        let mut lp = LongPress::default();
        lp.step(0., Some((P, true)), Some(P), false);
        assert_eq!(lp.step(0.1, None, None, false), None);
        assert_eq!(lp.step(0.5, None, None, false), None);
        // The finger drags (scroll or swipe): past the slop the press is lost for good, even if it comes back.
        let mut lp = LongPress::default();
        lp.step(0., Some((P, true)), Some(P), false);
        assert_eq!(lp.step(0.1, None, Some(P + vec2(0., SLOP + 1.)), false), None);
        assert_eq!(lp.step(0.5, None, Some(P), false), None);
        // The view scrolled under a still finger (wheel, momentum).
        let mut lp = LongPress::default();
        lp.step(0., Some((P, true)), Some(P), false);
        assert_eq!(lp.step(0.1, None, Some(P), true), None);
        assert_eq!(lp.step(0.5, None, Some(P), false), None);
        // Pressed in the back-swipe strip: never a long press.
        let mut lp = LongPress::default();
        lp.step(0., Some((pos2(5., 300.), false)), Some(pos2(5., 300.)), false);
        assert_eq!(lp.step(0.5, None, Some(pos2(5., 300.)), false), None);
        // A new press starts a new count.
        assert_eq!(lp.step(1., Some((P, true)), Some(P), false), None);
        assert_eq!(lp.step(1.35, None, Some(P), false), Some(P));
    }

    fn area() -> Rect {
        Rect::from_min_max(pos2(16., 100.), pos2(386., 760.))
    }
    const BAR: Vec2 = vec2(316., 52.);
    const LIST: Vec2 = vec2(280., 248.);

    fn inside(outer: Rect, r: Rect) -> bool {
        outer.contains_rect(r)
    }

    #[test]
    fn menu_goes_around_a_message_in_the_middle() {
        let bubble = Rect::from_min_max(pos2(200., 300.), pos2(378., 340.)); // own message, on the right
        let (bar, list) = place(area(), bubble, BAR, LIST);
        assert_eq!((bar.bottom(), list.top()), (300. - GAP, 340. + GAP));
        assert_eq!((bar.right(), list.right()), (378., 378.), "aligned to the bubble's far edge");
        assert!(inside(area(), bar) && inside(area(), list));
        // Someone else's message: aligned to its near edge, pushed inside the margin when the bar is wider than the room left.
        let theirs = Rect::from_min_max(pos2(60., 300.), pos2(180., 340.));
        let (bar, list) = place(area(), theirs, BAR, LIST);
        assert_eq!((bar.left(), list.left()), (60., 60.));
        let (bar, _) = place(area(), theirs.translate(vec2(40., 0.)), BAR, LIST);
        assert_eq!(bar.right(), 386.);
    }

    #[test]
    fn menu_flips_at_the_ends_of_the_screen() {
        // Last message, just above the composer: bar next to it, list above the bar.
        let low = Rect::from_min_max(pos2(200., 710.), pos2(378., 750.));
        let (bar, list) = place(area(), low, BAR, LIST);
        assert_eq!(bar.bottom(), 710. - GAP);
        assert_eq!(list.bottom(), bar.top() - GAP);
        assert!(inside(area(), bar) && inside(area(), list));
        // First message under the header: bar below it, list below the bar.
        let high = Rect::from_min_max(pos2(60., 104.), pos2(180., 144.));
        let (bar, list) = place(area(), high, BAR, LIST);
        assert_eq!((bar.top(), list.top()), (144. + GAP, 144. + GAP + BAR.y + GAP));
        assert!(inside(area(), bar) && inside(area(), list));
    }

    #[test]
    fn menu_covers_the_visible_part_of_a_message_taller_than_the_screen() {
        let tall = Rect::from_min_max(pos2(60., -400.), pos2(340., 1500.));
        let (bar, list) = place(area(), tall, BAR, LIST);
        assert_eq!((bar.top(), list.bottom()), (100., 760.));
        assert!(!bar.intersects(list));
        // Its end is in view with room for the list only: the list goes under it, the bar stays at the top of the screen.
        let (bar, list) = place(area(), tall.translate(vec2(0., -1020.)), BAR, LIST);
        assert_eq!((bar.top(), list.top()), (100., 480. + GAP));
        // Room for both under its end.
        let (bar, list) = place(area(), tall.translate(vec2(0., -1100.)), BAR, LIST);
        assert_eq!((bar.top(), list.top()), (400. + GAP, 400. + GAP + BAR.y + GAP));
        // Scrolled out below the room (cannot happen with a press, must not panic or leave the room).
        let (bar, list) = place(area(), tall.translate(vec2(0., 1300.)), BAR, LIST);
        assert!(inside(area(), bar) && inside(area(), list));
    }

    #[test]
    fn menu_fits_a_narrow_or_short_room() {
        // 300 pt wide screen: nothing is wider than the room between the margins.
        let narrow = Rect::from_min_max(pos2(16., 100.), pos2(284., 760.));
        let (bar, list) = place(narrow, Rect::from_min_max(pos2(150., 300.), pos2(276., 340.)), BAR, LIST);
        assert_eq!((bar.width(), list.width()), (268., 268.));
        assert!(inside(narrow, bar) && inside(narrow, list));
        // Keyboard up: the list is cut to the room (it scrolls) and never overlaps the bar.
        let short = Rect::from_min_max(pos2(16., 100.), pos2(386., 300.));
        let (bar, list) = place(short, Rect::from_min_max(pos2(200., 180.), pos2(378., 220.)), BAR, LIST);
        assert!(inside(short, bar) && inside(short, list) && !bar.intersects(list), "{bar:?} {list:?}");
    }

    #[test]
    fn entries_follow_permissions_and_message_type() {
        use Action::*;
        let none = MessagePermissions::default();
        let all = MessagePermissions { reply: true, edit: true, redact: true, react: true };
        // Own text message.
        assert_eq!(actions(all, true, false, false), [Reply, Thread, Copy, Edit, Delete]);
        // Someone else's text: no edit, no delete; a moderator may delete it.
        let theirs = MessagePermissions { reply: true, react: true, ..none };
        assert_eq!(actions(theirs, true, false, false), [Reply, Thread, Copy]);
        assert_eq!(actions(MessagePermissions { redact: true, ..theirs }, true, false, false), [Reply, Thread, Copy, Delete]);
        // A file without a caption: nothing to copy.
        assert_eq!(actions(theirs, false, false, false), [Reply, Thread]);
        // A picture or a sticker: it can join the own stickers; with a caption the caption can be copied.
        assert_eq!(actions(theirs, false, true, false), [Reply, Thread, Sticker]);
        assert_eq!(actions(MessagePermissions { edit: false, ..all }, true, true, false), [Reply, Thread, Copy, Sticker, Delete]);
        // Own open poll.
        assert_eq!(actions(MessagePermissions { edit: false, ..all }, false, false, true), [Reply, Thread, EndPoll, Delete]);
        // Read-only room, deleted message.
        assert_eq!(actions(none, false, false, false), []);
        assert_eq!(actions(none, true, false, false), [Copy]);
    }

    #[test]
    fn animation_springs_in_fades_out_and_respects_reduce_motion() {
        let near = |(a, s): (f32, f32), (ea, es): (f32, f32)| (a - ea).abs() < 1e-4 && (s - es).abs() < 1e-4;
        assert!(near(anim(0., None, false), (0., 0.9)));
        let (alpha, scale) = anim(OPEN_SECS * 0.7, None, false);
        assert!(alpha > 0.9 && scale > 1., "overshoots before settling: {alpha} {scale}");
        assert!(near(anim(1., None, false), (1., 1.)));
        assert!(near(anim(1., Some(CLOSE_SECS / 2.), false), (0.5, 1.)));
        assert!(near(anim(1., Some(1.), false), (0., 1.)));
        // Reduce Motion: opacity only.
        assert_eq!(anim(0., None, true).1, 1.);
        assert_eq!(anim(OPEN_SECS / 2., None, true).1, 1.);
    }

    #[test]
    fn quick_reactions_have_translated_names() {
        // The names are dynamic keys the coverage scan cannot see.
        for (name, emoji) in QUICK {
            assert_ne!(matrix_desktop::i18n::tr("it", "", name), name, "{emoji} {name} lacks an Italian name");
        }
    }
}
