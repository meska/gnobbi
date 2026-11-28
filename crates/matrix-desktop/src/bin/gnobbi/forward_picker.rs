//! «Forward to…»: the chat picker of a forward. A card over the app in a wide window, the whole screen on a phone: search, the chats
//! (newest first), a tick on each chosen one, and one button that says how many. Only the look: the caller fills `Picker` (chats
//! already ordered and filtered by `query`), draws it with `show` and acts on the returned `Action`. Nothing here searches or sends.
use crate::avatar::Avatar;
use crate::icons::Icon;
use crate::list::{one_line, rgb};
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use matrix_desktop::{DesignTokens, i18n};

const PAD: f32 = 12.;
const AVATAR: f32 = 40.;
const CHECK: f32 = 22.;
/// The card of a wide window.
const CARD: Vec2 = Vec2::new(400., 560.);

pub struct Chat {
    /// Stable across searches: what `Picker::selected` holds.
    pub key: String,
    pub name: String,
    pub initials: String,
    pub color: u32,
    pub avatar: Avatar,
}

#[derive(Clone, PartialEq)]
pub enum Phase {
    /// The chats are not there yet.
    Loading,
    Pick,
    Sending,
    /// One line: what happened.
    Failed(String),
}

pub struct Picker {
    pub query: String,
    pub chats: Vec<Chat>,
    pub selected: Vec<String>,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Cancel,
    /// To every chat in `Picker::selected`; also the retry after a failure.
    Forward,
}

pub fn show(ctx: &egui::Context, s: &mut Picker, t: &DesignTokens) -> Option<Action> {
    let mut action = None;
    if crate::layout::compact(ctx) {
        let (all, safe) = (ctx.viewport_rect(), ctx.content_rect());
        let fill = crate::list_fill(t);
        egui::Area::new(egui::Id::unique("forward-picker")).order(egui::Order::Foreground).fixed_pos(all.min).show(ctx, |ui| {
            // The bands outside the safe area too: the area's own painter is clipped to it.
            egui::Painter::new(ctx.clone(), ui.layer_id(), all).rect_filled(all, 0., fill);
            ui.allocate_rect(all, Sense::click()); // nothing under the picker gets the taps
            let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(safe));
            ui.set_clip_rect(safe);
            action = body(&mut ui, safe, s, fill, t);
        });
    } else {
        let fill = rgb(t.surface);
        let size = CARD.min(ctx.content_rect().size() - Vec2::splat(48.));
        let frame = egui::Frame::popup(&ctx.style_of(ctx.theme())).fill(fill).stroke(egui::Stroke::NONE).corner_radius(12).inner_margin(0);
        let modal = egui::Modal::new(egui::Id::unique("forward-picker")).frame(frame).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.set_clip_rect(rect);
            action = body(ui, rect, s, fill, t);
        });
        if modal.should_close() {
            action = Some(Action::Cancel);
        }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(Action::Cancel);
    }
    action
}

/// Title and search at the top, the button at the bottom, the list takes what is left of `rect`.
fn body(ui: &mut egui::Ui, rect: Rect, s: &mut Picker, fill: Color32, t: &DesignTokens) -> Option<Action> {
    let mut action = None;
    let side = crate::metrics::button();
    let busy = s.phase == Phase::Sending;
    let (head, rest) = rect.split_top_bottom_at_y(rect.top() + 8. + side + 4. + side + 8.);
    let error = match &s.phase {
        Phase::Failed(line) => Some(one_line(ui, line, 14., rgb(t.error), (rect.width() - 2. * PAD - 26.).max(20.))),
        _ => None,
    };
    let (list, foot) = rest.split_top_bottom_at_y(rest.bottom() - 8. - side - 8. - error.as_ref().map_or(0., |g| g.size().y + 8.));
    let inner = |r: Rect| egui::UiBuilder::new().max_rect(r.shrink2(Vec2::new(PAD, 8.)));

    ui.scope_builder(inner(head), |ui| crate::dir::scope(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.;
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), side), crate::dir::row_rev(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(i18n::t("Cancel")).min_size(Vec2::new(side, side))).clicked() {
                action = Some(Action::Cancel);
            }
            ui.with_layout(crate::dir::row(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(i18n::t("Forward to…")).size(crate::metrics::get().title).strong().color(rgb(t.text))).truncate().selectable(false));
            });
        });
        ui.add_enabled_ui(!busy, |ui| crate::metrics::search(ui, [ui.available_width(), side], &mut s.query, i18n::t("Search")));
    }));

    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| crate::dir::scope(ui, |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        ui.add_enabled_ui(!busy, |ui| chats(ui, s, fill, t));
    }));

    ui.painter().hline(foot.x_range(), foot.top(), egui::Stroke::new(1., rgb(t.border)));
    let mut button = foot.shrink2(Vec2::new(PAD, 8.));
    if let Some(line) = error {
        let at = egui::pos2(button.left() + 26., button.top());
        crate::icons::paint(ui.painter(), crate::dir::flip_pos(button, egui::pos2(button.left() + 9., at.y + line.size().y / 2.)), Icon::WarningCircle, false, 18., rgb(t.error));
        ui.painter().galley(crate::dir::place(button, at, line.size().x), line.clone(), rgb(t.error));
        let spoken = Rect::from_min_size(button.min, Vec2::new(button.width(), line.size().y));
        ui.interact(spoken, egui::Id::unique("forward-error"), Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, line.text()));
        button.min.y += line.size().y + 8.;
    }
    if confirm(ui, button, s, t) {
        action = Some(Action::Forward);
    }
    action
}

/// The chats, the ghost rows while they load, or the one label of an empty list.
fn chats(ui: &mut egui::Ui, s: &mut Picker, fill: Color32, t: &DesignTokens) {
    let row_h = crate::rows::ROW.max(crate::metrics::button());
    if s.phase == Phase::Loading {
        let time = crate::loading::motion(ui);
        crate::loading::skeleton_list(ui, fill, row_h, 1., time, i18n::t("Loading…"), t);
        if time.is_some() {
            ui.ctx().request_repaint();
        }
        return;
    }
    if s.chats.is_empty() {
        let (icon, text) = if s.query.trim().is_empty() { (Icon::ChatCircleDots, i18n::t("No chats")) } else { (Icon::MagnifyingGlass, i18n::t("No results")) };
        let area = ui.available_rect_before_wrap();
        let c = area.center();
        crate::icons::paint(ui.painter(), c - Vec2::new(0., 18.), icon, false, 28., rgb(t.secondary));
        let g = one_line(ui, text, 14., rgb(t.secondary), (area.width() - 2. * PAD).max(20.));
        ui.painter().galley(egui::pos2(c.x - g.size().x / 2., c.y + 6.), g, rgb(t.secondary));
        ui.allocate_rect(area, Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.;
        for chat in &s.chats {
            let on = s.selected.contains(&chat.key);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
            let resp = crate::metrics::clickable(ui.interact(rect, egui::Id::unique(("forward-row", &chat.key)), Sense::click()));
            if on || resp.hovered() {
                ui.painter().rect_filled(rect, 0., rgb(if on { t.selected } else { t.input }));
            }
            if resp.has_focus() {
                ui.painter().rect_stroke(rect, 0., ui.visuals().selection.stroke, egui::StrokeKind::Inside);
            }
            // Worked out for LTR; `flip_pos`/`place` put each piece on its side.
            let c = crate::dir::flip_pos(rect, egui::pos2(rect.left() + PAD + AVATAR / 2., rect.center().y));
            crate::avatar::paint(ui, c, AVATAR, rgb(chat.color), &chat.avatar, &chat.initials, 15., rgb(t.avatar_text));
            let left = rect.left() + PAD + AVATAR + PAD;
            let name = one_line(ui, &chat.name, 15., rgb(t.text), (rect.right() - PAD - CHECK - PAD - left).max(20.));
            ui.painter().galley(crate::dir::place(rect, egui::pos2(left, rect.center().y - name.size().y / 2.), name.size().x), name, rgb(t.text));
            let k = crate::dir::flip_pos(rect, egui::pos2(rect.right() - PAD - CHECK / 2., rect.center().y));
            if on {
                ui.painter().circle_filled(k, CHECK / 2., rgb(t.accent));
                crate::icons::paint(ui.painter(), k, Icon::Check, false, 14., rgb(t.on_accent));
            } else {
                ui.painter().circle_stroke(k, CHECK / 2. - 1., egui::Stroke::new(1.5, rgb(t.secondary)));
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &chat.name));
            if resp.clicked() {
                match s.selected.iter().position(|k| *k == chat.key) {
                    Some(i) => drop(s.selected.remove(i)),
                    None => s.selected.push(chat.key.clone()),
                }
            }
        }
    });
}

/// What the button says: the number of chosen chats, so the ones scrolled or searched away still count on screen.
fn confirm_label(chosen: usize, failed: bool) -> String {
    match (failed, chosen) {
        (true, _) => i18n::t("Retry").to_owned(),
        (_, 0) => i18n::t("Forward").to_owned(),
        (_, n) => i18n::plural(n as u64, "Forward to {n} chat", "Forward to {n} chats"),
    }
}

/// The full-width button; a ring in its place while sending. True when pressed.
fn confirm(ui: &mut egui::Ui, rect: Rect, s: &Picker, t: &DesignTokens) -> bool {
    let id = egui::Id::unique("forward-confirm");
    if s.phase == Phase::Sending {
        let time = crate::loading::motion(ui);
        crate::loading::ring(ui, rect.center(), rect.height() / 2. - 10., 1., time, t);
        if time.is_some() {
            ui.ctx().request_repaint();
        }
        ui.interact(rect, id, Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, i18n::t("Sending…")));
        return false;
    }
    let ready = !s.selected.is_empty() && s.phase != Phase::Loading;
    let label = confirm_label(s.selected.len(), matches!(s.phase, Phase::Failed(_)));
    let resp = crate::metrics::clickable(ui.interact(rect, id, if ready { Sense::click() } else { Sense::hover() }));
    let (fill, ink) = if ready { (rgb(t.accent), rgb(t.on_accent)) } else { (rgb(t.disabled), rgb(t.secondary)) };
    let radius = crate::metrics::get().radius.max(8.);
    ui.painter().rect_filled(rect, radius, if ready && resp.hovered() { fill.gamma_multiply(0.9) } else { fill });
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.), radius + 2., ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
    let text = one_line(ui, &label, 15., ink, (rect.width() - 2. * PAD).max(20.));
    ui.painter().galley(rect.center() - text.size() / 2., text, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, &label));
    resp.clicked()
}

/// The error line after a forward: the chats that did not get the message, by name.
fn failure(keys: &[String], names: &[(String, String)]) -> String {
    let failed: Vec<&str> = keys.iter().filter_map(|k| names.iter().find(|(key, _)| key == k).map(|(_, name)| name.as_str())).collect();
    if failed.is_empty() { i18n::t("Failed").to_owned() } else { i18n::t("Not forwarded to {name}").replace("{name}", &failed.join(", ")) }
}

/// «Inoltra» in progress: the message and the picker over the chats of the account.
pub struct Open {
    scope: matrix_domain::RoomScope,
    event: matrix_domain::EventId,
    picker: Picker,
    finished: bool,
    /// Key and name of every chat that can be chosen, whatever the search says: the error names the ones that failed.
    names: Vec<(String, String)>,
}

impl Open {
    pub fn new(scope: matrix_domain::RoomScope, event: matrix_domain::EventId) -> Self {
        Self { scope, event, picker: Picker { query: String::new(), chats: Vec::new(), selected: Vec::new(), phase: Phase::Loading }, finished: false, names: Vec::new() }
    }

    /// The engine's answer, one result per chat: the ones that took it leave the choice, the others stay ticked for a retry.
    pub fn done(&mut self, results: &[(String, Result<(), matrix_engine::forward::ForwardError>)]) {
        self.picker.selected.retain(|key| results.iter().any(|(room, r)| room == key && r.is_err()));
        self.finished = self.picker.selected.is_empty();
        self.picker.phase = Phase::Failed(failure(&self.picker.selected, &self.names));
    }
}

/// Draws the picker when one is open, fills it from the account's chats (newest first, filtered by the search) and acts on it.
pub fn drive(ctx: &egui::Context, s: &mut crate::session::Session, t: &DesignTokens) {
    let Some(open) = &mut s.forward else { return };
    let query = open.picker.query.trim().to_lowercase();
    // The chat the message comes from stays in the list, like on Telegram; a chat where the user cannot write does not.
    let mut rooms: Vec<_> = s.sidebar.rooms().iter().filter(|r| !r.info.invited && !r.info.is_space && !r.info.read_only).collect();
    open.picker.phase = match (&open.picker.phase, rooms.is_empty()) {
        (Phase::Loading | Phase::Pick, true) => Phase::Loading,
        (Phase::Loading, false) => Phase::Pick,
        (phase, _) => phase.clone(),
    };
    open.names = rooms.iter().map(|r| (r.scope.room_id.as_str().to_owned(), r.name.clone())).collect();
    rooms.retain(|r| r.name.to_lowercase().contains(&query));
    rooms.sort_by_key(|r| std::cmp::Reverse(r.info.last_event_ts));
    open.picker.chats = rooms.iter().map(|r| Chat { key: r.scope.room_id.as_str().to_owned(), name: r.name.clone(), initials: r.initials.clone(), color: r.color, avatar: crate::avatar::lookup(&s.avatars, r.info.avatar.as_deref()) }).collect();
    if open.finished {
        s.forward = None;
        return;
    }
    match show(ctx, &mut open.picker, t) {
        Some(Action::Cancel) => s.forward = None,
        Some(Action::Forward) => {
            open.picker.phase = Phase::Sending;
            let (scope, event, rooms) = (open.scope.clone(), open.event.clone(), open.picker.selected.clone());
            s.forward_send(scope, event, rooms);
        }
        None => {}
    }
}

/// Debug builds, GNOBBI_EGUI_VIEW=forward[-selected|-long|-no-results|-empty|-loading|-sending|-error]: the picker alone with
/// made-up chats, for screenshots without an account.
#[cfg(debug_assertions)]
pub fn demo(ctx: &egui::Context, t: &DesignTokens) {
    thread_local!(static DEMO: std::cell::RefCell<Option<Picker>> = const { std::cell::RefCell::new(None) });
    let view = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
    let Some(state) = view.strip_prefix("forward") else { return };
    DEMO.with_borrow_mut(|d| {
        show(ctx, d.get_or_insert_with(|| fixture(state.trim_start_matches('-'))), t);
    });
}

#[cfg(any(debug_assertions, test))]
fn fixture(state: &str) -> Picker {
    let names: &[&str] = if state == "long" {
        &["Associazione Amici della Montagna di Bassano del Grappa – Direttivo", "Maria Antonietta Degli Esposti Castiglioni", "Condominio via Garibaldi 12 scala B"]
    } else {
        &["Anna Rossi", "Famiglia", "Luca Bianchi", "Condominio via Garibaldi", "Giulia", "Lavoro", "Paolo Verdi", "Calcetto del giovedì", "Mamma", "Vicini di casa", "Elena", "Coro parrocchiale"]
    };
    let colors = [0x53936b, 0x4a7fb5, 0xb5654a, 0x8a5fb0, 0xb09a3f];
    let chats = names.iter().enumerate().map(|(i, n)| Chat { key: i.to_string(), name: (*n).to_owned(), initials: n.chars().take(1).collect(), color: colors[i % colors.len()], avatar: Avatar::Missing }).collect();
    let mut p = Picker { query: String::new(), chats, selected: Vec::new(), phase: Phase::Pick };
    match state {
        "empty" | "loading" => p.chats.clear(),
        "no-results" => { p.chats.clear(); p.query = "zio".into(); }
        "selected" | "sending" | "error" => p.selected = vec!["0".into(), "3".into()],
        "long" => p.selected = vec!["1".into()],
        _ => {}
    }
    match state {
        "loading" => p.phase = Phase::Loading,
        "sending" => p.phase = Phase::Sending,
        "error" => p.phase = Phase::Failed(i18n::t("Failed").to_owned()),
        _ => {}
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the picker for a few frames, the last ones with `events`; returns the action and the rects of the button and the first row.
    fn run(ctx: &egui::Context, s: &mut Picker, screen: Vec2, events: Vec<Vec<egui::Event>>) -> (Option<Action>, Option<Rect>, Option<Rect>) {
        let mut action = None;
        for frame in std::iter::repeat_n(Vec::new(), 3).chain(events) {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, screen)), events: frame, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| action = action.or(show(ui.ctx(), s, &matrix_desktop::DARK)));
            out.textures_delta.clear();
        }
        let rect = |id: egui::Id| ctx.read_response(id).map(|r| r.rect);
        (action, rect(egui::Id::unique("forward-confirm")), rect(egui::Id::unique(("forward-row", &"0".to_owned()))))
    }

    fn tap(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]]
    }

    #[test]
    fn the_error_names_the_chats_that_failed() {
        let names = [("!a".to_owned(), "Anna".to_owned()), ("!b".to_owned(), "Lavoro".to_owned())];
        assert_eq!(failure(&["!b".into(), "!a".into()], &names), "Not forwarded to Lavoro, Anna");
        assert_eq!(failure(&["!gone".into()], &names), "Failed");
    }

    #[test]
    fn the_button_counts_the_chosen_chats() {
        assert_eq!(confirm_label(0, false), "Forward");
        assert_eq!(confirm_label(1, false), "Forward to 1 chat");
        assert_eq!(confirm_label(3, false), "Forward to 3 chats");
        assert_eq!(confirm_label(3, true), "Retry");
    }

    #[test]
    fn rows_and_button_fit_phone_and_desktop_and_forward_needs_a_chat() {
        crate::mobile::FORCE_TOUCH.set(true);
        // A phone at a large system text size is ~262 pt wide; 436 and 1280 are the two reference widths.
        for screen in [Vec2::new(262., 520.), Vec2::new(436., 860.), Vec2::new(1280., 800.), Vec2::new(1280., 360.)] {
            for state in ["", "long", "error"] {
                let ctx = crate::icons::test_context();
                crate::metrics::apply(&ctx, &matrix_desktop::Preferences::default(), 1.);
                let mut s = fixture(state);
                s.selected.clear();
                let (_, button, row) = run(&ctx, &mut s, screen, vec![]);
                let (button, row) = (button.unwrap(), row.unwrap());
                let safe = Rect::from_min_size(egui::Pos2::ZERO, screen);
                assert!(safe.contains_rect(button) && safe.contains_rect(row) && row.bottom() <= button.top(), "{screen:?} {state}: {button:?} {row:?}");
                assert!(button.height() >= crate::metrics::button() && row.height() >= crate::metrics::button(), "touch targets");
                // Nothing ticked: the button does nothing. A tap on a row ticks it, a second one unticks it.
                assert_eq!(run(&ctx, &mut s, screen, tap(button.center())).0, None);
                run(&ctx, &mut s, screen, tap(row.center()));
                assert_eq!(s.selected, ["0"], "{screen:?} {state}");
                assert_eq!(run(&ctx, &mut s, screen, tap(button.center())).0, Some(Action::Forward));
                run(&ctx, &mut s, screen, tap(row.center()));
                assert!(s.selected.is_empty());
            }
        }
        crate::mobile::FORCE_TOUCH.set(false);
    }
}
